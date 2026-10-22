//! Flow details: heads, body metadata and where each body lives. A flow in
//! flight is read from its live recorder; a finished one is postcard-encoded
//! into the segment files, with an LRU of decoded details in front.
//! WebSocket and SSE message logs are kept beside them, by flow id.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use moka::sync::Cache;
use parking_lot::Mutex;
use pm_model::{BodyPart, FlowDetail, FlowId};
use serde::{Deserialize, Serialize};

use super::blob::BlobRef;
use super::messages::{SseLog, WsLog};
use super::segments::{Extent, SegmentStore};
use crate::recorder::LiveFlow;

const CACHE_ENTRIES: u64 = 2_048;

/// What is persisted per finished flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredDetail {
    pub detail: FlowDetail,
    pub request_body: BlobRef,
    pub response_body: BlobRef,
}

impl StoredDetail {
    pub fn body(&self, part: BodyPart) -> &BlobRef {
        match part {
            BodyPart::Request => &self.request_body,
            BodyPart::Response => &self.response_body,
        }
    }
}

pub(crate) struct DetailStore {
    pub segments: SegmentStore,
    /// Ids below this were cleared.
    base: AtomicU32,
    live: Mutex<HashMap<FlowId, Arc<LiveFlow>>>,
    persisted: Mutex<HashMap<FlowId, Extent>>,
    cache: Cache<FlowId, Arc<StoredDetail>>,
    ws: Mutex<HashMap<FlowId, Arc<WsLog>>>,
    sse: Mutex<HashMap<FlowId, Arc<SseLog>>>,
}

impl DetailStore {
    pub fn new(segments: SegmentStore) -> Self {
        DetailStore {
            segments,
            base: AtomicU32::new(0),
            live: Mutex::new(HashMap::new()),
            persisted: Mutex::new(HashMap::new()),
            cache: Cache::new(CACHE_ENTRIES),
            ws: Mutex::new(HashMap::new()),
            sse: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_cleared(&self, id: FlowId) -> bool {
        id < self.base.load(Ordering::Acquire)
    }

    pub fn add_live(&self, flow: Arc<LiveFlow>) {
        if !self.is_cleared(flow.id()) {
            self.live.lock().insert(flow.id(), flow);
        }
    }

    /// Stores a finished flow and retires its live recorder.
    pub fn persist(&self, id: FlowId, stored: Arc<StoredDetail>) -> io::Result<()> {
        if self.is_cleared(id) {
            self.live.lock().remove(&id);
            return Ok(());
        }
        let encoded = postcard::to_allocvec(&*stored).map_err(io::Error::other)?;
        let result = self.segments.append(id, &encoded).map(|extent| {
            self.persisted.lock().insert(id, extent);
        });
        // Cached either way, so the detail stays readable for a while even if
        // the write failed.
        self.cache.insert(id, stored);
        // After the index insert, so a reader that misses the live map finds
        // the persisted copy.
        self.live.lock().remove(&id);
        result
    }

    /// The detail and body locations of a flow, live or finished.
    pub fn get(&self, id: FlowId) -> io::Result<Option<Arc<StoredDetail>>> {
        if self.is_cleared(id) {
            return Ok(None);
        }
        let live = self.live.lock().get(&id).cloned();
        if let Some(flow) = live {
            return Ok(Some(Arc::new(flow.snapshot())));
        }
        if let Some(stored) = self.cache.get(&id) {
            return Ok(Some(stored));
        }
        let Some(extent) = self.persisted.lock().get(&id).copied() else {
            return Ok(None);
        };
        let bytes = self.segments.read(extent)?;
        let stored: StoredDetail = postcard::from_bytes(&bytes).map_err(io::Error::other)?;
        let stored = Arc::new(stored);
        self.cache.insert(id, stored.clone());
        Ok(Some(stored))
    }

    /// A body's captured bytes, still content-encoded.
    pub fn body(&self, id: FlowId, part: BodyPart) -> io::Result<Option<Vec<u8>>> {
        if self.is_cleared(id) {
            return Ok(None);
        }
        let live = self.live.lock().get(&id).cloned();
        if let Some(flow) = live {
            return flow.body_snapshot(part, &self.segments).map(Some);
        }
        match self.get(id)? {
            Some(stored) => stored.body(part).read(&self.segments).map(Some),
            None => Ok(None),
        }
    }

    pub fn add_ws(&self, id: FlowId, log: Arc<WsLog>) {
        if !self.is_cleared(id) {
            self.ws.lock().insert(id, log);
        }
    }

    pub fn ws(&self, id: FlowId) -> Option<Arc<WsLog>> {
        self.ws.lock().get(&id).cloned()
    }

    pub fn add_sse(&self, id: FlowId, log: Arc<SseLog>) {
        if !self.is_cleared(id) {
            self.sse.lock().insert(id, log);
        }
    }

    pub fn sse(&self, id: FlowId) -> Option<Arc<SseLog>> {
        self.sse.lock().get(&id).cloned()
    }

    /// Forgets evicted flows. Their segment files go once nothing kept uses
    /// them: `kept` are the older flows that stay (pinned or in flight),
    /// and `through` the newest evicted id. Returns the bytes freed.
    pub fn evict(&self, evicted: &[FlowId], kept: &[FlowId], through: FlowId) -> u64 {
        {
            let mut persisted = self.persisted.lock();
            let mut live = self.live.lock();
            let mut ws = self.ws.lock();
            let mut sse = self.sse.lock();
            for id in evicted {
                persisted.remove(id);
                live.remove(id);
                ws.remove(id);
                sse.remove(id);
                self.cache.invalidate(id);
            }
        }
        let mut keep = HashSet::new();
        for &id in kept {
            self.segments_used(id, &mut keep);
        }
        self.segments.drop_through(through, &keep)
    }

    /// Adds the segment files a flow's detail, bodies and messages live in.
    fn segments_used(&self, id: FlowId, out: &mut HashSet<u32>) {
        if let Some(extent) = self.persisted.lock().get(&id) {
            out.insert(extent.seg);
        }
        let live = self.live.lock().get(&id).cloned();
        match live {
            Some(flow) => flow.segments_used(out),
            None => {
                if let Ok(Some(stored)) = self.get(id) {
                    stored.request_body.segments_used(out);
                    stored.response_body.segments_used(out);
                }
            }
        }
        if let Some(log) = self.ws(id) {
            log.segments_used(out);
        }
    }

    /// Forgets every flow below `base` and deletes the segment files.
    pub fn clear(&self, base: FlowId) {
        self.base.store(base, Ordering::Release);
        // Every flow so far is below `base`, live ones included.
        self.ws.lock().clear();
        self.sse.lock().clear();
        self.live.lock().retain(|&id, _| id >= base);
        self.persisted.lock().clear();
        self.cache.invalidate_all();
        self.segments.clear();
    }
}
