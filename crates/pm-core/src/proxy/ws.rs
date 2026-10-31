//! WebSocket relay, for `ws://` (absolute-form or inside a CONNECT) and for
//! `wss://` inside an intercepted TLS connection.
//!
//! The handshake goes through `exchange.rs` like any request (rules apply
//! to it), minus `Sec-WebSocket-Extensions` so frames stay uncompressed and
//! readable. On `101` both connections are taken over and wrapped as
//! WebSockets, the client's in the server role and the upstream's in the
//! client role, and every message is relayed and recorded.
//!
//! Each direction has a reader, which reads frames as they come (so pings
//! are answered and a peer is never stalled by the other side), and a
//! processor, which takes them from a queue in order, applies the message
//! rules (`rules::eval::ws_message_phase`), and writes them on. A message
//! held by a delay or a breakpoint holds the ones behind it in its
//! direction; the other direction keeps flowing. Messages Proxymate sends
//! itself (`Engine::ws_send`) join the same queue, so they keep their place
//! too.
//!
//! The rules are those whose request matcher matches the handshake as the
//! client sent it. They are matched again whenever the rule set changes, so
//! a rule added while a connection is open applies to its next message.
//!
//! A replayed handshake (`Engine::send`) has no client: received messages
//! are recorded and go nowhere, and only injected messages are sent.
//!
//! Messages are reassembled and re-framed, so fragmentation on the wire may
//! differ from what the peer sent. Pings are forwarded, and tungstenite also
//! answers them itself, so the pinging side may see two pongs (unsolicited
//! pongs are allowed by RFC 6455).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;
use parking_lot::Mutex;
use pm_model::{
    BreakpointBody, BreakpointInfo, BreakpointResolution, FlowId, HeldMessage, MessageEdits, RequestHead, WsClose,
    WsDirection, WsOpcode,
};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};
use tokio_tungstenite::tungstenite::{Error as WsError, Message, Utf8Bytes};

use super::{ProxyBody, now_ms, strip_hop_by_hop};
use crate::Engine;
use crate::client::PeerClient;
use crate::recorder::{FlowRecorder, WsRecorder};
use crate::rules::compile::{CompiledRule, CompiledRuleSet};
use crate::rules::eval::{self, RequestDraft, WsDraft};
use crate::rules::{BREAKPOINT_EDIT_CAP, Outcome};
use crate::scripts::ScriptRun;
use crate::store::WsEntry;

/// Once one side has closed, the other gets this long to finish closing.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// Messages read ahead per direction while the processor is busy (a
/// held message). Past this the reader waits, which pushes back on the
/// peer.
const QUEUE: usize = 1024;
/// Script problems recorded per connection; one script failing on every
/// message would otherwise grow the flow without bound.
const ISSUES_MAX: u32 = 20;

/// No close code in the close frame (RFC 6455 §7.1.5).
const NO_STATUS: u16 = 1005;
/// The connection dropped without a close frame.
const ABNORMAL: u16 = 1006;

type Ws = WebSocketStream<TokioIo<Upgraded>>;

/// True for an HTTP/1.1 WebSocket handshake request.
pub(super) fn is_upgrade<B>(req: &Request<B>) -> bool {
    req.method() == Method::GET
        && has_token(req.headers(), header::UPGRADE, "websocket")
        && has_token(req.headers(), header::CONNECTION, "upgrade")
}

fn has_token(headers: &HeaderMap, name: HeaderName, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case(token))
}

/// Headers for the upstream handshake: hop-by-hop headers go as for any
/// request, except the two the handshake needs, and extensions go too.
pub(super) fn prepare_request(headers: &mut HeaderMap) {
    let upgrade = headers.get(header::UPGRADE).cloned();
    strip_hop_by_hop(headers);
    headers.remove(header::SEC_WEBSOCKET_EXTENSIONS);
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    if let Some(upgrade) = upgrade {
        headers.insert(header::UPGRADE, upgrade);
    }
}

/// The client end of a handshake.
pub(super) enum ClientSide {
    /// A client connection, taken over once the upstream answers `101`.
    Upgrade(OnUpgrade),
    /// No client: a handshake Proxymate sent itself (a replay).
    Headless,
}

