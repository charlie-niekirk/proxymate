//! The single-writer ingest path. Request tasks, the synthetic generator and
//! `Engine::insert_flows` all send [`FlowEvent`]s down one channel; one
//! thread batches them and applies each batch under a single store write lock.
//!
//! Ids are allocated and Started events sent under one mutex, so the channel
//! carries flows in id order and the store can append without reordering.
//!
//! Size updates (Progress, and WebSocket message sizes) are the one stream
//! that grows with the traffic rather than with the flows, so they don't
//! queue (R2): each flow keeps one pending update, folded into the next,
//! and the channel carries a single marker for all of them.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{ContentKind, FlowId, FlowLabel, FlowState, FlowSummary, WsDirection};

use crate::Engine;

/// A batch closes this long after its first event...
const BATCH_WINDOW: Duration = Duration::from_millis(5);
/// ...or once it holds this many events.
const BATCH_MAX: usize = 50_000;

/// One flow's header column values: `(column, value)` pairs (M17).
pub(crate) type ExtraCells = (FlowId, Vec<(u8, Option<Arc<str>>)>);

/// A change to the summary arena. Body bytes never travel here.
pub(crate) enum FlowEvent {
    Started(FlowSummary),
    Response {
        id: FlowId,
        status: u16,
        content_kind: ContentKind,
    },
    /// Body detection finished (or improved on the header's guess).
    Content {
        id: FlowId,
        content_kind: ContentKind,
        graphql_op: Option<Arc<str>>,
    },
    Progress {
        id: FlowId,
        req_size: u64,
        resp_size: u64,
    },
    /// One WebSocket message; adds its payload size to the flow's totals.
    WsMsg {
        id: FlowId,
        direction: WsDirection,
        len: u64,
    },
    Finished {
        id: FlowId,
        state: FlowState,
        duration_ms: u32,
        req_size: u64,
        resp_size: u64,
    },
    /// A gRPC call's status, from its trailers.
    Grpc {
        id: FlowId,
        status: u8,
    },
    /// A rule changed the flow (the Edited column).
    Edited {
        id: FlowId,
    },
    /// The flow paused at a breakpoint, or went on.
    State {
        id: FlowId,
        state: FlowState,
    },
    /// The client app behind the flow's connection became known.
    ClientResolved {
        id: FlowId,
        client: Arc<str>,
    },
    /// The user pinned (or unpinned) flows: pinned flows outlive eviction.
    Pin {
        ids: Vec<FlowId>,
        pinned: bool,
    },
    /// The user labelled flows (M17); `None` takes the label off.
    Label {
        ids: Vec<FlowId>,
        label: Option<FlowLabel>,
    },
    /// The user's note on a flow (M17); `None` removes it.
    Comment {
        id: FlowId,
        comment: Option<Arc<str>>,
    },
    /// Header column values (M17): each flow's `(column, value)` pairs.
    /// Ignored unless `generation` is the store's current column set.
    Extra {
        generation: u32,
        values: Vec<ExtraCells>,
    },
    /// The header columns changed (M17): every flow's values are dropped
    /// until they are filled again for `generation`.
    Columns {
        generation: u32,
    },
    /// Bulk append with ids already assigned (synthetic data, benches).
    Insert(Vec<FlowSummary>),
    /// Bulk replace by id.
    Update(Vec<FlowSummary>),
    /// Drops every flow. Later events for older ids are ignored.
    Clear,
    /// Acknowledged once every earlier event has been applied.
    Flush(SyncSender<()>),
    /// The coalesced size updates go here: the ingest thread swaps in what
    /// [`IngestTx::take_sizes`] holds by now.
    Sizes,
}

/// The sending side, owned by the engine.
pub(crate) struct IngestTx {
    inner: Mutex<TxInner>,
}

struct TxInner {
    next_id: FlowId,
    tx: Sender<FlowEvent>,
    /// Size updates not applied yet; a [`FlowEvent::Sizes`] is queued
    /// whenever this goes from empty to not.
    sizes: HashMap<FlowId, PendingSizes>,
}

#[derive(Default)]
struct PendingSizes {
    /// The latest Progress: totals so far.
    progress: Option<(u64, u64)>,
    /// WebSocket payload bytes since the last update, each way.
    ws_sent: u64,
    ws_received: u64,
}

impl IngestTx {
    pub fn new(tx: Sender<FlowEvent>) -> Self {
        IngestTx {
            inner: Mutex::new(TxInner {
                next_id: 0,
                tx,
                sizes: HashMap::new(),
            }),
        }
    }

    /// Assigns the next id to `summary` and queues its Started event.
    pub fn start(&self, mut summary: FlowSummary) -> FlowId {
        let mut inner = self.inner.lock();
        let id = inner.next_id;
        inner.next_id += 1;
        summary.id = id;
        let _ = inner.tx.send(FlowEvent::Started(summary));
        id
    }

