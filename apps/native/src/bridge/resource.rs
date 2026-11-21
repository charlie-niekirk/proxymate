//! A small keyed cache of server state: the native app's TanStack Query.
//! One `Resource<K, V>` entity per shape of data (its keys and value type
//! live in [`super::keys`]), so every reader of a key sees the same value.
//!
//! [`Resource::get`] answers from the cache and fetches what is missing or
//! stale on the background executor (the fetcher is a blocking engine
//! call); a stale value stays visible until its refetch replaces it, as
//! TanStack's do. Writes invalidate what they make stale
//! ([`super::mutations`]); push streams do too (the flow table's deltas).
//! Without a fetcher (tests, offscreen renders) values are [`Resource::set`]
//! directly.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, SharedString};

/// A blocking fetch of one key, run off the foreground thread.
pub type Fetcher<K, V> = Arc<dyn Fn(&K) -> Result<V, String> + Send + Sync>;

/// What a reader gets for a key.
#[derive(Clone, Debug, PartialEq)]
pub enum Load<V> {
    /// Nothing yet: the first fetch is running.
    Loading,
    Ready(V),
    /// The fetch failed; `Resource::retry` asks again.
    Failed(SharedString),
}

impl<V> Load<V> {
    pub fn ready(&self) -> Option<&V> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

struct Entry<V> {
    value: Option<Result<V, SharedString>>,
    stale: bool,
    fetching: bool,
    /// Bumped by every invalidation: a fetch that started before one lands
    /// as stale, so the next read fetches again.
    generation: u64,
    used: u64,
}

pub struct Resource<K, V> {
    entries: HashMap<K, Entry<V>>,
    fetch: Option<Fetcher<K, V>>,
    capacity: usize,
    clock: u64,
    fetches: u64,
}

/// Entries kept before the least recently read go.
pub const DEFAULT_CAPACITY: usize = 64;

impl<K, V> Resource<K, V>
where
    K: Clone + Eq + Hash + Send + 'static,
    V: Clone + Send + 'static,
{
    pub fn new(fetch: Option<Fetcher<K, V>>) -> Self {
        Resource {
            entries: HashMap::new(),
            fetch,
            capacity: DEFAULT_CAPACITY,
            clock: 0,
            fetches: 0,
        }
    }

    /// The cached value (fresh or stale), fetching it if it is missing or
    /// stale.
    pub fn get(&mut self, key: &K, cx: &mut Context<Self>) -> Load<V> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.entry(key.clone()).or_insert_with(|| Entry {
            value: None,
            stale: true,
            fetching: false,
            generation: 0,
            used: clock,
        });
        entry.used = clock;
        let load = match &entry.value {
            None => Load::Loading,
            Some(Ok(v)) => Load::Ready(v.clone()),
            Some(Err(e)) => Load::Failed(e.clone()),
        };
        // A failure waits for `retry`; anything else missing or stale refetches.
        let failed = matches!(entry.value, Some(Err(_)));
        if entry.stale && !entry.fetching && !failed {
            self.start_fetch(key.clone(), cx);
        }
        self.evict();
        load
    }

