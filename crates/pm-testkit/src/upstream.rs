//! Plain HTTP/1.1 test upstreams on loopback ephemeral ports.
//!
//! [`Upstream`] is a hyper server with fixed routes. [`RawUpstream`] is a
//! bare TCP server that records the request head byte for byte and replies
//! with canned bytes, for tests that care about exact wire format.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::{Request, Response, StatusCode, header};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Served by `/json` and, encoded, by `/gzip`, `/br`, `/deflate` and `/zstd`.
pub const JSON_BODY: &str =
    r#"{"name":"proxymate","version":1,"features":["capture","inspect","decode"],"nested":{"ok":true,"count":3}}"#;

/// The chunks `/chunked` sends, in order.
pub const CHUNKS: [&str; 4] = ["first chunk\n", "second chunk\n", "third chunk\n", "last chunk\n"];

/// Byte `i` of the deterministic `/bytes/{n}` body.
pub fn pattern_byte(i: u64) -> u8 {
    (i % 251) as u8
}

/// Routes:
/// - `/json`: a JSON document.
/// - `/status/{code}`: that status, with a short text body unless the code forbids one.
/// - `/chunked`: [`CHUNKS`] with chunked transfer encoding.
/// - `/gzip`, `/br`, `/deflate`, `/zstd`: [`JSON_BODY`] content-encoded.
/// - `/bytes/{n}`: `n` pattern bytes, generated while streaming.
/// - `/echo`: the request body back, with the request's content type.
/// - `/version`: the request's HTTP version, e.g. `HTTP/2.0`.
/// - `/zoo`: the zoo fixture names; `/zoo/{name}`: that fixture (see [`crate::zoo`]).
/// - `/ws`: a WebSocket echo server (h1 only; see [`crate::stream`]).
/// - `/sse?count=N&interval_ms=M`: a slow `text/event-stream`.
pub struct Upstream {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Upstream {
    pub async fn start() -> Upstream {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind upstream");
        let addr = listener.local_addr().expect("upstream addr");
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service_fn(route))
                        .with_upgrades()
                        .await;
                });
            }
        });
        Upstream { addr, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) type TestBody = http_body_util::combinators::UnsyncBoxBody<Bytes, Infallible>;

pub(crate) async fn route(req: Request<Incoming>) -> Result<Response<TestBody>, Infallible> {
    let path = req.uri().path().to_owned();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let response = match segments.as_slice() {
        ["json"] => with_type(full(JSON_BODY), "application/json"),
        ["ws"] => crate::stream::websocket(req),
        ["sse"] => crate::stream::sse(&req),
        ["version"] => with_type(full(format!("{:?}", req.version())), "text/plain"),
        ["status", code] => {
            let status = code.parse().ok().and_then(|c| StatusCode::from_u16(c).ok());
            let status = status.unwrap_or(StatusCode::BAD_REQUEST);
            let body = if status.is_informational() || matches!(status.as_u16(), 204 | 304) {
                full("")
            } else {
                full(format!("status {}\n", status.as_u16()))
            };
            let mut response = with_type(body, "text/plain");
            *response.status_mut() = status;
            response
        }
        ["chunked"] => with_type(
            ChunkedBody(CHUNKS.iter().map(|c| Bytes::from_static(c.as_bytes())).collect()).boxed_unsync(),
            "text/plain",
        ),
        [coding @ ("gzip" | "br" | "deflate" | "zstd")] => {
            let mut response = with_type(full(encode(coding, JSON_BODY.as_bytes())), "application/json");
            response
                .headers_mut()
                .insert(header::CONTENT_ENCODING, coding.parse().expect("valid header"));
            response
        }
        ["bytes", n] => match n.parse() {
            Ok(len) => with_type(PatternBody { sent: 0, len }.boxed_unsync(), "application/octet-stream"),
            Err(_) => status_only(StatusCode::BAD_REQUEST),
        },
        ["zoo"] => {
            let names: Vec<&str> = crate::zoo::fixtures().iter().map(|f| f.name.as_str()).collect();
            with_type(full(serde_json::to_vec(&names).expect("names")), "application/json")
        }
        ["zoo", name] => match crate::zoo::fixture(name) {
            Some(fixture) => {
                // Drain the request body, as a real endpoint would.
                let _ = req.into_body().collect().await;
                let mut response = Response::new(full(fixture.response_body()));
                *response.status_mut() = StatusCode::from_u16(fixture.status).unwrap_or(StatusCode::OK);
                let headers = response.headers_mut();
                if let Some(ct) = &fixture.content_type {
                    headers.insert(header::CONTENT_TYPE, ct.parse().expect("valid content type"));
                }
                if let Some(coding) = &fixture.encoding {
                    headers.insert(header::CONTENT_ENCODING, coding.parse().expect("valid coding"));
                }
                response
            }
            None => status_only(StatusCode::NOT_FOUND),
        },
        ["echo"] => {
            let content_type = req.headers().get(header::CONTENT_TYPE).cloned();
            let body = req
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            let mut response = Response::new(full(body));
            if let Some(ct) = content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            response
        }
        _ => status_only(StatusCode::NOT_FOUND),
    };
    Ok(response)
}