/// What a WebSocket handshake carries through the exchange to the relay.
pub(super) struct Upgrade {
    pub client: ClientSide,
    /// `None` when rules are off (`Engine::send` without them).
    pub rules: Option<MessageRules>,
}

/// Matches the message rules for one connection.
pub(super) struct MessageRules {
    engine: Weak<Engine>,
    /// The handshake as the client sent it.
    handshake: RequestDraft,
    peer: Arc<PeerClient>,
    cache: Mutex<Option<Matched>>,
}

/// A rule set, and the rules of it that act on one connection's messages.
type Matched = (Arc<CompiledRuleSet>, Vec<Arc<CompiledRule>>);

impl MessageRules {
    pub fn new(engine: Weak<Engine>, handshake: RequestDraft, peer: Arc<PeerClient>) -> MessageRules {
        MessageRules {
            engine,
            handshake,
            peer,
            cache: Mutex::new(None),
        }
    }

    /// The rules for the next message, matched again if the rule set
    /// changed since the last one.
    async fn current(&self, engine: &Engine) -> Vec<Arc<CompiledRule>> {
        let set = engine.rule_set();
        if let Some((cached, rules)) = &*self.cache.lock()
            && Arc::ptr_eq(cached, &set)
        {
            return rules.clone();
        }
        let app = match set.needs_client_app() {
            true => self.peer.app(Duration::ZERO).await.map(|a| a.name.clone()),
            false => None,
        };
        let rules: Vec<Arc<CompiledRule>> = set
            .matching(&self.handshake.info(app.as_deref()))
            .into_iter()
            .filter(|r| r.acts_on_messages())
            .collect();
        *self.cache.lock() = Some((set, rules.clone()));
        rules
    }
}

/// Open WebSocket connections by flow, for sending into them
/// (`Engine::ws_send`).
#[derive(Default)]
pub(crate) struct Registry(DashMap<FlowId, Handle>);

#[derive(Clone)]
struct Handle {
    to_server: mpsc::Sender<Item>,
    /// `None` for a replay, which has no client.
    to_client: Option<mpsc::Sender<Item>>,
}

impl Registry {
    /// Queues `message` in `direction`: `Sent` goes to the server, as if
    /// the client had sent it. It goes out after anything held ahead of it.
    pub fn send(&self, flow: FlowId, direction: WsDirection, message: Message) -> Result<(), String> {
        let handle = self
            .0
            .get(&flow)
            .map(|h| h.clone())
            .ok_or_else(|| format!("flow {flow} isn't an open WebSocket"))?;
        let queue = match direction {
            WsDirection::Sent => Some(handle.to_server),
            WsDirection::Received => handle.to_client,
        }
        .ok_or("this connection has no client (a replay): messages can only be sent to the server")?;
        queue.try_send(Item::Inject(message)).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => format!("{QUEUE} messages are already waiting in that direction"),
            mpsc::error::TrySendError::Closed(_) => "that side of the connection has closed".to_owned(),
        })
    }

    /// Closes the connection: a close frame to each side.
    pub fn close(&self, flow: FlowId) -> Result<(), String> {
        let frame = || {
            Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: Utf8Bytes::from_static("Closed in Proxymate"),
            }))
        };
        let handle = self
            .0
            .get(&flow)
            .map(|h| h.clone())
            .ok_or_else(|| format!("flow {flow} isn't an open WebSocket"))?;
        let _ = handle.to_server.try_send(Item::Inject(frame()));
        if let Some(client) = handle.to_client {
            let _ = client.try_send(Item::Inject(frame()));
        }
        Ok(())
    }

    pub fn is_open(&self, flow: FlowId) -> bool {
        self.0.contains_key(&flow)
    }
}

/// One entry of a direction's queue.
enum Item {
    /// A frame read from the peer.
    Wire(Message),
    /// A frame Proxymate sends itself.
    Inject(Message),
    /// The peer stopped sending, with why if it broke.
    End(Option<String>),
}

