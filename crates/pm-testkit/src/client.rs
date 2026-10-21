//! Minimal clients that talk to a forward proxy the way curl does: they
//! connect to the proxy and send absolute-form request lines.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Sends `req` (whose URI must be absolute) through the proxy at `proxy`
/// on a fresh connection. Adds a `Host` header, as curl does.
pub async fn send(proxy: SocketAddr, mut req: Request<Full<Bytes>>) -> Response<Incoming> {
    if let Some(authority) = req.uri().authority().map(|a| a.to_string()) {
        req.headers_mut()
            .entry(http::header::HOST)
            .or_insert(authority.parse().expect("authority is a valid header"));
    }
    let stream = TcpStream::connect(proxy).await.expect("connect to proxy");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake with proxy");
    tokio::spawn(conn);
    sender.send_request(req).await.expect("send through proxy")
}

/// `GET url` through the proxy; returns the status and the whole body.
pub async fn get(proxy: SocketAddr, url: &str) -> (u16, Bytes) {
    let req = Request::get(url).body(Full::default()).expect("valid request");
    let response = send(proxy, req).await;
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.expect("read body").to_bytes();
    (status, body)
}

/// Writes `request` verbatim to the proxy and returns everything it sends
/// back until it closes the connection.
pub async fn raw(proxy: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(proxy).await.expect("connect to proxy");
    stream.write_all(request).await.expect("write request");
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut out)).await;
    out
}

/// Polls `check` until it returns `Some`, or panics after `timeout`.
pub async fn eventually<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let start = std::time::Instant::now();
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(start.elapsed() < timeout, "condition not met within {timeout:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
