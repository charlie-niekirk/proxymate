//! A [`Store`] behind its lock, with a change generation (R2). Every write
//! guard bumps the generation as it is dropped, while it still holds the
//! lock, so the publisher can tell a clean tick from an atomic load and
//! skip the write lock it would otherwise take to drain deltas.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::Store;

/// Who looks at the generation, each with its own idea of the last one it
/// saw.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Watcher {
    /// The publisher's pass over the view deltas.
    Views,
    /// The sidebar snapshot.
    Sidebar,
}

pub struct SharedStore {
    lock: RwLock<Store>,
    generation: AtomicU64,
    seen: [AtomicU64; 2],
}

impl SharedStore {
    pub fn new(store: Store) -> Self {
        SharedStore {
            lock: RwLock::new(store),
            // Starts ahead of what any watcher saw: the first tick looks.
            generation: AtomicU64::new(1),
            seen: [AtomicU64::new(0), AtomicU64::new(0)],
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Store> {
        self.lock.read()
    }

    /// Write access; anything done through the guard counts as a change.
    pub fn write(&self) -> StoreWriteGuard<'_> {
        StoreWriteGuard {
            guard: self.lock.write(),
            generation: &self.generation,
        }
    }

    /// Write access that doesn't count as a change: the publisher draining
    /// deltas it is about to send, or a deep search taking candidates.
    pub(crate) fn write_quiet(&self) -> RwLockWriteGuard<'_, Store> {
        self.lock.write()
    }

    /// True if the store may have changed since `watcher` last asked; the
    /// answer is consumed. Call it before locking: a change made after
    /// the load is caught on the next call.
    pub(crate) fn changed(&self, watcher: Watcher) -> bool {
        let now = self.generation.load(Ordering::Acquire);
        self.seen[watcher as usize].swap(now, Ordering::AcqRel) != now
    }
}

pub struct StoreWriteGuard<'a> {
    guard: RwLockWriteGuard<'a, Store>,
    generation: &'a AtomicU64,
}

impl Deref for StoreWriteGuard<'_> {
    type Target = Store;

    fn deref(&self) -> &Store {
        &self.guard
    }
}

impl DerefMut for StoreWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.guard
    }
}

impl Drop for StoreWriteGuard<'_> {
    fn drop(&mut self) {
        // Still under the lock: a watcher that loads the new generation
        // can only lock after this guard's changes are in.
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_are_seen_once_per_watcher_and_quiet_writes_not_at_all() {
        let store = SharedStore::new(Store::default());
        assert!(store.changed(Watcher::Views));
        assert!(!store.changed(Watcher::Views));
        drop(store.read());
        drop(store.write_quiet());
        assert!(!store.changed(Watcher::Views));
        drop(store.write());
        assert!(store.changed(Watcher::Views));
        assert!(!store.changed(Watcher::Views));
        // The sidebar keeps its own place.
        assert!(store.changed(Watcher::Sidebar));
        assert!(!store.changed(Watcher::Sidebar));
    }
}
