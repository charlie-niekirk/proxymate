//! Change notification for state the UI and MCP mirror (the breakpoint
//! queue, the rule list, the MCP status). A change bumps a version; the
//! publisher tick pushes a fresh snapshot to every subscriber that hasn't
//! seen that version, so bursts of changes coalesce into one push per tick.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

/// Receives snapshots. Returning `false` unsubscribes.
pub type Sink<T> = Box<dyn FnMut(T) -> bool + Send>;

struct Subscriber<T> {
    /// The version this subscriber last got; `None` until its first push.
    seen: Option<u64>,
    sink: Sink<T>,
}

pub(crate) struct Notifier<T> {
    version: AtomicU64,
    subscribers: Mutex<Vec<Subscriber<T>>>,
}

impl<T: Clone> Notifier<T> {
    pub fn new() -> Notifier<T> {
        Notifier {
            version: AtomicU64::new(0),
            subscribers: Mutex::new(Vec::new()),
        }
    }

    pub fn changed(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Adds a subscriber. It gets the current state on the next tick.
    pub fn subscribe(&self, sink: Sink<T>) {
        self.subscribers.lock().push(Subscriber { seen: None, sink });
    }

    /// Part of the publisher tick: pushes `snapshot()` to every subscriber
    /// that is behind. The snapshot is only taken if someone needs it.
    pub fn publish(&self, snapshot: impl FnOnce() -> T) {
        let mut subscribers = self.subscribers.lock();
        let version = self.version.load(Ordering::Acquire);
        if subscribers.iter().all(|s| s.seen == Some(version)) {
            return;
        }
        let value = snapshot();
        subscribers.retain_mut(|s| {
            if s.seen == Some(version) {
                return true;
            }
            s.seen = Some(version);
            (s.sink)(value.clone())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn new_subscribers_get_one_push_and_changes_reach_everyone() {
        let notifier = Notifier::<u32>::new();
        let first = Arc::new(AtomicU32::new(0));
        let counter = first.clone();
        notifier.subscribe(Box::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        }));
        notifier.publish(|| 1);
        notifier.publish(|| 1);
        assert_eq!(first.load(Ordering::Relaxed), 1);

        let second = Arc::new(AtomicU32::new(0));
        let counter = second.clone();
        notifier.subscribe(Box::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        }));
        notifier.publish(|| 1);
        assert_eq!((first.load(Ordering::Relaxed), second.load(Ordering::Relaxed)), (1, 1));

        notifier.changed();
        notifier.publish(|| 2);
        assert_eq!((first.load(Ordering::Relaxed), second.load(Ordering::Relaxed)), (2, 2));
    }

    #[test]
    fn closed_sinks_are_dropped() {
        let notifier = Notifier::<u32>::new();
        notifier.subscribe(Box::new(|_| false));
        notifier.publish(|| 0);
        assert!(notifier.subscribers.lock().is_empty());
    }
}