/// Answers the handshake with a `101` carrying `headers` (the upstream's,
/// after rules) and relays the two connections once both have switched
/// protocols.
pub(super) fn switch(
    recorder: Option<FlowRecorder>,
    upgrade: Upgrade,
    upstream: OnUpgrade,
    mut headers: HeaderMap,
) -> Response<ProxyBody> {
    let ws = recorder.as_ref().map(FlowRecorder::websocket);
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed_unsync());
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    // Rules see the head without hop-by-hop headers; the switch needs them.
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    if !headers.contains_key(header::UPGRADE) {
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    }
    *response.headers_mut() = headers;
    let Upgrade { client, rules } = upgrade;
    tokio::spawn(async move {
        let client = async {
            match client {
                ClientSide::Upgrade(on) => on.await.map(Some),
                ClientSide::Headless => Ok(None),
            }
        };
        match tokio::try_join!(client, upstream) {
            Ok((client, upstream)) => relay(ws, rules, client, upstream).await,
            Err(err) => {
                if let Some(ws) = ws {
                    ws.closed(None, Some(format!("switching to WebSocket failed: {err}")));
                }
            }
        }
    });
    response
}

/// What both directions of one connection share.
struct Conn {
    ws: Option<WsRecorder>,
    rules: Option<MessageRules>,
    first_close: Mutex<Option<WsClose>>,
    issues: AtomicU32,
    /// The handshake as sent, for message breakpoints.
    handshake: Option<RequestHead>,
}

async fn relay(ws: Option<WsRecorder>, rules: Option<MessageRules>, client: Option<Upgraded>, upstream: Upgraded) {
    let client = match client {
        Some(io) => Some(WebSocketStream::from_raw_socket(TokioIo::new(io), Role::Server, None).await),
        None => None,
    };
    let headless = client.is_none();
    let upstream = WebSocketStream::from_raw_socket(TokioIo::new(upstream), Role::Client, None).await;
    let (client_tx, client_rx) = match client.map(StreamExt::split) {
        Some((tx, rx)) => (Some(tx), Some(rx)),
        None => (None, None),
    };
    let (upstream_tx, upstream_rx) = upstream.split();
    let conn = Arc::new(Conn {
        handshake: ws.as_ref().and_then(|w| w.flow().request_snapshot()),
        ws,
        rules,
        first_close: Mutex::new(None),
        issues: AtomicU32::new(0),
    });

    let (to_server, server_queue) = mpsc::channel(QUEUE);
    let (to_client, client_queue) = mpsc::channel(QUEUE);
    let flow = conn.ws.as_ref().map(|w| w.flow().id());
    let registry = conn
        .ws
        .as_ref()
        .and_then(|w| w.flow().engine())
        .filter(|_| flow.is_some());
    if let (Some(engine), Some(flow)) = (&registry, flow) {
        engine.websockets.0.insert(
            flow,
            Handle {
                to_server: to_server.clone(),
                to_client: (!headless).then(|| to_client.clone()),
            },
        );
    }

    // Readers run on their own, so a peer that is slow to read never
    // stalls reading from the other.
    let mut readers = vec![tokio::spawn(read(
        upstream_rx,
        to_client,
        WsDirection::Received,
        conn.clone(),
    ))];
    if let Some(rx) = client_rx {
        readers.push(tokio::spawn(read(rx, to_server, WsDirection::Sent, conn.clone())));
    }

    let up = process(&conn, WsDirection::Sent, server_queue, Some(upstream_tx));
    let down = process(&conn, WsDirection::Received, client_queue, client_tx);
    tokio::pin!(up, down);
    // A replay has no client to finish closing.
    let grace = if headless { Duration::ZERO } else { CLOSE_GRACE };
    let (first, second) = tokio::select! {
        end = &mut up => (end, tokio::time::timeout(grace, down).await.ok()),
        end = &mut down => (end, tokio::time::timeout(grace, up).await.ok()),
    };

    // A side that never finished closing is cut off.
    for reader in readers {
        reader.abort();
    }
    if let (Some(engine), Some(flow)) = (&registry, flow) {
        engine.websockets.0.remove(&flow);
    }
    if let Some(ws) = &conn.ws {
        let error = first.error.or_else(|| second.and_then(|end| end.error));
        ws.closed(conn.first_close.lock().take(), error);
    }
}

