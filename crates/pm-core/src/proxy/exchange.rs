//! One request and its response, with rules applied. Both the forward
//! proxy (`http://` in absolute form, or plain HTTP in a CONNECT) and the
//! MITM (decrypted h1 and h2) send every non-WebSocket request through
//! [`Exchange::run`].
//!
//! Without a matching rule the bodies stream through untouched, exactly as
//! before rules existed. With one, the order of `rules::eval` applies, and
//! the body policy:
//!
//! - Bodies stream by default.
//! - When a body action or breakpoint applies to a message, its body is
//!   buffered up to [`REWRITE_BODY_CAP`], decoded, transformed, and re-sent
//!   identity-encoded with a fresh `Content-Length` (h1 chunking is
//!   dropped).
//! - A larger body is forwarded untouched as it arrives (what was read so
//!   far, then the rest of the stream) and the flow is marked
//!   `rewrite_skipped`. Reading never waits on the other side, so this
//!   can't deadlock.
//!
//! A breakpoint parks only its own exchange: other h2 streams on the same
//! connection keep flowing.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderValue};
use http::uri::{Authority, PathAndQuery};
use http::{Request, Response, StatusCode, Version, request, response};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use pm_model::{
    BodyPart, BreakpointBody, BreakpointInfo, BreakpointResolution, MessageEdits, Phase, RequestHead, Scheme,
};

use super::body::{Buffered, Inbound, RuleBody, buffer};
use super::mitm::{self, MitmConn};
use super::{
    BoxError, ProxyBody, ProxyCtx, Reset, TeeBody, content_kind, error_chain, headers_of, now_ms, response_head,
    strip_hop_by_hop, text_response,
};
use crate::Engine;
use crate::client::PeerClient;
use crate::io::decode;
use crate::recorder::FlowRecorder;
use crate::rules::compile::CompiledRule;
use crate::rules::eval::{self, RequestDraft, ResponseDraft, ThrottleSpec, header_map, reframe};
use crate::rules::{BREAKPOINT_EDIT_CAP, Outcome, REWRITE_BODY_CAP, local};

/// How long a request waits for its client app when a rule matches on it.
const CLIENT_APP_WAIT: Duration = Duration::from_millis(300);

/// Where a request goes when no rule sends it elsewhere.
pub(super) enum Route<'a> {
    /// The pooled plain-HTTP client.
    Forward,
    /// The intercepted connection's own upstream.
    Mitm(&'a MitmConn),
}

pub(super) struct Exchange<'a> {
    pub ctx: &'a ProxyCtx,
    pub route: Route<'a>,
    pub peer: &'a Arc<PeerClient>,
    pub recorder: Option<FlowRecorder>,
    /// The request's origin as the client addressed it.
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    /// False for `Engine::send` without rules: straight to the upstream.
    pub apply_rules: bool,
}

type Reply = Result<Response<ProxyBody>, Reset>;

/// A body that was (or wasn't) read ahead.
enum Held {
    Stream(Inbound),
    /// Over the cap: the part read, then the rest.
    Over(std::collections::VecDeque<Bytes>, Inbound),
    /// Read whole but not decodable; sent back as is.
    Raw(Bytes),
    /// Read whole and decoded into the draft; these are the wire bytes.
    Decoded(Bytes),
}

impl Held {
    fn into_body(self) -> RuleBody {
        match self {
            Held::Stream(body) => RuleBody::stream(body),
            Held::Over(prefix, rest) => RuleBody::chain(prefix, rest),
            Held::Raw(bytes) | Held::Decoded(bytes) => RuleBody::whole(bytes),
        }
    }

    /// Reads whatever is still to come and throws it away (or records it,
    /// with a recorder), so an h1 client can finish sending.
    fn drain(self, recorder: Option<FlowRecorder>, part: BodyPart) {
        let body = TeeBody::new(self.into_body(), recorder, part);
        tokio::spawn(async move {
            let _ = body.collect().await;
        });
    }

