//! Where a store's flow details, bodies and message logs live. The live
//! capture keeps them in segment files ([`SegmentBackend`]); saved
//! sessions and imported HARs keep them in one SQLite file
//! ([`SqliteBackend`]). The summary arena and views ([`super::Store`]) are
//! the same for both, and the engine reads either through this trait.
//!
//! [`SqliteBackend`]: super::sqlite::SqliteBackend

use std::io;
use std::sync::Arc;

use pm_model::{BodyPart, FlowDetail, FlowId};

use super::detail::DetailStore;
use super::messages::{SseLog, WsLog};
use super::segments::SegmentStore;

/// The live capture's backend: segment files with an LRU of details, and
/// in-flight flows read from their recorders.
pub(crate) type SegmentBackend = DetailStore;

pub(crate) trait BlobBackend: Send + Sync {
    /// Heads and body metadata of a flow.
    fn detail(&self, id: FlowId) -> io::Result<Option<FlowDetail>>;

    /// A body's captured bytes, still content-encoded (as its
    /// `BodyInfo::encoding` says).
    fn body(&self, id: FlowId, part: BodyPart) -> io::Result<Option<Vec<u8>>>;

    /// The flow's WebSocket messages, if it became a WebSocket.
    fn ws(&self, id: FlowId) -> io::Result<Option<Arc<WsLog>>>;

    /// The flow's server-sent events, if they were parsed as they arrived.
    fn sse(&self, id: FlowId) -> io::Result<Option<Arc<SseLog>>>;

    /// Segment files the message logs may point into.
    fn segments(&self) -> Option<&SegmentStore>;
}

impl BlobBackend for SegmentBackend {
    fn detail(&self, id: FlowId) -> io::Result<Option<FlowDetail>> {
        Ok(self.get(id)?.map(|stored| stored.detail.clone()))
    }

    fn body(&self, id: FlowId, part: BodyPart) -> io::Result<Option<Vec<u8>>> {
        DetailStore::body(self, id, part)
    }

    fn ws(&self, id: FlowId) -> io::Result<Option<Arc<WsLog>>> {
        Ok(DetailStore::ws(self, id))
    }

    fn sse(&self, id: FlowId) -> io::Result<Option<Arc<SseLog>>> {
        Ok(DetailStore::sse(self, id))
    }

    fn segments(&self) -> Option<&SegmentStore> {
        Some(&self.segments)
    }
}