/// Reads one direction until it ends, queueing every frame.
async fn read(mut rx: SplitStream<Ws>, queue: mpsc::Sender<Item>, direction: WsDirection, conn: Arc<Conn>) {
    let mut error = None;
    while let Some(item) = rx.next().await {
        let message = match item {
            Ok(message) => message,
            Err(err) => {
                error = describe(&err, direction);
                break;
            }
        };
        if let Message::Close(frame) = &message {
            conn.first_close.lock().get_or_insert_with(|| WsClose {
                code: frame.as_ref().map_or(NO_STATUS, |f| u16::from(f.code)),
                reason: frame.as_ref().map(|f| f.reason.to_string()).unwrap_or_default(),
                by: direction,
            });
        }
        if queue.send(Item::Wire(message)).await.is_err() {
            return;
        }
    }
    // Ended without a close frame from either side so far: this side
    // dropped (or broke) the connection first.
    conn.first_close.lock().get_or_insert_with(|| WsClose {
        code: ABNORMAL,
        reason: error
            .clone()
            .unwrap_or_else(|| format!("the {} closed the connection without a close frame", side(direction))),
        by: direction,
    });
    let _ = queue.send(Item::End(error)).await;
}

struct PumpEnd {
    error: Option<String>,
}

/// Relays one direction's queue in order until its reader ends, then
/// closes the writing side (which sends a close frame unless one went
/// already).
async fn process(
    conn: &Conn,
    direction: WsDirection,
    mut queue: mpsc::Receiver<Item>,
    mut tx: Option<SplitSink<Ws, Message>>,
) -> PumpEnd {
    let mut error = None;
    while let Some(item) = queue.recv().await {
        match item {
            Item::End(why) => {
                error = why;
                break;
            }
            Item::Inject(message) => {
                // Closing from Proxymate counts as the side it speaks for.
                if let Message::Close(frame) = &message {
                    conn.first_close.lock().get_or_insert_with(|| WsClose {
                        code: frame.as_ref().map_or(NO_STATUS, |f| u16::from(f.code)),
                        reason: frame.as_ref().map(|f| f.reason.to_string()).unwrap_or_default(),
                        by: direction,
                    });
                }
                conn.record(direction, &message, Flags::INJECTED, None);
                write(&mut tx, message).await;
            }
            Item::Wire(message) => match message {
                Message::Text(_) | Message::Binary(_) => {
                    if let Some(message) = conn.data(direction, message).await {
                        write(&mut tx, message).await;
                    }
                }
                Message::Frame(_) => {}
                other => {
                    conn.record(direction, &other, Flags::NONE, None);
                    write(&mut tx, other).await;
                }
            },
        }
    }
    if let Some(tx) = tx.as_mut() {
        let _ = tx.close().await;
    }
    PumpEnd { error }
}