    /// Assigns consecutive ids to `batch` and queues it as one event.
    pub fn insert(&self, mut batch: Vec<FlowSummary>) -> std::ops::Range<FlowId> {
        let mut inner = self.inner.lock();
        let first = inner.next_id;
        for flow in &mut batch {
            flow.id = inner.next_id;
            inner.next_id += 1;
        }
        let range = first..inner.next_id;
        let _ = inner.tx.send(FlowEvent::Insert(batch));
        range
    }

    /// Queues a Clear. Returns the first id that survives it.
    pub fn clear(&self) -> FlowId {
        let inner = self.inner.lock();
        let _ = inner.tx.send(FlowEvent::Clear);
        inner.next_id
    }

    /// Queues an event that needs no ordering with id allocation. Size
    /// updates are folded into the flow's pending one.
    pub fn send(&self, event: FlowEvent) {
        let mut inner = self.inner.lock();
        let id = match &event {
            FlowEvent::Progress { id, .. } | FlowEvent::WsMsg { id, .. } => *id,
            _ => {
                let _ = inner.tx.send(event);
                return;
            }
        };
        let first = inner.sizes.is_empty();
        let pending = inner.sizes.entry(id).or_default();
        match event {
            FlowEvent::Progress {
                req_size, resp_size, ..
            } => pending.progress = Some((req_size, resp_size)),
            FlowEvent::WsMsg { direction, len, .. } => match direction {
                WsDirection::Sent => pending.ws_sent += len,
                WsDirection::Received => pending.ws_received += len,
            },
            _ => unreachable!("only size updates get here"),
        }
        if first {
            let _ = inner.tx.send(FlowEvent::Sizes);
        }
    }

    /// The pending size updates, as events, oldest flow first.
    pub(crate) fn take_sizes(&self) -> Vec<FlowEvent> {
        let mut sizes: Vec<(FlowId, PendingSizes)> = self.inner.lock().sizes.drain().collect();
        sizes.sort_unstable_by_key(|(id, _)| *id);
        let mut events = Vec::with_capacity(sizes.len());
        for (id, pending) in sizes {
            if let Some((req_size, resp_size)) = pending.progress {
                events.push(FlowEvent::Progress {
                    id,
                    req_size,
                    resp_size,
                });
            }
            for (direction, len) in [
                (WsDirection::Sent, pending.ws_sent),
                (WsDirection::Received, pending.ws_received),
            ] {
                if len > 0 {
                    events.push(FlowEvent::WsMsg { id, direction, len });
                }
            }
        }
        events
    }

    /// Blocks until every event queued so far has been applied.
    pub fn flush(&self) {
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
        self.send(FlowEvent::Flush(ack_tx));
        let _ = ack_rx.recv();
    }
}

/// Runs on the `pm-ingest` thread until every sender is gone or the engine
/// is dropped.
pub(crate) fn ingest_loop(rx: Receiver<FlowEvent>, engine: Weak<Engine>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        let deadline = Instant::now() + BATCH_WINDOW;
        let mut disconnected = false;
        while batch.len() < BATCH_MAX && !matches!(batch.last(), Some(FlowEvent::Flush(_))) {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(event) => batch.push(event),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        let Some(engine) = engine.upgrade() else {
            return;
        };
        if batch.iter().any(|e| matches!(e, FlowEvent::Sizes)) {
            let mut expanded = Vec::with_capacity(batch.len());
            for event in batch {
                match event {
                    FlowEvent::Sizes => expanded.extend(engine.ingest().take_sizes()),
                    event => expanded.push(event),
                }
            }
            batch = expanded;
        }
        let acks = engine.apply_events(batch);
        for ack in acks {
            let _ = ack.send(());
        }
        if disconnected {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_updates_fold_into_one_pending_update_per_flow() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ingest = IngestTx::new(tx);
        for size in 1..=100u64 {
            ingest.send(FlowEvent::Progress {
                id: 3,
                req_size: size,
                resp_size: size * 2,
            });
            ingest.send(FlowEvent::WsMsg {
                id: 1,
                direction: WsDirection::Received,
                len: 10,
            });
        }
        ingest.send(FlowEvent::Edited { id: 3 });
        // One marker for all 200 updates, then the other event.
        let queued: Vec<FlowEvent> = rx.try_iter().collect();
        assert!(matches!(queued[..], [FlowEvent::Sizes, FlowEvent::Edited { id: 3 }]));
        let sizes = ingest.take_sizes();
        assert!(matches!(
            sizes[..],
            [
                FlowEvent::WsMsg {
                    id: 1,
                    direction: WsDirection::Received,
                    len: 1000
                },
                FlowEvent::Progress {
                    id: 3,
                    req_size: 100,
                    resp_size: 200
                },
            ]
        ));
        // Taken: the next update queues a new marker.
        assert!(ingest.take_sizes().is_empty());
        ingest.send(FlowEvent::Progress {
            id: 3,
            req_size: 101,
            resp_size: 202,
        });
        assert!(matches!(rx.try_iter().collect::<Vec<_>>()[..], [FlowEvent::Sizes]));
    }
}