    /// The cached value, if any, without fetching.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key)?.value.as_ref()?.as_ref().ok()
    }

    /// Sets a value (tests, offscreen renders, a mutation's answer).
    pub fn set(&mut self, key: K, value: V, cx: &mut Context<Self>) {
        self.clock += 1;
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            value: None,
            stale: false,
            fetching: false,
            generation: 0,
            used: 0,
        });
        entry.value = Some(Ok(value));
        entry.stale = false;
        entry.used = self.clock;
        self.evict();
        cx.notify();
    }

    /// Marks the keys `stale` picks as stale; readers refetch them on their
    /// next read. True if any was cached.
    pub fn invalidate(&mut self, stale: impl Fn(&K, Option<&V>) -> bool, cx: &mut Context<Self>) -> bool {
        let mut any = false;
        for (key, entry) in &mut self.entries {
            let value = entry.value.as_ref().and_then(|v| v.as_ref().ok());
            if stale(key, value) {
                entry.stale = true;
                entry.generation += 1;
                // A failure is retried by an invalidation too.
                if matches!(entry.value, Some(Err(_))) {
                    entry.value = None;
                }
                any = true;
            }
        }
        if any {
            cx.notify();
        }
        any
    }

    /// Asks for a failed (or any) key again ("Try again").
    pub fn retry(&mut self, key: &K, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries.get_mut(key) {
            if matches!(entry.value, Some(Err(_))) {
                entry.value = None;
            }
            entry.stale = true;
            entry.generation += 1;
        }
        cx.notify();
    }

    /// How many fetches have started (tests).
    pub fn fetches(&self) -> u64 {
        self.fetches
    }

    fn start_fetch(&mut self, key: K, cx: &mut Context<Self>) {
        let Some(fetch) = self.fetch.clone() else { return };
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        entry.fetching = true;
        let generation = entry.generation;
        self.fetches += 1;
        let task = cx.background_spawn({
            let key = key.clone();
            async move { fetch(&key) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(entry) = this.entries.get_mut(&key) else {
                    return;
                };
                entry.fetching = false;
                entry.value = Some(result.map_err(SharedString::from));
                entry.stale = entry.generation != generation;
                cx.notify();
            });
        })
        .detach();
    }

    fn evict(&mut self) {
        while self.entries.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, e)| !e.fetching)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => {
                    self.entries.remove(&key);
                }
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn fetches_once_keeps_stale_values_and_refetches_after_invalidation(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicU32::new(0));
        let counter = calls.clone();
        let fetch: Fetcher<u32, String> = Arc::new(move |k| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{k}#{n}"))
        });
        let res = cx.new(|_| Resource::new(Some(fetch)));
        assert_eq!(res.update(cx, |r, cx| r.get(&7, cx)), Load::Loading);
        assert_eq!(
            res.update(cx, |r, cx| r.get(&7, cx)),
            Load::Loading,
            "one fetch in flight"
        );
        cx.run_until_parked();
        assert_eq!(res.update(cx, |r, cx| r.get(&7, cx)), Load::Ready("7#0".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        res.update(cx, |r, cx| r.invalidate(|k, _| *k == 7, cx));
        // The stale value shows while the refetch runs.
        assert_eq!(res.update(cx, |r, cx| r.get(&7, cx)), Load::Ready("7#0".into()));
        cx.run_until_parked();
        assert_eq!(res.update(cx, |r, cx| r.get(&7, cx)), Load::Ready("7#1".into()));
    }

    #[gpui_kit::test]
    fn failures_wait_for_retry_and_values_can_be_set(cx: &mut TestAppContext) {
        let fail = Arc::new(AtomicU32::new(1));
        let flag = fail.clone();
        let fetch: Fetcher<u32, u32> = Arc::new(move |k| {
            if flag.load(Ordering::SeqCst) == 1 {
                Err("gone".into())
            } else {
                Ok(k * 2)
            }
        });
        let res = cx.new(|_| Resource::new(Some(fetch)));
        res.update(cx, |r, cx| r.get(&2, cx));
        cx.run_until_parked();
        assert_eq!(res.update(cx, |r, cx| r.get(&2, cx)), Load::Failed("gone".into()));
        cx.run_until_parked();
        assert_eq!(res.read_with(cx, |r, _| r.fetches()), 1, "no refetch loop");
        fail.store(0, Ordering::SeqCst);
        res.update(cx, |r, cx| r.retry(&2, cx));
        assert_eq!(res.update(cx, |r, cx| r.get(&2, cx)), Load::Loading);
        cx.run_until_parked();
        assert_eq!(res.update(cx, |r, cx| r.get(&2, cx)), Load::Ready(4));
        let offline = cx.new(|_| Resource::<u32, u32>::new(None));
        offline.update(cx, |r, cx| r.set(1, 10, cx));
        assert_eq!(offline.update(cx, |r, cx| r.get(&1, cx)), Load::Ready(10));
        assert_eq!(offline.update(cx, |r, cx| r.get(&2, cx)), Load::Loading);
    }

    #[gpui_kit::test]
    fn the_least_recently_read_go_first(cx: &mut TestAppContext) {
        let res = cx.new(|_| {
            let mut r = Resource::<u32, u32>::new(None);
            r.capacity = 2;
            r
        });
        res.update(cx, |r, cx| {
            r.set(1, 1, cx);
            r.set(2, 2, cx);
            r.get(&1, cx);
            r.set(3, 3, cx);
        });
        res.read_with(cx, |r, _| {
            assert_eq!((r.peek(&1), r.peek(&2), r.peek(&3)), (Some(&1), None, Some(&3)));
        });
    }
}