async fn write(tx: &mut Option<SplitSink<Ws, Message>>, message: Message) {
    // The other side may be closing too; its own pump reports that.
    if let Some(tx) = tx.as_mut() {
        let _ = tx.send(message).await;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Flags {
    injected: bool,
    dropped: bool,
}

impl Flags {
    const NONE: Flags = Flags {
        injected: false,
        dropped: false,
    };
    const INJECTED: Flags = Flags {
        injected: true,
        dropped: false,
    };
    const DROPPED: Flags = Flags {
        injected: false,
        dropped: true,
    };
}

impl Conn {
    /// Runs the message rules on a text or binary message; `None` if it
    /// is dropped.
    async fn data(&self, direction: WsDirection, message: Message) -> Option<Message> {
        let engine = self.rules.as_ref().and_then(|r| r.engine.upgrade());
        let (Some(rules), Some(engine)) = (&self.rules, engine) else {
            self.record(direction, &message, Flags::NONE, None);
            return Some(message);
        };
        let matched = rules.current(&engine).await;
        if matched.is_empty() {
            self.record(direction, &message, Flags::NONE, None);
            return Some(message);
        }
        let (text, original) = match &message {
            Message::Text(t) => (true, Bytes::from(t.clone())),
            Message::Binary(b) => (false, b.clone()),
            _ => unreachable!("only data messages reach the rules"),
        };
        let mut draft = WsDraft {
            direction,
            text,
            payload: original.to_vec(),
        };
        let flow = self.ws.as_ref().map(|w| w.flow().id());
        let run = ScriptRun::new(engine.scripts_state(), flow);
        let mut fx = eval::ws_message_phase(&matched, &mut draft, run).await;
        if let Some(rec) = self.ws.as_ref().map(WsRecorder::flow) {
            rec.rules_applied(std::mem::take(&mut fx.applied));
            for reason in fx.skipped.drain(..) {
                rec.rewrite_skipped(reason);
            }
            for issue in fx.script_issues.drain(..) {
                if self.issues.fetch_add(1, Ordering::Relaxed) < ISSUES_MAX {
                    rec.script_issue(issue);
                }
            }
        }
        if fx.drop {
            self.record(direction, &message, Flags::DROPPED, None);
            return None;
        }
        if fx.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(fx.delay_ms)).await;
        }
        if let Some(rule) = fx.breakpoint.take() {
            match self.pause(&engine, &rule, &draft).await {
                Outcome::TimedOut => {}
                Outcome::Resolved(BreakpointResolution::Continue { edits: Some(edits) }) => edit(&mut draft, edits),
                Outcome::Resolved(BreakpointResolution::Continue { edits: None }) => {}
                // Refused for messages by `Engine::resolve_breakpoint`.
                Outcome::Resolved(BreakpointResolution::Respond { .. }) => {}
                Outcome::Resolved(BreakpointResolution::Abort) => {
                    self.record(direction, &message, Flags::DROPPED, None);
                    return None;
                }
            }
        }
        let changed = draft.text != text || draft.payload != original;
        if !changed {
            self.record(direction, &message, Flags::NONE, None);
            return Some(message);
        }
        let out = into_message(draft);
        self.record(direction, &out, Flags::NONE, Some(&original));
        Some(out)
    }

    /// Holds a message at a breakpoint until it is resolved or times out.
    async fn pause(&self, engine: &Engine, rule: &CompiledRule, draft: &WsDraft) -> Outcome {
        let rec = self.ws.as_ref().map(WsRecorder::flow);
        let size = draft.payload.len();
        let editable = size <= BREAKPOINT_EDIT_CAP;
        let info = BreakpointInfo {
            id: 0,
            flow_id: rec.map(FlowRecorder::id),
            phase: draft.phase(),
            rule_id: rule.applied.rule_id.clone(),
            rule_name: rule.applied.name.clone(),
            paused_at_ms: now_ms(),
            deadline_ms: None,
            request: self.handshake.clone().unwrap_or_else(|| RequestHead {
                method: "GET".into(),
                url: self.rules.as_ref().map(|r| r.handshake.url()).unwrap_or_default(),
                version: pm_model::HttpVersion::Http11,
                headers: Vec::new(),
            }),
            response: None,
            body: BreakpointBody {
                size: size as u64,
                editable,
                note: (!editable).then(|| {
                    format!(
                        "The message is larger than {} MB, so it can only be sent on or dropped.",
                        BREAKPOINT_EDIT_CAP / 1024 / 1024
                    )
                }),
            },
            message: Some(HeldMessage {
                direction: draft.direction,
                opcode: if draft.text { WsOpcode::Text } else { WsOpcode::Binary },
            }),
        };
        if let Some(rec) = rec {
            rec.paused(true);
        }
        let outcome = engine.breakpoint_manager().pause(info, draft.payload.clone()).await;
        if let Some(rec) = rec {
            rec.paused(false);
            if matches!(outcome, Outcome::TimedOut) {
                rec.breakpoint_timeout();
            }
        }
        outcome
    }

    /// Records a message as it is forwarded (or dropped), so the log's
    /// order is the order messages went out, whatever was held.
    fn record(&self, direction: WsDirection, message: &Message, flags: Flags, original: Option<&[u8]>) {
        let Some(ws) = &self.ws else {
            return;
        };
        let (opcode, payload, code): (WsOpcode, &[u8], Option<u16>) = match message {
            Message::Text(text) => (WsOpcode::Text, text.as_bytes(), None),
            Message::Binary(data) => (WsOpcode::Binary, data, None),
            Message::Ping(data) => (WsOpcode::Ping, data, None),
            Message::Pong(data) => (WsOpcode::Pong, data, None),
            Message::Close(Some(CloseFrame { code, reason })) => {
                (WsOpcode::Close, reason.as_bytes(), Some(u16::from(*code)))
            }
            Message::Close(None) => (WsOpcode::Close, &[], None),
            // Only produced when writing raw frames, never by reading.
            Message::Frame(_) => return,
        };
        let entry = WsEntry {
            direction,
            opcode,
            payload,
            close_code: code,
            original,
            injected: flags.injected,
            dropped: flags.dropped,
        };
        ws.record(entry, now_ms());
    }
}

