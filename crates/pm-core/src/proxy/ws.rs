//! WebSocket relay, for `ws://` (absolute-form or inside a CONNECT) and for
//! `wss://` inside an intercepted TLS connection.
//!
//! The handshake goes upstream like any request, minus
//! `Sec-WebSocket-Extensions` so frames stay uncompressed and readable. On
//! `101` both connections are taken over and wrapped as WebSockets, the
//! client's in the server role and the upstream's in the client role, and
//! every message is relayed and recorded.
//!
//! Messages are reassembled and re-framed, so fragmentation on the wire may
//! differ from what the peer sent. Pings are forwarded, and tungstenite also
//! answers them itself, so the pinging side may see two pongs (unsolicited
//! pongs are allowed by RFC 6455).

use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;
use parking_lot::Mutex;
use pm_model::{WsClose, WsDirection, WsOpcode};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::{ProxyBody, strip_hop_by_hop};
use crate::recorder::{FlowRecorder, WsRecorder};

/// Once one side has closed, the other gets this long to finish closing.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

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

/// Answers the client's handshake with the upstream's `101` and relays the
/// two connections once both have switched protocols.
pub(super) fn switch(
    recorder: Option<FlowRecorder>,
    client: OnUpgrade,
    mut upstream: Response<hyper::body::Incoming>,
) -> Response<ProxyBody> {
    debug_assert_eq!(upstream.status(), StatusCode::SWITCHING_PROTOCOLS);
    let ws = recorder.as_ref().map(FlowRecorder::websocket);
    let upstream_io = hyper::upgrade::on(&mut upstream);
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed_unsync());
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *response.headers_mut() = upstream.headers().clone();
    tokio::spawn(async move {
        match tokio::try_join!(client, upstream_io) {
            Ok((client, upstream)) => relay(ws, client, upstream).await,
            Err(err) => {
                if let Some(ws) = ws {
                    ws.closed(None, Some(format!("switching to WebSocket failed: {err}")));
                }
            }
        }
    });
    response
}

async fn relay(ws: Option<WsRecorder>, client: Upgraded, upstream: Upgraded) {
    let client = WebSocketStream::from_raw_socket(TokioIo::new(client), Role::Server, None).await;
    let upstream = WebSocketStream::from_raw_socket(TokioIo::new(upstream), Role::Client, None).await;
    let (client_tx, client_rx) = client.split();
    let (upstream_tx, upstream_rx) = upstream.split();
    let first_close = Mutex::new(None);

    // Both directions run concurrently, so a peer that is slow to read
    // never stalls the other direction.
    let up = pump(client_rx, upstream_tx, WsDirection::Sent, ws.as_ref(), &first_close);
    let down = pump(upstream_rx, client_tx, WsDirection::Received, ws.as_ref(), &first_close);
    tokio::pin!(up, down);
    let (first, second) = tokio::select! {
        end = &mut up => (end, tokio::time::timeout(CLOSE_GRACE, down).await.ok()),
        end = &mut down => (end, tokio::time::timeout(CLOSE_GRACE, up).await.ok()),
    };

    if let Some(ws) = &ws {
        let error = first.error.or_else(|| second.and_then(|end| end.error));
        ws.closed(first_close.lock().take(), error);
    }
}

struct PumpEnd {
    error: Option<String>,
}

/// Relays one direction until its reading side ends, then closes the
/// writing side (which sends a close frame unless one went already).
async fn pump(
    mut rx: SplitStream<Ws>,
    mut tx: SplitSink<Ws, Message>,
    direction: WsDirection,
    ws: Option<&WsRecorder>,
    first_close: &Mutex<Option<WsClose>>,
) -> PumpEnd {
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
            first_close.lock().get_or_insert_with(|| WsClose {
                code: frame.as_ref().map_or(NO_STATUS, |f| u16::from(f.code)),
                reason: frame.as_ref().map(|f| f.reason.to_string()).unwrap_or_default(),
                by: direction,
            });
        }
        if let Some(ws) = ws {
            record(ws, direction, &message);
        }
        // The other side may be closing too; its own pump reports that.
        let _ = tx.send(message).await;
    }
    // Ended without a close frame from either side so far: this side
    // dropped (or broke) the connection first.
    first_close.lock().get_or_insert_with(|| WsClose {
        code: ABNORMAL,
        reason: error
            .clone()
            .unwrap_or_else(|| format!("the {} closed the connection without a close frame", side(direction))),
        by: direction,
    });
    let _ = tx.close().await;
    PumpEnd { error }
}

fn record(ws: &WsRecorder, direction: WsDirection, message: &Message) {
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
    ws.message(direction, opcode, payload, code);
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
}