    /// Whether a paused body can be edited, and why not.
    fn editability(&self, body: Option<&Vec<u8>>) -> (bool, Option<String>) {
        match (self, body) {
            (_, Some(b)) if b.len() <= BREAKPOINT_EDIT_CAP => (true, None),
            (_, Some(_)) => (
                false,
                Some(format!(
                    "The body is larger than {} MB, so only the head can be edited.",
                    BREAKPOINT_EDIT_CAP / 1024 / 1024
                )),
            ),
            (Held::Over(..), None) => (
                false,
                Some(format!(
                    "The body is larger than {} MB; it is forwarded as is.",
                    REWRITE_BODY_CAP / 1024 / 1024
                )),
            ),
            (Held::Raw(_), None) => (
                false,
                Some("The body couldn't be decoded; it is forwarded as is.".into()),
            ),
            _ => (true, None),
        }
    }
}

/// What a response phase starts from.
enum Answer {
    Upstream(Response<Incoming>),
    /// Map Local's answer.
    Local(ResponseDraft),
}

impl Exchange<'_> {
    pub async fn run(self, req: Request<impl Into<Inbound>>) -> Reply {
        let (mut parts, body) = req.into_parts();
        let body = body.into();
        strip_hop_by_hop(&mut parts.headers);
        let Some(engine) = self.ctx.engine.upgrade().filter(|_| self.apply_rules) else {
            return self.plain(parts, body).await;
        };
        let rules = engine.rule_set();
        if rules.is_empty() {
            return self.plain(parts, body).await;
        }
        let draft = self.draft(&parts);
        let app = match rules.needs_client_app() {
            true => self.peer.app(CLIENT_APP_WAIT).await.map(|a| a.name.clone()),
            false => None,
        };
        let matched = rules.matching(&draft.info(app.as_deref()));
        if matched.is_empty() {
            return self.plain(parts, body).await;
        }
        self.with_rules(&engine, parts, body, draft, matched).await
    }

    /// No rule applies: stream both ways.
    async fn plain(&self, parts: request::Parts, body: Inbound) -> Reply {
        let pq = parts
            .uri
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"));
        let body = TeeBody::new(RuleBody::stream(body), self.recorder.clone(), BodyPart::Request);
        let target = Target::new(self.scheme, &self.host, self.port);
        let upstream = match self.send(parts, pq, &target, body).await {
            Ok(response) => response,
            Err(message) => return Ok(self.fail(message)),
        };
        // The recorded head is what the upstream sent, before hop-by-hop stripping.
        let head = response_head(&upstream);
        let (mut parts, body) = upstream.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        Ok(self.reply(Response::from_parts(parts, RuleBody::stream(body)), Some(head)))
    }

    async fn with_rules(
        &self,
        engine: &Arc<Engine>,
        mut parts: request::Parts,
        body: Inbound,
        mut draft: RequestDraft,
        matched: Vec<Arc<CompiledRule>>,
    ) -> Reply {
        let rec = self.recorder.clone();
        let blocks = matched.iter().any(|r| {
            r.actions
                .iter()
                .any(|a| matches!(a.action, pm_model::Action::Block { .. }))
        });
        let mut held = Held::Stream(body);
        if !blocks && eval::needs_body(&matched, Phase::Request) {
            let Held::Stream(body) = held else { unreachable!() };
            held = match self.hold(body, &draft.headers, BodyPart::Request).await {
                Ok((held, decoded)) => {
                    draft.body = decoded;
                    held
                }
                Err(err) => {
                    let message = format!("request body: {err}");
                    if let Some(rec) = &rec {
                        rec.fail(message.clone());
                    }
                    return Err(Reset::new(message));
                }
            };
        }

        let mut fx = eval::request_phase(&matched, &mut draft);
        if let Some(rec) = &rec {
            rec.rules_applied(std::mem::take(&mut fx.applied));
            for reason in &fx.body_skipped {
                rec.rewrite_skipped(reason.clone());
            }
        }
        let mut changed = fx.head_changed || fx.body_changed;

        if let Some(mode) = fx.block {
            return self.block(mode, held);
        }
        if let Some(remap) = fx.map_remote.take() {
            draft.scheme = remap.scheme;
            draft.host = remap.host;
            draft.port = remap.port;
            if let Some(path) = remap.path {
                draft.path = path;
            }
            if !remap.preserve_host
                && draft.headers.contains_key(header::HOST)
                && let Ok(host) = HeaderValue::from_str(&draft.authority())
            {
                draft.headers.insert(header::HOST, host);
            }
            changed = true;
        }
        if fx.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(fx.delay_ms)).await;
        }

        if let Some(local) = fx.map_local.take() {
            if let Some(rec) = &rec {
                rec.mocked();
                if changed {
                    rec.request_head(self.request_head(&draft, parts.version));
                }
            }
            held.drain(rec.clone(), BodyPart::Request);
            let answer = local::answer(&local, &draft.path).await;
            return self.respond(engine, &matched, Answer::Local(answer), fx.throttle).await;
        }

        if let Some(rule) = fx.breakpoint.take() {
            let (editable, note) = held.editability(draft.body.as_ref());
            let info = BreakpointInfo {
                id: 0,
                flow_id: rec.as_ref().map(FlowRecorder::id),
                phase: Phase::Request,
                rule_id: rule.applied.rule_id.clone(),
                rule_name: rule.applied.name.clone(),
                paused_at_ms: now_ms(),
                deadline_ms: None,
                request: self.request_head(&draft, parts.version),
                response: None,
                body: BreakpointBody {
                    size: draft.body.as_ref().map_or(0, |b| b.len() as u64),
                    editable,
                    note,
                },
            };
            let outcome = self.pause(engine, info, draft.body.clone().unwrap_or_default()).await;
            match outcome {
                Outcome::TimedOut => {}
                Outcome::Resolved(BreakpointResolution::Continue { edits }) => {
                    if let Some(edits) = edits {
                        changed |= edit_request(&mut draft, edits, editable);
                    }
                }
                Outcome::Resolved(BreakpointResolution::Abort) => {
                    held.drain(None, BodyPart::Request);
                    if let Some(rec) = &rec {
                        rec.reset("Aborted at a breakpoint.", false);
                    }
                    return Err(Reset::new("aborted at a breakpoint"));
                }
                Outcome::Resolved(BreakpointResolution::Respond { status, headers, body }) => {
                    held.drain(rec.clone(), BodyPart::Request);
                    if let Some(rec) = &rec {
                        rec.mocked();
                    }
                    let answer = ResponseDraft {
                        status,
                        headers: header_map(&headers),
                        body: Some(body.into_bytes()),
                    };
                    return Ok(self.finish_response(answer, None, true, None, None));
                }
            }
        }

        let body = match draft.body.take() {
            Some(bytes) => {
                held.drain(None, BodyPart::Request);
                reframe(&mut draft.headers, bytes.len());
                changed = true;
                RuleBody::whole(bytes)
            }
            None => held.into_body(),
        };
        let body = body.throttled(fx.throttle.map_or(0, |t| t.up_bps));
        if changed && let Some(rec) = &rec {
            rec.request_head(self.request_head(&draft, parts.version));
        }
        let pq: PathAndQuery = draft
            .path_and_query()
            .parse()
            .unwrap_or_else(|_| PathAndQuery::from_static("/"));
        if let Ok(method) = draft.method.parse() {
            parts.method = method;
        }
        parts.headers = std::mem::take(&mut draft.headers);
        let target = Target::new(draft.scheme, &draft.host, draft.port);
        let body = TeeBody::new(body, rec.clone(), BodyPart::Request);
        let upstream = match self.send(parts, pq, &target, body).await {
            Ok(response) => response,
            Err(message) => return Ok(self.fail(message)),
        };
        self.respond(engine, &matched, Answer::Upstream(upstream), fx.throttle)
            .await
    }

    /// The response phase: gate by each rule's response matcher, rewrite,
    /// pause, then send on.
    async fn respond(
        &self,
        engine: &Arc<Engine>,
        matched: &[Arc<CompiledRule>],
        answer: Answer,
        throttle: Option<ThrottleSpec>,
    ) -> Reply {
        let rec = self.recorder.clone();
        let mut upstream_head = None;
        let (mut parts, mut held, local_body) = match answer {
            Answer::Upstream(response) => {
                upstream_head = Some(response_head(&response));
                let (mut parts, body) = response.into_parts();
                strip_hop_by_hop(&mut parts.headers);
                (parts, Held::Stream(body.into()), None)
            }
            Answer::Local(draft) => {
                let (mut parts, ()) = Response::new(()).into_parts();
                parts.status = StatusCode::from_u16(draft.status).unwrap_or(StatusCode::OK);
                parts.headers = draft.headers;
                (parts, Held::Decoded(Bytes::new()), draft.body)
            }
        };
        let mocked = local_body.is_some();
        let gated = eval::response_rules(matched, parts.status.as_u16(), &parts.headers);
        let mut draft = ResponseDraft {
            status: parts.status.as_u16(),
            headers: parts.headers.clone(),
            body: local_body,
        };
        if draft.body.is_none() && eval::needs_body(&gated, Phase::Response) {
            let Held::Stream(body) = held else { unreachable!() };
            held = match self.hold(body, &draft.headers, BodyPart::Response).await {
                Ok((held, decoded)) => {
                    draft.body = decoded;
                    held
                }
                Err(err) => {
                    let message = format!("The upstream response body failed: {}", error_chain(&err));
                    return Ok(self.fail(message));
                }
            };
        }
        let mut fx = eval::response_phase(&gated, &mut draft);
        if let Some(rec) = &rec {
            rec.rules_applied(std::mem::take(&mut fx.applied));
            for reason in &fx.body_skipped {
                rec.rewrite_skipped(reason.clone());
            }
        }
        let mut changed = mocked || fx.body_changed;

        if let Some(rule) = fx.breakpoint.take() {
            let (editable, note) = held.editability(draft.body.as_ref());
            let request = match &rec {
                Some(rec) => rec.request_snapshot(),
                None => None,
            };
            let info = BreakpointInfo {
                id: 0,
                flow_id: rec.as_ref().map(FlowRecorder::id),
                phase: Phase::Response,
                rule_id: rule.applied.rule_id.clone(),
                rule_name: rule.applied.name.clone(),
                paused_at_ms: now_ms(),
                deadline_ms: None,
                request: request.unwrap_or_else(|| RequestHead {
                    method: String::new(),
                    url: Target::new(self.scheme, &self.host, self.port).url(""),
                    version: pm_model::HttpVersion::Http11,
                    headers: Vec::new(),
                }),
                response: Some(super::head_of(draft.status, &draft.headers, parts.version)),
                body: BreakpointBody {
                    size: draft.body.as_ref().map_or(0, |b| b.len() as u64),
                    editable,
                    note,
                },
            };
            match self.pause(engine, info, draft.body.clone().unwrap_or_default()).await {
                Outcome::TimedOut => {}
                Outcome::Resolved(BreakpointResolution::Continue { edits }) => {
                    if let Some(edits) = edits {
                        changed |= edit_response(&mut draft, edits, editable);
                    }
                }
                Outcome::Resolved(BreakpointResolution::Abort) => {
                    held.drain(None, BodyPart::Response);
                    if let Some(rec) = &rec {
                        rec.reset("Aborted at a breakpoint.", false);
                    }
                    return Err(Reset::new("aborted at a breakpoint"));
                }
                Outcome::Resolved(BreakpointResolution::Respond { status, headers, body }) => {
                    held.drain(None, BodyPart::Response);
                    if let Some(rec) = &rec {
                        rec.mocked();
                    }
                    draft = ResponseDraft {
                        status,
                        headers: header_map(&headers),
                        body: Some(body.into_bytes()),
                    };
                    held = Held::Decoded(Bytes::new());
                    changed = true;
                    parts.extensions.remove::<hyper::ext::ReasonPhrase>();
                }
            }
        }
        let delay = fx.delay_ms + throttle.map_or(0, |t| u64::from(t.latency_ms));
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        if draft.status != parts.status.as_u16() {
            parts.extensions.remove::<hyper::ext::ReasonPhrase>();
        }
        let untouched = !changed && !fx.head_changed && matches!(held, Held::Stream(_));
        let head = upstream_head.filter(|_| untouched);
        Ok(self.finish_response(draft, Some((parts, held)), changed, throttle, head))
    }

    /// Builds the response the client gets and starts recording it.
    fn finish_response(
        &self,
        mut draft: ResponseDraft,
        original: Option<(response::Parts, Held)>,
        changed: bool,
        throttle: Option<ThrottleSpec>,
        head: Option<pm_model::ResponseHead>,
    ) -> Response<ProxyBody> {
        let (mut parts, held) = original.unwrap_or_else(|| {
            let (parts, ()) = Response::new(()).into_parts();
            (parts, Held::Decoded(Bytes::new()))
        });
        let body = match (draft.body.take(), held) {
            // A buffered, decoded, untouched empty body (HEAD, 204, 304)
            // keeps its framing headers.
            (Some(bytes), Held::Decoded(wire)) if !changed && bytes.is_empty() && wire.is_empty() => RuleBody::empty(),
            (Some(bytes), held) => {
                held.drain(None, BodyPart::Response);
                reframe(&mut draft.headers, bytes.len());
                RuleBody::whole(bytes)
            }
            (None, held) => held.into_body(),
        };
        parts.status = StatusCode::from_u16(draft.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        parts.headers = draft.headers;
        let body = body.throttled(throttle.map_or(0, |t| t.down_bps));
        self.reply(Response::from_parts(parts, body), head)
    }

    /// Records the response head and tees its body to the client.
    /// `head` overrides the recorded head (the upstream's own, when nothing
    /// changed it).
    fn reply(&self, response: Response<RuleBody>, head: Option<pm_model::ResponseHead>) -> Response<ProxyBody> {
        if let Some(rec) = &self.recorder {
            let head = head.unwrap_or_else(|| response_head(&response));
            rec.response(head, content_kind(response.headers()));
        }
        let (parts, body) = response.into_parts();
        let body = TeeBody::new(body, self.recorder.clone(), BodyPart::Response);
        Response::from_parts(parts, body.map_err(BoxError::from).boxed_unsync())
    }

    fn block(&self, mode: pm_model::BlockMode, held: Held) -> Reply {
        match mode {
            pm_model::BlockMode::Status { status } => {
                held.drain(self.recorder.clone(), BodyPart::Request);
                if let Some(rec) = &self.recorder {
                    rec.blocked();
                }
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                let answer = ResponseDraft {
                    status,
                    headers,
                    body: Some(b"Blocked by a Proxymate rule.\n".to_vec()),
                };
                Ok(self.finish_response(answer, None, true, None, None))
            }
            pm_model::BlockMode::Reset => {
                held.drain(None, BodyPart::Request);
                if let Some(rec) = &self.recorder {
                    rec.reset("Blocked by a rule: the connection was reset.", true);
                }
                Err(Reset::new("blocked by a rule"))
            }
        }
    }

    /// Reads a body ahead (up to the cap) and decodes it.
    async fn hold(
        &self,
        body: Inbound,
        headers: &HeaderMap,
        part: BodyPart,
    ) -> Result<(Held, Option<Vec<u8>>), hyper::Error> {
        let which = match part {
            BodyPart::Request => "request",
            BodyPart::Response => "response",
        };
        match buffer(body, REWRITE_BODY_CAP).await? {
            Buffered::Over(prefix, rest) => {
                if let Some(rec) = &self.recorder {
                    rec.rewrite_skipped(format!(
                        "The {which} body is larger than {} MB, so it was forwarded unchanged.",
                        REWRITE_BODY_CAP / 1024 / 1024
                    ));
                }
                Ok((Held::Over(prefix, rest), None))
            }
            Buffered::Whole(bytes) => {
                let encoding = headers
                    .get(header::CONTENT_ENCODING)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.trim().to_ascii_lowercase())
                    .filter(|v| !v.is_empty() && v != "identity");
                match encoding {
                    None => {
                        let decoded = bytes.to_vec();
                        Ok((Held::Decoded(bytes), Some(decoded)))
                    }
                    Some(encoding) => match decode::decode(&encoding, &bytes) {
                        Ok(decoded) => Ok((Held::Decoded(bytes), Some(decoded))),
                        Err(err) => {
                            if let Some(rec) = &self.recorder {
                                rec.rewrite_skipped(format!(
                                    "The {which} body couldn't be decoded ({err}), so it was forwarded unchanged."
                                ));
                            }
                            Ok((Held::Raw(bytes), None))
                        }
                    },
                }
            }
        }
    }

    /// Parks the flow at a breakpoint.
    async fn pause(&self, engine: &Arc<Engine>, info: BreakpointInfo, body: Vec<u8>) -> Outcome {
        if let Some(rec) = &self.recorder {
            rec.paused(true);
        }
        // Dropped while paused: the client went away.
        let mut gone = ClientGone(self.recorder.clone());
        let outcome = engine.breakpoint_manager().pause(info, body).await;
        gone.0 = None;
        if let Some(rec) = &self.recorder {
            rec.paused(false);
            if matches!(outcome, Outcome::TimedOut) {
                rec.breakpoint_timeout();
            }
        }
        outcome
    }

    /// Sends the request to `target`: over the route's own connection when
    /// it is the original origin, else directly.
    async fn send(
        &self,
        mut parts: request::Parts,
        pq: PathAndQuery,
        target: &Target,
        body: super::UpstreamBody,
    ) -> Result<Response<Incoming>, String> {
        let original = target.scheme == self.scheme && target.host == self.host && target.port == self.port;
        let authority: Authority = target
            .authority()
            .parse()
            .map_err(|_| format!("{:?} isn't a valid authority", target.authority()))?;
        if let (Route::Mitm(conn), true) = (&self.route, original) {
            return conn.send(parts, pq, authority, body, self.recorder.as_ref()).await;
        }
        if target.scheme.is_tls() {
            return mitm::send_direct(
                self.ctx,
                &target.host,
                target.port,
                parts,
                pq,
                authority,
                body,
                self.recorder.as_ref(),
            )
            .await;
        }
        parts.uri = target
            .url(pq.as_str())
            .parse()
            .map_err(|e| format!("bad upstream URL: {e}"))?;
        // The pooled client speaks HTTP/1.1, whatever the client spoke.
        parts.version = Version::HTTP_11;
        if !original
            && !parts.headers.contains_key(header::HOST)
            && let Ok(host) = HeaderValue::from_str(authority.as_str())
        {
            parts.headers.insert(header::HOST, host);
        }
        if let Some(rec) = &self.recorder {
            rec.request_sent();
        }
        self.ctx
            .client
            .request(Request::from_parts(parts, body))
            .await
            .map_err(|e| error_chain(&e))
    }

    /// Fails the flow with an upstream error and answers 502.
    fn fail(&self, message: String) -> Response<ProxyBody> {
        if let Some(rec) = &self.recorder {
            rec.fail(message.clone());
        }
        match &self.route {
            Route::Forward => text_response(
                StatusCode::BAD_GATEWAY,
                &format!("Proxymate: upstream error: {message}\n"),
            ),
            Route::Mitm(conn) => mitm::error_page(&conn.target.host, &message),
        }
    }

    fn draft(&self, parts: &request::Parts) -> RequestDraft {
        let path = parts.uri.path();
        RequestDraft {
            method: parts.method.to_string(),
            scheme: self.scheme,
            host: self.host.clone(),
            port: self.port,
            path: if path.is_empty() { "/".into() } else { path.to_owned() },
            query: parts.uri.query().unwrap_or_default().to_owned(),
            headers: parts.headers.clone(),
            body: None,
        }
    }

    fn request_head(&self, draft: &RequestDraft, version: Version) -> RequestHead {
        RequestHead {
            method: draft.method.clone(),
            url: draft.url(),
            version: super::version(version),
            headers: headers_of(&draft.headers),
        }
    }
}

