//! Per-flow recording. A [`FlowRecorder`] is how traffic handlers (the
//! proxy now; replay and WebSockets later) report a flow: heads and body
//! chunks go into the live detail and the body captures, summary changes go
//! to the ingest thread as small events.

use std::io;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{BodyPart, ContentKind, FlowDetail, FlowId, FlowState, ResponseHead};

use crate::Engine;
use crate::ingest::FlowEvent;
use crate::store::{Capture, SegmentStore, StoredDetail};

/// Size updates for a streaming body go out at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// How a body stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyEnd {
    Complete,
    Error(String),
    /// Dropped before the end, e.g. the peer went away.
    Aborted,
}

/// Handle to one flow being recorded. Cheap to clone; the request and
/// response body taps each hold one.
#[derive(Clone)]
pub struct FlowRecorder {
    flow: Arc<LiveFlow>,
}

pub(crate) struct LiveFlow {
    id: FlowId,
    engine: Weak<Engine>,
    started: Instant,
    state: Mutex<LiveState>,
}

struct LiveState {
    detail: FlowDetail,
    request: Capture,
    response: Capture,
    last_progress: Instant,
    /// Set once the flow has finished and been persisted.
    finished: Option<Arc<StoredDetail>>,
}

impl FlowRecorder {
    pub(crate) fn new(id: FlowId, engine: &Arc<Engine>, mut detail: FlowDetail) -> Self {
        detail.id = id;
        let now = Instant::now();
        let flow = Arc::new(LiveFlow {
            id,
            engine: Arc::downgrade(engine),
            started: now,
            state: Mutex::new(LiveState {
                detail,
                request: Capture::default(),
                response: Capture::default(),
                last_progress: now,
                finished: None,
            }),
        });
        engine.details().add_live(flow.clone());
        FlowRecorder { flow }
    }

    pub fn id(&self) -> FlowId {
        self.flow.id
    }

    /// Records the response head; the status and content kind reach the
    /// flow table on the next ingest batch.
    pub fn response(&self, head: ResponseHead, content_kind: ContentKind) {
        let status = head.status;
        {
            let mut state = self.flow.state.lock();
            if state.finished.is_some() {
                return;
            }
            state.detail.response_body.encoding = content_encoding(&head.headers);
            state.detail.response = Some(head);
        }
        self.send(FlowEvent::Response {
            id: self.flow.id,
            status,
            content_kind,
        });
    }

    /// Tees one chunk of a body.
    pub fn body_data(&self, part: BodyPart, data: &[u8]) {
        let Some(engine) = self.flow.engine.upgrade() else {
            return;
        };
        let details = engine.details();
        if details.is_cleared(self.flow.id) {
            return;
        }
        let mut state = self.flow.state.lock();
        if state.finished.is_some() {
            return;
        }
        state.capture_mut(part).push(data, &details.segments);
        if state.last_progress.elapsed() >= PROGRESS_INTERVAL {
            state.last_progress = Instant::now();
            let (req_size, resp_size) = (state.request.size, state.response.size);
            drop(state);
            engine.ingest().send(FlowEvent::Progress {
                id: self.flow.id,
                req_size,
                resp_size,
            });
        }
    }

    /// Marks a body as ended. The end of the response body finishes the flow.
    pub fn body_end(&self, part: BodyPart, end: BodyEnd) {
        match part {
            BodyPart::Request => {
                let mut state = self.flow.state.lock();
                state.detail.request_body.complete = end == BodyEnd::Complete;
            }
            BodyPart::Response => match end {
                BodyEnd::Complete => self.finish(FlowState::Completed, None),
                BodyEnd::Error(err) => self.finish(FlowState::Failed, Some(format!("response body: {err}"))),
                BodyEnd::Aborted => self.finish(
                    FlowState::Failed,
                    Some("the client closed the connection before the response finished".into()),
                ),
            },
        }
    }

    /// Finishes the flow as failed, e.g. the upstream was unreachable.
    pub fn fail(&self, error: impl Into<String>) {
        self.finish(FlowState::Failed, Some(error.into()));
    }

    fn finish(&self, end_state: FlowState, error: Option<String>) {
        let Some(engine) = self.flow.engine.upgrade() else {
            return;
        };
        let details = engine.details();
        let duration_ms = self.flow.started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let (stored, req_size, resp_size) = {
            let mut state = self.flow.state.lock();
            if state.finished.is_some() {
                return;
            }
            let state = &mut *state;
            if end_state == FlowState::Completed {
                state.detail.response_body.complete = true;
            }
            state.detail.error = error;
            fill_body_info(&mut state.detail.request_body, &state.request);
            fill_body_info(&mut state.detail.response_body, &state.response);
            let stored = Arc::new(StoredDetail {
                detail: state.detail.clone(),
                request_body: state.request.finish(&details.segments),
                response_body: state.response.finish(&details.segments),
            });
            state.finished = Some(stored.clone());
            (stored, state.request.size, state.response.size)
        };
        let _ = details.persist(self.flow.id, stored);
        engine.ingest().send(FlowEvent::Finished {
            id: self.flow.id,
            state: end_state,
            duration_ms,
            req_size,
            resp_size,
        });
    }

    fn send(&self, event: FlowEvent) {
        if let Some(engine) = self.flow.engine.upgrade() {
            engine.ingest().send(event);
        }
    }
}

impl LiveFlow {
    pub fn id(&self) -> FlowId {
        self.id
    }

    /// The detail as of now, with body sizes filled in.
    pub fn snapshot(&self) -> StoredDetail {
        let state = self.state.lock();
        if let Some(stored) = &state.finished {
            return StoredDetail::clone(stored);
        }
        let mut detail = state.detail.clone();
        fill_body_info(&mut detail.request_body, &state.request);
        fill_body_info(&mut detail.response_body, &state.response);
        StoredDetail {
            detail,
            request_body: Default::default(),
            response_body: Default::default(),
        }
    }

    pub fn body_snapshot(&self, part: BodyPart, segments: &SegmentStore) -> io::Result<Vec<u8>> {
        let state = self.state.lock();
        match &state.finished {
            Some(stored) => stored.body(part).read(segments),
            None => state.capture(part).snapshot(segments),
        }
    }
}

impl LiveState {
    fn capture(&self, part: BodyPart) -> &Capture {
        match part {
            BodyPart::Request => &self.request,
            BodyPart::Response => &self.response,
        }
    }

    fn capture_mut(&mut self, part: BodyPart) -> &mut Capture {
        match part {
            BodyPart::Request => &mut self.request,
            BodyPart::Response => &mut self.response,
        }
    }
}

fn fill_body_info(info: &mut pm_model::BodyInfo, capture: &Capture) {
    info.size = capture.size;
    info.captured = capture.captured;
    info.truncated = capture.truncated;
}

/// The `Content-Encoding` header, if any, lower-cased.
pub(crate) fn content_encoding(headers: &[pm_model::Header]) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-encoding"))
        .map(|h| h.value.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "identity")
}