/// Applies a breakpoint's edits to a held message.
fn edit(draft: &mut WsDraft, edits: MessageEdits) {
    use base64::Engine as _;
    if let Some(data) = edits.body_base64 {
        // Checked by `Engine::resolve_breakpoint`.
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.trim()) {
            draft.payload = bytes;
        }
    } else if let Some(body) = edits.body {
        draft.payload = body.into_bytes();
    }
}

/// A data message from a draft; text that isn't UTF-8 goes as binary.
fn into_message(draft: WsDraft) -> Message {
    if draft.text {
        match String::from_utf8(draft.payload) {
            Ok(text) => Message::Text(text.into()),
            Err(err) => Message::Binary(Bytes::from(err.into_bytes())),
        }
    } else {
        Message::Binary(Bytes::from(draft.payload))
    }
}

/// A data message from bytes; text that isn't UTF-8 is refused.
pub(crate) fn message_of(payload: Vec<u8>, binary: bool) -> Result<Message, String> {
    if binary {
        return Ok(Message::Binary(Bytes::from(payload)));
    }
    String::from_utf8(payload)
        .map(|text| Message::Text(text.into()))
        .map_err(|_| "a text message must be UTF-8".to_owned())
}

/// A reading error worth reporting, or `None` for the ways a connection
/// normally ends (a completed close, or a peer that just went away).
fn describe(err: &WsError, direction: WsDirection) -> Option<String> {
    match err {
        WsError::ConnectionClosed | WsError::AlreadyClosed => None,
        WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => None,
        WsError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            None
        }
        other => Some(format!("WebSocket error from the {}: {other}", side(direction))),
    }
}

/// Who sends in `direction`.
fn side(direction: WsDirection) -> &'static str {
    match direction {
        WsDirection::Sent => "client",
        WsDirection::Received => "server",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(headers: &[(&str, &str)]) -> Request<()> {
        let mut builder = Request::get("http://example.com/socket");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).unwrap()
    }

    #[test]
    fn detects_websocket_handshakes() {
        assert!(is_upgrade(&request(&[
            ("upgrade", "WebSocket"),
            ("connection", "keep-alive, Upgrade")
        ])));
        assert!(!is_upgrade(&request(&[("upgrade", "websocket")])));
        assert!(!is_upgrade(&request(&[("upgrade", "h2c"), ("connection", "upgrade")])));
    }

    #[test]
    fn handshake_headers_keep_upgrade_and_drop_extensions() {
        let mut req = request(&[
            ("upgrade", "websocket"),
            ("connection", "Upgrade"),
            ("proxy-connection", "keep-alive"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
            ("sec-websocket-extensions", "permessage-deflate; client_max_window_bits"),
        ]);
        prepare_request(req.headers_mut());
        let headers = req.headers();
        assert_eq!(headers[header::UPGRADE], "websocket");
        assert_eq!(headers[header::CONNECTION], "upgrade");
        assert!(headers.contains_key(header::SEC_WEBSOCKET_KEY));
        assert!(!headers.contains_key(header::SEC_WEBSOCKET_EXTENSIONS));
        assert!(!headers.contains_key("proxy-connection"));
    }

    #[test]
    fn edited_text_that_isnt_utf8_goes_as_binary() {
        let draft = |payload: &[u8]| WsDraft {
            direction: WsDirection::Sent,
            text: true,
            payload: payload.to_vec(),
        };
        assert_eq!(into_message(draft(b"ok")), Message::text("ok"));
        assert_eq!(into_message(draft(&[0xff])), Message::binary(vec![0xffu8]));
        let mut held = draft(b"old");
        edit(
            &mut held,
            MessageEdits {
                body: Some("new".into()),
                ..Default::default()
            },
        );
        assert_eq!(held.payload, b"new");
        edit(
            &mut held,
            MessageEdits {
                body: Some("ignored".into()),
                body_base64: Some("AAE=".into()),
                ..Default::default()
            },
        );
        assert_eq!(held.payload, [0, 1]);
        assert!(message_of(vec![0xff], false).is_err());
    }
}