/// Fails a paused flow whose exchange was dropped (the client closed the
/// connection or reset the stream).
struct ClientGone(Option<FlowRecorder>);

impl Drop for ClientGone {
    fn drop(&mut self) {
        if let Some(rec) = &self.0 {
            rec.fail("The client went away while the flow was paused at a breakpoint.");
        }
    }
}

/// Applies a breakpoint's edits to a request; true if anything changed.
fn edit_request(draft: &mut RequestDraft, edits: MessageEdits, editable: bool) -> bool {
    let mut changed = false;
    if let Some(method) = edits.method.map(|m| m.trim().to_ascii_uppercase())
        && !method.is_empty()
        && http::Method::from_bytes(method.as_bytes()).is_ok()
        && method != draft.method
    {
        draft.method = method;
        changed = true;
    }
    if let Some(url) = edits.url
        && url != draft.url()
        && let Ok(uri) = url.trim().parse::<http::Uri>()
        && let (Some(scheme), Some(host)) = (uri.scheme_str(), uri.host())
    {
        let scheme = if scheme.eq_ignore_ascii_case("https") {
            Scheme::Https
        } else {
            Scheme::Http
        };
        let old_authority = draft.authority();
        draft.scheme = scheme;
        draft.host = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        draft.port = uri.port_u16().unwrap_or(if scheme.is_tls() { 443 } else { 80 });
        draft.path = match uri.path() {
            "" => "/".into(),
            path => path.to_owned(),
        };
        draft.query = uri.query().unwrap_or_default().to_owned();
        if draft.authority() != old_authority
            && draft.headers.contains_key(header::HOST)
            && let Ok(host) = HeaderValue::from_str(&draft.authority())
        {
            draft.headers.insert(header::HOST, host);
        }
        changed = true;
    }
    if let Some(headers) = edits.headers {
        draft.headers = header_map(&headers);
        changed = true;
    }
    if let Some(body) = edits.body
        && editable
    {
        draft.body = Some(body.into_bytes());
        changed = true;
    }
    changed
}

fn edit_response(draft: &mut ResponseDraft, edits: MessageEdits, editable: bool) -> bool {
    let mut changed = false;
    if let Some(status) = edits.status
        && (100..=999).contains(&status)
    {
        draft.status = status;
        changed = true;
    }
    if let Some(headers) = edits.headers {
        draft.headers = header_map(&headers);
        changed = true;
    }
    if let Some(body) = edits.body
        && editable
    {
        draft.body = Some(body.into_bytes());
        changed = true;
    }
    changed
}

/// An origin a request is sent to.
struct Target {
    scheme: Scheme,
    host: String,
    port: u16,
}

impl Target {
    fn new(scheme: Scheme, host: &str, port: u16) -> Target {
        Target {
            scheme,
            host: host.to_owned(),
            port,
        }
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default = if self.scheme.is_tls() { 443 } else { 80 };
        if self.port == default {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    fn url(&self, pq: &str) -> String {
        let scheme = if self.scheme.is_tls() { "https" } else { "http" };
        format!("{scheme}://{}{pq}", self.authority())
    }
}
