//! Per-flow recording. A [`FlowRecorder`] is how traffic handlers (the
//! proxy now; replay and WebSockets later) report a flow: heads and body
//! chunks go into the live detail and the body captures, summary changes go
//! to the ingest thread as small events.

use std::io;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{BodyPart, ContentKind, FlowDetail, FlowId, FlowState, ResponseHead, Timing};

use crate::Engine;
use crate::ingest::FlowEvent;
use crate::io::content::{self, GRAPHQL_MAX, SNIFF_LEN};
use crate::io::decode;
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
    /// When the request went to the upstream; TTFB counts from here.
    request_sent: Option<Instant>,
    /// When the response head arrived; transfer time counts from here.
    response_at: Option<Instant>,
    /// The response's kind going by its `Content-Type`, until its body has
    /// been seen.
    response_hint: ContentKind,
    /// Set once the request body has been through detection.
    request_detected: bool,
    graphql_op: Option<Arc<str>>,
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
                request_sent: None,
                response_at: None,
                response_hint: ContentKind::None,
                request_detected: false,
                graphql_op: None,
                finished: None,
            }),
        });
        engine.details().add_live(flow.clone());
        FlowRecorder { flow }
    }

    pub fn id(&self) -> FlowId {
        self.flow.id
    }

    /// Records how the upstream connection was set up: DNS, connect and TLS
    /// times for a new connection, or that an existing one was reused.
    pub fn connection_timing(&self, timing: &Timing) {
        let mut state = self.flow.state.lock();
        let t = &mut state.detail.timing;
        t.dns_ms = timing.dns_ms;
        t.connect_ms = timing.connect_ms;
        t.tls_ms = timing.tls_ms;
        t.reused_connection = timing.reused_connection;
    }

    /// Marks the request as handed to the upstream, which starts TTFB.
    pub fn request_sent(&self) {
        self.flow.state.lock().request_sent = Some(Instant::now());
    }

    /// Records the response head; the status and content kind reach the
    /// flow table on the next ingest batch. `content_kind` is the header's
    /// guess; the body is sniffed once it has finished.
    pub fn response(&self, head: ResponseHead, content_kind: ContentKind) {
        let status = head.status;
        let content_kind = {
            let mut state = self.flow.state.lock();
            if state.finished.is_some() {
                return;
            }
            state.response_hint = content_kind;
            let now = Instant::now();
            let sent = state.request_sent.unwrap_or(self.flow.started);
            state.detail.timing.ttfb_ms = Some(ms(now - sent));
            state.response_at = Some(now);
            state.detail.response_body.encoding = content_encoding(&head.headers);
            state.detail.response = Some(head);
            content::flow_kind(state.detail.request_body.kind, content_kind)
        };
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
                let Some(engine) = self.flow.engine.upgrade() else {
                    return;
                };
                let event = {
                    let mut state = self.flow.state.lock();
                    if state.finished.is_some() {
                        return;
                    }
                    state.detail.request_body.complete = end == BodyEnd::Complete;
                    state.detect_request(&engine.details().segments);
                    let response = match state.detail.response_body.kind {
                        ContentKind::None => state.response_hint,
                        kind => kind,
                    };
                    FlowEvent::Content {
                        id: self.flow.id,
                        content_kind: content::flow_kind(state.detail.request_body.kind, response),
                        graphql_op: state.graphql_op.clone(),
                    }
                };
                engine.ingest().send(event);
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

    /// Live byte counts of a tunnel (nothing is captured).
    pub fn tunnel_progress(&self, sent: u64, received: u64) {
        self.send(FlowEvent::Progress {
            id: self.flow.id,
            req_size: sent,
            resp_size: received,
        });
    }

    /// Finishes a passthrough tunnel with the bytes relayed each way.
    pub fn tunnel_closed(&self, sent: u64, received: u64) {
        {
            let mut state = self.flow.state.lock();
            state.request.size = sent;
            state.response.size = received;
        }
        self.finish(FlowState::Tunnel, None);
    }

    fn finish(&self, end_state: FlowState, error: Option<String>) {
        let Some(engine) = self.flow.engine.upgrade() else {
            return;
        };
        let details = engine.details();
        let duration_ms = self.flow.started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let (stored, req_size, resp_size, content) = {
            let mut state = self.flow.state.lock();
            if state.finished.is_some() {
                return;
            }
            let state = &mut *state;
            let content = (end_state != FlowState::Tunnel).then(|| {
                if !state.request_detected {
                    state.detect_request(&details.segments);
                }
                state.detect_response();
                FlowEvent::Content {
                    id: self.flow.id,
                    content_kind: content::flow_kind(state.detail.request_body.kind, state.detail.response_body.kind),
                    graphql_op: state.graphql_op.clone(),
                }
            });
            if end_state == FlowState::Completed {
                state.detail.response_body.complete = true;
            }
            state.detail.error = error;
            if let Some(response_at) = state.response_at {
                state.detail.timing.transfer_ms = Some(ms(response_at.elapsed()));
            }
            fill_body_info(&mut state.detail.request_body, &state.request);
            fill_body_info(&mut state.detail.response_body, &state.response);
            let stored = Arc::new(StoredDetail {
                detail: state.detail.clone(),
                request_body: state.request.finish(&details.segments),
                response_body: state.response.finish(&details.segments),
            });
            state.finished = Some(stored.clone());
            (stored, state.request.size, state.response.size, content)
        };
        let _ = details.persist(self.flow.id, stored);
        if let Some(content) = content {
            engine.ingest().send(content);
        }
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
    /// Sniffs the request body, and recognises GraphQL operations (which
    /// can need the whole, normally small, body).
    fn detect_request(&mut self, segments: &SegmentStore) {
        self.request_detected = true;
        let request = &self.detail.request;
        let content_type = header_value(&request.headers, "content-type");
        let query = request.url.split_once('?').map_or("", |(_, q)| q);
        let query = query.split('#').next().unwrap_or_default();
        let capture = &self.request;
        let whole = if capture.captured as usize <= GRAPHQL_MAX && !capture.truncated {
            capture.snapshot(segments).ok()
        } else {
            None
        };
        let encoding = self.detail.request_body.encoding.as_deref();
        let decoded = |bytes: &[u8], limit: usize| match encoding {
            Some(encoding) => decode::decode_prefix(encoding, bytes, limit),
            None => bytes[..bytes.len().min(limit)].to_vec(),
        };
        let body = match &whole {
            Some(bytes) => decoded(bytes, GRAPHQL_MAX),
            None => decoded(&capture.head, SNIFF_LEN),
        };
        let detected = content::detect(content_type, &body[..body.len().min(SNIFF_LEN)]);
        let graphql = whole
            .is_some()
            .then(|| content::graphql_request(content_type, &body, query))
            .flatten();
        let info = &mut self.detail.request_body;
        info.kind = if graphql.is_some() {
            ContentKind::GraphQl
        } else {
            detected.kind
        };
        info.mime = detected.mime;
        self.graphql_op = graphql.and_then(|g| g.operation).map(Arc::from);
    }

    fn detect_response(&mut self) {
        let Some(response) = &self.detail.response else {
            return;
        };
        let content_type = header_value(&response.headers, "content-type");
        let head = &self.response.head;
        let head = match &self.detail.response_body.encoding {
            Some(encoding) => decode::decode_prefix(encoding, head, SNIFF_LEN),
            None => head.clone(),
        };
        let detected = content::detect(content_type, &head);
        let info = &mut self.detail.response_body;
        info.kind = detected.kind;
        info.mime = detected.mime;
    }

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

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn fill_body_info(info: &mut pm_model::BodyInfo, capture: &Capture) {
    info.size = capture.size;
    info.captured = capture.captured;
    info.truncated = capture.truncated;
}

fn header_value<'a>(headers: &'a [pm_model::Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// The `Content-Encoding` header, if any, lower-cased.
pub(crate) fn content_encoding(headers: &[pm_model::Header]) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-encoding"))
        .map(|h| h.value.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "identity")
}
