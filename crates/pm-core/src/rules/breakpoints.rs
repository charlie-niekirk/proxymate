//! Paused flows. A flow that hits a breakpoint parks a [`Pending`] entry
//! with a oneshot sender and waits on the receiver. The UI and agents
//! resolve it by id; the first resolution wins, later ones get
//! [`RuleError::AlreadyResolved`]. Without an answer the flow continues
//! unmodified after the timeout from the rule settings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use pm_model::{BreakpointId, BreakpointInfo, BreakpointResolution};
use tokio::sync::oneshot;

use super::RuleError;
use crate::notify::Notifier;

/// Receives the breakpoint queue whenever it changes. Returning `false`
/// unsubscribes.
pub type BreakpointSink = crate::notify::Sink<Vec<BreakpointInfo>>;

struct Pending {
    info: BreakpointInfo,
    body: Arc<Vec<u8>>,
    tx: oneshot::Sender<BreakpointResolution>,
}

pub(crate) enum Outcome {
    Resolved(BreakpointResolution),
    TimedOut,
}

pub(crate) struct BreakpointManager {
    pending: DashMap<BreakpointId, Pending>,
    next: AtomicU32,
    /// 0 waits forever.
    timeout_secs: AtomicU32,
    /// The UI and every MCP session watching `proxymate://breakpoints`.
    subscribers: Notifier<Vec<BreakpointInfo>>,
}

impl BreakpointManager {
    pub fn new(timeout_secs: u32) -> BreakpointManager {
        BreakpointManager {
            pending: DashMap::new(),
            next: AtomicU32::new(1),
            timeout_secs: AtomicU32::new(timeout_secs),
            subscribers: Notifier::new(),
        }
    }

    pub fn set_timeout_secs(&self, secs: u32) {
        self.timeout_secs.store(secs, Ordering::Relaxed);
    }

    /// Parks a flow until it is resolved or times out. `info.id` and the
    /// deadline are filled in here. Dropping the future (the client went
    /// away) removes the entry.
    pub async fn pause(&self, mut info: BreakpointInfo, body: Vec<u8>) -> Outcome {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let secs = self.timeout_secs.load(Ordering::Relaxed);
        let timeout = (secs > 0).then(|| Duration::from_secs(u64::from(secs)));
        info.id = id;
        info.deadline_ms = timeout.map(|t| info.paused_at_ms + t.as_secs_f64() * 1000.0);
        let (tx, mut rx) = oneshot::channel();
        self.pending.insert(
            id,
            Pending {
                info,
                body: Arc::new(body),
                tx,
            },
        );
        self.changed();
        let guard = Unpark { manager: self, id };
        let result = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, &mut rx).await.ok(),
            None => Some((&mut rx).await),
        };
        std::mem::forget(guard);
        match result {
            Some(Ok(resolution)) => Outcome::Resolved(resolution),
            Some(Err(_)) => Outcome::TimedOut,
            None => {
                if self.pending.remove(&id).is_some() {
                    self.changed();
                    return Outcome::TimedOut;
                }
                // A resolver won the race with the timeout.
                match rx.await {
                    Ok(resolution) => Outcome::Resolved(resolution),
                    Err(_) => Outcome::TimedOut,
                }
            }
        }
    }

    pub fn resolve(&self, id: BreakpointId, resolution: BreakpointResolution) -> Result<(), RuleError> {
        match self.pending.remove(&id) {
            Some((_, pending)) => {
                self.changed();
                let _ = pending.tx.send(resolution);
                Ok(())
            }
            None if id > 0 && id < self.next.load(Ordering::Relaxed) => Err(RuleError::AlreadyResolved(id)),
            None => Err(RuleError::UnknownBreakpoint(id)),
        }
    }

    /// The queue, oldest first.
    pub fn list(&self) -> Vec<BreakpointInfo> {
        let mut list: Vec<BreakpointInfo> = self.pending.iter().map(|p| p.info.clone()).collect();
        list.sort_by_key(|b| b.id);
        list
    }

    pub fn get(&self, id: BreakpointId) -> Option<BreakpointInfo> {
        self.pending.get(&id).map(|p| p.info.clone())
    }

    pub fn body(&self, id: BreakpointId) -> Option<Arc<Vec<u8>>> {
        self.pending.get(&id).map(|p| p.body.clone())
    }

    /// Adds a subscriber; it gets the queue on the next tick and whenever
    /// it changes.
    pub fn subscribe(&self, sink: BreakpointSink) {
        self.subscribers.subscribe(sink);
    }

    /// Part of the publisher tick: pushes the queue if it changed.
    pub fn publish(&self) {
        self.subscribers.publish(|| self.list());
    }

    fn changed(&self) {
        self.subscribers.changed();
    }
}

/// Removes a pending entry if the paused future is dropped.
struct Unpark<'a> {
    manager: &'a BreakpointManager,
    id: BreakpointId,
}

impl Drop for Unpark<'_> {
    fn drop(&mut self) {
        if self.manager.pending.remove(&self.id).is_some() {
            self.manager.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{BreakpointBody, HttpVersion, Phase, RequestHead};

    fn info() -> BreakpointInfo {
        BreakpointInfo {
            id: 0,
            flow_id: None,
            phase: Phase::Request,
            rule_id: "r".into(),
            rule_name: "r".into(),
            paused_at_ms: 0.0,
            deadline_ms: None,
            request: RequestHead {
                method: "GET".into(),
                url: "http://x/".into(),
                version: HttpVersion::Http11,
                headers: Vec::new(),
            },
            response: None,
            body: BreakpointBody::default(),
        }
    }

    #[tokio::test]
    async fn first_resolver_wins() {
        let manager = Arc::new(BreakpointManager::new(0));
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.pause(info(), Vec::new()).await })
        };
        let id = loop {
            if let Some(b) = manager.list().first() {
                break b.id;
            }
            tokio::task::yield_now().await;
        };
        manager.resolve(id, BreakpointResolution::Abort).unwrap();
        assert!(matches!(
            manager.resolve(id, BreakpointResolution::Abort),
            Err(RuleError::AlreadyResolved(_))
        ));
        assert!(matches!(
            manager.resolve(999, BreakpointResolution::Abort),
            Err(RuleError::UnknownBreakpoint(999))
        ));
        assert!(matches!(
            waiter.await.unwrap(),
            Outcome::Resolved(BreakpointResolution::Abort)
        ));
        assert!(manager.list().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn unanswered_breakpoints_time_out() {
        let manager = BreakpointManager::new(300);
        let outcome = manager.pause(info(), Vec::new()).await;
        assert!(matches!(outcome, Outcome::TimedOut));
        assert!(manager.list().is_empty());
    }

    #[tokio::test]
    async fn a_dropped_waiter_leaves_the_queue() {
        let manager = BreakpointManager::new(0);
        let pause = manager.pause(info(), Vec::new());
        let _ = tokio::time::timeout(Duration::from_millis(20), pause).await;
        assert!(manager.list().is_empty());
    }
}
