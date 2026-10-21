//! Streaming routes shared by the test upstreams: a WebSocket echo server
//! (`/ws`) and a slow server-sent events stream (`/sse`).

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http::{HeaderValue, Request, Response, StatusCode, header};
use http_body::{Body, Frame};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::time::Sleep;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};

use crate::upstream::TestBody;

/// Text the echo server answers by closing with [`SERVER_CLOSE_CODE`].
pub const CLOSE_COMMAND: &str = "close";
pub const SERVER_CLOSE_CODE: u16 = 4000;
pub const SERVER_CLOSE_REASON: &str = "bye";
/// Text the echo server answers by sending a ping with [`SERVER_PING`].
pub const PING_COMMAND: &str = "ping";
pub const SERVER_PING: &[u8] = b"server ping";
/// Response header with the `Sec-WebSocket-Extensions` the echo server was
/// offered (`none` if absent), so tests can check the proxy stripped it.
pub const SEEN_EXTENSIONS: &str = "x-seen-extensions";

/// `/ws`: accepts the handshake and echoes text and binary messages.
/// Pings are answered automatically; [`CLOSE_COMMAND`] and
/// [`PING_COMMAND`] make the server close or ping.
pub(crate) fn websocket(mut req: Request<Incoming>) -> Response<TestBody> {
    let key = req
        .headers()
        .get(header::SEC_WEBSOCKET_KEY)
        .map(|k| derive_accept_key(k.as_bytes()));
    let Some(accept) = key else {
        let mut response = Response::new(full("expected a WebSocket handshake\n"));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    };
    let seen = req
        .headers()
        .get(header::SEC_WEBSOCKET_EXTENSIONS)
        .cloned()
        .unwrap_or(HeaderValue::from_static("none"));
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let ws = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
            echo(ws).await;
        }
    });
    let mut response = Response::new(full(""));
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = response.headers_mut();
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(
        header::SEC_WEBSOCKET_ACCEPT,
        accept.parse().expect("accept key is a valid header"),
    );
    headers.insert(SEEN_EXTENSIONS, seen);
    response
}

async fn echo<S>(mut ws: WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(Ok(message)) = ws.next().await {
        let reply = match message {
            Message::Text(text) if text.as_str() == CLOSE_COMMAND => Message::Close(Some(CloseFrame {
                code: CloseCode::from(SERVER_CLOSE_CODE),
                reason: SERVER_CLOSE_REASON.into(),
            })),
            Message::Text(text) if text.as_str() == PING_COMMAND => Message::Ping(Bytes::from_static(SERVER_PING)),
            Message::Text(_) | Message::Binary(_) => message,
            // Pings are answered and closes completed by tungstenite.
            _ => continue,
        };
        if ws.send(reply).await.is_err() {
            break;
        }
    }
}

/// `/sse?count=N&interval_ms=M`: `N` events (default 5), one every `M`
/// ms (default 100), as `id: i`, `event: tick`, `data: {"n":i}`. A comment
/// line comes first.
pub(crate) fn sse(req: &Request<Incoming>) -> Response<TestBody> {
    let param = |name: &str| {
        req.uri()
            .query()
            .unwrap_or_default()
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v.parse::<u64>().ok())
    };
    let body = SseBody {
        next: 0,
        count: param("count").unwrap_or(5) as u32,
        interval: Duration::from_millis(param("interval_ms").unwrap_or(100)),
        sleep: None,
    };
    let mut response = Response::new(body.boxed_unsync());
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// The SSE text of event `n`, as [`sse`] sends it.
pub fn sse_event(n: u32) -> String {
    format!("id: {n}\nevent: tick\ndata: {{\"n\":{n}}}\n\n")
}

struct SseBody {
    next: u32,
    count: u32,
    interval: Duration,
    sleep: Option<Pin<Box<Sleep>>>,
}

impl Body for SseBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if self.next == 0 && self.sleep.is_none() {
            let interval = self.interval;
            self.sleep = Some(Box::pin(tokio::time::sleep(interval)));
            return Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b": stream start\n\n")))));
        }
        if self.next >= self.count {
            return Poll::Ready(None);
        }
        if let Some(sleep) = self.sleep.as_mut() {
            ready!(sleep.as_mut().poll(cx));
        }
        let n = self.next;
        self.next += 1;
        let interval = self.interval;
        self.sleep = Some(Box::pin(tokio::time::sleep(interval)));
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(sse_event(n))))))
    }
}

fn full(data: &'static str) -> TestBody {
    http_body_util::Full::new(Bytes::from_static(data.as_bytes())).boxed_unsync()
}