/// `data` compressed with an HTTP content coding.
pub fn encode(coding: &str, data: &[u8]) -> Vec<u8> {
    match coding {
        "gzip" => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).expect("gzip");
            enc.finish().expect("gzip")
        }
        "deflate" => {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(data).expect("deflate");
            enc.finish().expect("deflate")
        }
        "br" => {
            let mut out = Vec::new();
            {
                let mut enc = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
                enc.write_all(data).expect("br");
            }
            out
        }
        "zstd" => zstd::stream::encode_all(data, 3).expect("zstd"),
        other => panic!("unknown coding {other}"),
    }
}

fn full(data: impl Into<Bytes>) -> TestBody {
    http_body_util::Full::new(data.into()).boxed_unsync()
}

fn with_type(body: TestBody, content_type: &'static str) -> Response<TestBody> {
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, header::HeaderValue::from_static(content_type));
    response
}

fn status_only(status: StatusCode) -> Response<TestBody> {
    let mut response = Response::new(full(""));
    *response.status_mut() = status;
    response
}

/// No size hint, so hyper uses chunked transfer encoding.
struct ChunkedBody(VecDeque<Bytes>);

impl Body for ChunkedBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Ready(self.0.pop_front().map(|chunk| Ok(Frame::data(chunk))))
    }
}

/// `len` pattern bytes, generated 64 KiB at a time so a huge body costs
/// the upstream no memory.
struct PatternBody {
    sent: u64,
    len: u64,
}

impl Body for PatternBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if self.sent >= self.len {
            return Poll::Ready(None);
        }
        let n = (self.len - self.sent).min(64 * 1024);
        let chunk: Vec<u8> = (self.sent..self.sent + n).map(pattern_byte).collect();
        self.sent += n;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(chunk)))))
    }

    fn is_end_stream(&self) -> bool {
        self.sent >= self.len
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.len - self.sent)
    }
}

/// Accepts connections, records each request head (up to the blank line)
/// verbatim, replies with `response` and closes.
pub struct RawUpstream {
    addr: SocketAddr,
    heads: Arc<Mutex<Vec<Vec<u8>>>>,
    task: JoinHandle<()>,
}

impl RawUpstream {
    pub async fn start(response: &'static [u8]) -> RawUpstream {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind raw upstream");
        let addr = listener.local_addr().expect("raw upstream addr");
        let heads = Arc::new(Mutex::new(Vec::new()));
        let recorded = heads.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let head = read_head(&mut stream).await;
                recorded.lock().await.push(head);
                let _ = stream.write_all(response).await;
                let _ = stream.shutdown().await;
            }
        });
        RawUpstream { addr, heads, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Request heads received so far, in order.
    pub async fn heads(&self) -> Vec<Vec<u8>> {
        self.heads.lock().await.clone()
    }
}

impl Drop for RawUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads up to and including the first `\r\n\r\n`.
pub async fn read_head(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte).await {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    head
}
