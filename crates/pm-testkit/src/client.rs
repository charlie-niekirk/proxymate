//! Minimal clients that talk to a forward proxy the way curl does: they
//! connect to the proxy and send absolute-form request lines, or open a
//! CONNECT tunnel and speak TLS (h1 or h2) through it. WebSocket clients
//! do either too.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;

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

/// A WebSocket handshake sent to the proxy in absolute form (`GET
/// ws://…` becomes `GET http://…`), as clients configured with an HTTP
/// proxy do for plain WebSockets. Returns the response head and, if it was
/// `101`, the socket.
pub async fn ws_absolute(
    proxy: SocketAddr,
    url: &str,
    extra_headers: &[(&str, &str)],
) -> (String, Option<WebSocketStream<TcpStream>>) {
    let uri: http::Uri = url.parse().expect("valid url");
    let authority = uri.authority().expect("url authority").as_str().to_owned();
    let mut request = format!(
        "GET {url} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n"
    );
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    let mut stream = TcpStream::connect(proxy).await.expect("connect to proxy");
    stream.write_all(request.as_bytes()).await.expect("write handshake");
    let head = String::from_utf8_lossy(&crate::upstream::read_head(&mut stream).await).into_owned();
    if !head.starts_with("HTTP/1.1 101 ") {
        return (head, None);
    }
    let ws = WebSocketStream::from_raw_socket(stream, Role::Client, None).await;
    (head, Some(ws))
}

/// A WebSocket handshake for `url` over `stream` (a CONNECT tunnel, or TLS
/// through one).
pub async fn ws_over<S>(stream: S, url: &str) -> WebSocketStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (ws, response) = tokio_tungstenite::client_async(url, stream)
        .await
        .expect("WebSocket handshake through the proxy");
    assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
    ws
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

/// Opens a CONNECT tunnel to `authority` through the proxy and returns the
/// stream once the proxy has answered 200.
pub async fn tunnel(proxy: SocketAddr, authority: &str) -> TcpStream {
    let mut stream = TcpStream::connect(proxy).await.expect("connect to proxy");
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write CONNECT");
    let head = crate::upstream::read_head(&mut stream).await;
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 200 "), "CONNECT answered {head:?}");
    stream
}

/// A TLS client config trusting only `root`, offering `alpn`.
pub fn tls_config(root: &CertificateDer<'_>, alpn: &[&[u8]]) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.clone().into_owned()).expect("add root");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

/// TLS through a CONNECT tunnel to `host:port`, as a browser would do it.
pub async fn tls_through(
    proxy: SocketAddr,
    host: &str,
    port: u16,
    config: Arc<rustls::ClientConfig>,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let stream = tunnel(proxy, &format!("{host}:{port}")).await;
    let name = ServerName::try_from(host.to_owned()).expect("server name");
    tokio_rustls::TlsConnector::from(config).connect(name, stream).await
}

/// What [`https_get`] saw.
#[derive(Debug)]
pub struct HttpsResponse {
    pub status: u16,
    pub version: http::Version,
    /// The protocol the proxy picked for the client side.
    pub alpn: Option<Vec<u8>>,
    pub body: Bytes,
}

/// `GET url` (https) through the proxy's CONNECT, trusting only `root`,
/// offering `alpn`, and speaking whichever protocol was negotiated.
pub async fn https_get(proxy: SocketAddr, url: &str, root: &CertificateDer<'_>, alpn: &[&[u8]]) -> HttpsResponse {
    let uri: http::Uri = url.parse().expect("valid url");
    let host = uri.host().expect("url host").to_owned();
    let port = uri.port_u16().unwrap_or(443);
    let tls = tls_through(proxy, &host, port, tls_config(root, alpn))
        .await
        .expect("TLS handshake through the proxy");
    let negotiated = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
    let io = TokioIo::new(tls);
    let response = if negotiated.as_deref() == Some(b"h2") {
        let (mut sender, conn) = hyper::client::conn::http2::handshake(hyper_util::rt::TokioExecutor::new(), io)
            .await
            .expect("h2 handshake");
        tokio::spawn(conn);
        let req = Request::get(uri.clone())
            .body(Full::<Bytes>::default())
            .expect("request");
        sender.send_request(req).await.expect("h2 request")
    } else {
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.expect("h1 handshake");
        tokio::spawn(conn);
        let pq = uri.path_and_query().map_or("/", |p| p.as_str());
        let req = Request::get(pq)
            .header(http::header::HOST, uri.authority().expect("authority").as_str())
            .body(Full::<Bytes>::default())
            .expect("request");
        sender.send_request(req).await.expect("h1 request")
    };
    let status = response.status().as_u16();
    let version = response.version();
    let body = response.into_body().collect().await.expect("read body").to_bytes();
    HttpsResponse {
        status,
        version,
        alpn: negotiated,
        body,
    }
}
