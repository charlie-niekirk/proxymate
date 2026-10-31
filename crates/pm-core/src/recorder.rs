//! Per-flow recording. A [`FlowRecorder`] is how traffic handlers (the
//! proxy now; replay later) report a flow: heads and body chunks go into
//! the live detail and the body captures, summary changes go to the ingest
//! thread as small events. A WebSocket flow records its messages through a
//! [`WsRecorder`]; an event-stream response is parsed into events as its
//! body streams.

use std::io;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{
    AppliedRule, BodyPart, ContentKind, FlowDetail, FlowId, FlowState, RequestHead, ResponseHead, Timing, WsClose,
    WsDirection, WsOpcode,
};

use crate::Engine;
use crate::ingest::FlowEvent;
use crate::io::content::{self, GRAPHQL_MAX, SNIFF_LEN};
use crate::io::{decode, grpc};
use crate::proxy::now_ms;
use crate::store::{Capture, SegmentStore, SseLog, StoredDetail, WsLog};

/// gRPC-web bodies up to this size are read at the end for their trailer
/// frame.
const GRPC_WEB_TRAILER_MAX: usize = 8 * 1024 * 1024;
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
    /// Events parsed from a `text/event-stream` response so far.
    sse: Option<Arc<SseLog>>,
    /// A `Block` rule answered: the flow ends as Blocked, not Completed.
    blocked: bool,
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
                request: Capture::new(id),
                response: Capture::new(id),
                last_progress: now,
                request_sent: None,
                response_at: None,
                response_hint: ContentKind::None,
                request_detected: false,
                graphql_op: None,
                finished: None,
                sse: None,
                blocked: false,
            }),
        });
        engine.details().add_live(flow.clone());
        let recorder = FlowRecorder { flow };
        recorder.header_cells(BodyPart::Request);
        recorder
    }

    /// Sends the header column values (M17) of one side, if any column is
    /// about it.
    fn header_cells(&self, part: BodyPart) {
        let Some(engine) = self.flow.engine.upgrade() else {
            return;
        };
        let event = {
            let state = self.flow.state.lock();
            match part {
                BodyPart::Request => engine.header_cells(self.flow.id, part, &state.detail.request.headers),
                BodyPart::Response => match &state.detail.response {
                    Some(response) => engine.header_cells(self.flow.id, part, &response.headers),
                    None => None,
                },
            }
        };
        if let Some(event) = event {
            engine.ingest().send(event);
        }
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

    /// How the upstream connection was made (M18). A reverse proxy's port,
    /// set when the request arrived, is kept.
    pub fn via(&self, via: &pm_model::Via) {
        let mut state = self.flow.state.lock();
        let reverse_port = state.detail.via.as_ref().and_then(|v| v.reverse_port);
        state.detail.via = Some(pm_model::Via {
            reverse_port: via.reverse_port.or(reverse_port),
            ..via.clone()
        });
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
            // A compressed stream is parsed when asked for instead.
            if is_event_stream(&head.headers)
                && state.detail.response_body.encoding.is_none()
                && let Some(engine) = self.flow.engine.upgrade()
            {
                let log = Arc::new(SseLog::new());
                engine.details().add_sse(self.flow.id, log.clone());
                state.sse = Some(log);
            }
            state.detail.response = Some(head);
            content::flow_kind(state.detail.request_body.kind, content_kind)
        };
        self.send(FlowEvent::Response {
            id: self.flow.id,
            status,
            content_kind,
        });
        self.header_cells(BodyPart::Response);
    }

    /// Replaces the recorded request head with what rules (or a
    /// breakpoint) turned it into: the flow shows what was sent.
    pub fn request_head(&self, head: RequestHead) {
        let mut state = self.flow.state.lock();
        if state.finished.is_some() {
            return;
        }
        state.detail.request_body.encoding = content_encoding(&head.headers);
        state.detail.request = head;
        drop(state);
        self.header_cells(BodyPart::Request);
    }

    /// The request head as recorded so far (after rewrites).
    pub fn request_snapshot(&self) -> Option<RequestHead> {
        Some(self.flow.state.lock().detail.request.clone())
    }

    /// Records rules that changed the flow, and marks it edited.
    pub fn rules_applied(&self, applied: Vec<AppliedRule>) {
        if applied.is_empty() {
            return;
        }
        {
            let mut state = self.flow.state.lock();
            if state.finished.is_some() {
                return;
            }
            let rules = &mut state.detail.rules.applied;
            for rule in applied {
                if !rules.iter().any(|r| r.rule_id == rule.rule_id && r.phase == rule.phase) {
                    rules.push(rule);
                }
            }
        }
        self.send(FlowEvent::Edited { id: self.flow.id });
    }

    /// A body action or breakpoint couldn't touch a body; it went through
    /// unchanged.
    pub fn rewrite_skipped(&self, reason: impl Into<String>) {
        let reason = reason.into();
        let mut state = self.flow.state.lock();
        let skipped = &mut state.detail.rules.rewrite_skipped;
        match skipped {
            Some(existing) if !existing.contains(&reason) => {
                existing.push_str("; ");
                existing.push_str(&reason);
            }
            Some(_) => {}
            None => *skipped = Some(reason),
        }
    }

    /// A script failed on this flow; it went on unmodified by that script.
    pub(crate) fn script_issue(&self, issue: pm_model::ScriptIssue) {
        let mut state = self.flow.state.lock();
        let issues = &mut state.detail.rules.scripts;
        if !issues.contains(&issue) {
            issues.push(issue);
        }
    }

    /// The flow is held at a breakpoint (the violet Paused pill), or goes on.
    pub fn paused(&self, paused: bool) {
        let state = if paused { FlowState::Paused } else { FlowState::Active };
        self.send(FlowEvent::State {
            id: self.flow.id,
            state,
        });
    }

    /// A breakpoint timed out; the flow went on unmodified.
    pub fn breakpoint_timeout(&self) {
        self.flow.state.lock().detail.rules.breakpoint_timeout = true;
    }

    /// The response came from Map Local or a breakpoint, not the upstream.
    pub fn mocked(&self) {
        self.flow.state.lock().detail.rules.mocked = true;
    }

    /// A `Block` rule answered with a status: the flow ends Blocked.
    pub fn blocked(&self) {
        self.flow.state.lock().blocked = true;
    }

    /// A `Block` rule (or a breakpoint's Abort) reset the connection.
    pub fn reset(&self, reason: impl Into<String>, blocked: bool) {
        let state = if blocked { FlowState::Blocked } else { FlowState::Failed };
        self.finish(state, Some(reason.into()));
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
        if part == BodyPart::Response
            && let Some(sse) = &state.sse
        {
            sse.push(data, now_ms());
        }
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

    /// Records HTTP/2 trailers (gRPC's status). Request trailers are rare
    /// and not kept.
    pub fn trailers(&self, part: BodyPart, trailers: &http::HeaderMap) {
        if part != BodyPart::Response {
            return;
        }
        let mut state = self.flow.state.lock();
        if state.finished.is_some() {
            return;
        }
        let list = state.detail.trailers.get_or_insert_with(Vec::new);
        list.extend(
            trailers
                .iter()
                .map(|(name, value)| pm_model::Header::new(name.as_str(), String::from_utf8_lossy(value.as_bytes()))),
        );
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

    /// Switches the flow to a WebSocket once the upstream has answered
    /// `101`: from here on it records messages, and stays Active until
    /// [`WsRecorder::closed`].
    pub fn websocket(&self) -> WsRecorder {
        let log = Arc::new(WsLog::new(self.flow.id));
        if let Some(engine) = self.flow.engine.upgrade() {
            engine.details().add_ws(self.flow.id, log.clone());
        }
        self.flow.state.lock().detail.request_body.complete = true;
        WsRecorder {
            flow: self.clone(),
            log,
        }
    }

    fn finish(&self, end_state: FlowState, error: Option<String>) {
        self.finish_with(end_state, error, None);
    }

    /// Finishes the flow. `sizes` overrides the summary's (request,
    /// response) byte counts, which otherwise come from the body captures.
    fn finish_with(&self, end_state: FlowState, error: Option<String>, sizes: Option<(u64, u64)>) {
        let Some(engine) = self.flow.engine.upgrade() else {
            return;
        };
        let details = engine.details();
        let duration_ms = self.flow.started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let (stored, req_size, resp_size, content, end_state) = {
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
                state.grpc_trailers(&details.segments);
                FlowEvent::Content {
                    id: self.flow.id,
                    content_kind: content::flow_kind(state.detail.request_body.kind, state.detail.response_body.kind),
                    graphql_op: state.graphql_op.clone(),
                }
            });
            if end_state == FlowState::Completed {
                state.detail.response_body.complete = true;
            }
            let end_state = if end_state == FlowState::Completed && state.blocked {
                FlowState::Blocked
            } else {
                end_state
            };
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
            if let Some(sse) = &state.sse {
                sse.finish();
            }
            let (req_size, resp_size) = sizes.unwrap_or((state.request.size, state.response.size));
            (stored, req_size, resp_size, content, end_state)
        };
        let grpc_status = stored.detail.response.as_ref().and_then(|r| {
            let trailers = stored.detail.trailers.as_deref().unwrap_or_default();
            grpc::status_code(trailers).or_else(|| grpc::status_code(&r.headers))
        });
        let _ = details.persist(self.flow.id, stored);
        if let Some(content) = content {
            engine.ingest().send(content);
        }
        if let Some(status) = grpc_status {
            engine.ingest().send(FlowEvent::Grpc {
                id: self.flow.id,
                status,
            });
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

/// Records the messages of one WebSocket flow. Cheap to clone.
#[derive(Clone)]
pub struct WsRecorder {
    flow: FlowRecorder,
    log: Arc<WsLog>,
}

impl WsRecorder {
    /// Records one message as it is relayed. `payload` is the whole message
    /// (the close reason, for a close frame); only the first 1 MB is kept.
    pub fn message(&self, direction: WsDirection, opcode: WsOpcode, payload: &[u8], close_code: Option<u16>) {
        let Some(engine) = self.flow.flow.engine.upgrade() else {
            return;
        };
        let details = engine.details();
        let id = self.flow.flow.id;
        if details.is_cleared(id) {
            return;
        }
        let max = engine.storage_settings().max_ws_messages as usize;
        self.log
            .push(&details.segments, direction, opcode, payload, close_code, now_ms(), max);
        engine.ingest().send(FlowEvent::WsMsg {
            id,
            direction,
            len: payload.len() as u64,
        });
    }

    /// Finishes the flow: completed with how it closed, or failed with
    /// `error` (a protocol or I/O error on either side).
    pub fn closed(&self, close: Option<WsClose>, error: Option<String>) {
        self.log.close(close);
        let state = if error.is_some() {
            FlowState::Failed
        } else {
            FlowState::Completed
        };
        self.flow.finish_with(state, error, Some(self.log.totals()));
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

    /// Adds the segment files the flow's bodies live in so far.
    pub fn segments_used(&self, out: &mut std::collections::HashSet<u32>) {
        let state = self.state.lock();
        match &state.finished {
            Some(stored) => {
                stored.request_body.segments_used(out);
                stored.response_body.segments_used(out);
            }
            None => {
                state.request.segments_used(out);
                state.response.segments_used(out);
            }
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

    /// A gRPC-web response carries its trailers in a final frame of the
    /// body: kept as the flow's trailers, like HTTP/2 ones.
    fn grpc_trailers(&mut self, segments: &SegmentStore) {
        if self.detail.response_body.kind != ContentKind::Grpc || self.detail.trailers.is_some() {
            return;
        }
        let Some(response) = &self.detail.response else {
            return;
        };
        let flavor = grpc::Flavor::of(header_value(&response.headers, "content-type"));
        if !flavor.is_web() || self.response.truncated || self.response.captured as usize > GRPC_WEB_TRAILER_MAX {
            return;
        }
        let Ok(body) = self.response.snapshot(segments) else {
            return;
        };
        let body = match &self.detail.response_body.encoding {
            Some(encoding) => match decode::decode(encoding, &body) {
                Ok(body) => body,
                Err(_) => return,
            },
            None => body,
        };
        // Compressed messages needn't be decompressed to find the trailers.
        let framed = grpc::split(&body, flavor, None);
        if !framed.trailers.is_empty() {
            self.detail.trailers = Some(framed.trailers);
        }
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

/// True for a `text/event-stream` response (server-sent events).
pub(crate) fn is_event_stream(headers: &[pm_model::Header]) -> bool {
    header_value(headers, "content-type")
        .and_then(|v| v.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
}

/// The `Content-Encoding` header, if any, lower-cased.
pub(crate) fn content_encoding(headers: &[pm_model::Header]) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-encoding"))
        .map(|h| h.value.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "identity")
}
