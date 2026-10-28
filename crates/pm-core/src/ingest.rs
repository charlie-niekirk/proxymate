//! The single-writer ingest path. Request tasks, the synthetic generator and
//! `Engine::insert_flows` all send [`FlowEvent`]s down one channel; one
//! thread batches them and applies each batch under a single store write lock.
//!
//! Ids are allocated and Started events sent under one mutex, so the channel
//! carries flows in id order and the store can append without reordering.

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
}

/// The sending side, owned by the engine.
pub(crate) struct IngestTx {
    inner: Mutex<TxInner>,
}

struct TxInner {
    next_id: FlowId,
    tx: Sender<FlowEvent>,
}

impl IngestTx {
    pub fn new(tx: Sender<FlowEvent>) -> Self {
        IngestTx {
            inner: Mutex::new(TxInner { next_id: 0, tx }),
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

    /// Queues an event that needs no ordering with id allocation.
    pub fn send(&self, event: FlowEvent) {
        let _ = self.inner.lock().tx.send(event);
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
        let acks = engine.apply_events(batch);
        for ack in acks {
            let _ = ack.send(());
        }
        if disconnected {
            return;
        }
    }
}
