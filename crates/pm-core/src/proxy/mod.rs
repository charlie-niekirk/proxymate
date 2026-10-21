//! The proxy listener.
//!
//! Each accepted connection is sniffed, then served by hyper with header
//! case preserved. Absolute-form requests are forwarded through a pooled
//! upstream client and recorded; CONNECT goes to [`mitm`], which decrypts
//! or tunnels it; origin-form requests are addressed to the proxy itself
//! and get a short info page (the onboarding server is M9). WebSocket
//! handshakes on either path switch to the [`ws`] relay.

mod io;
mod mitm;
mod sniff;
mod tee;
mod ws;

use std::convert::Infallible;
use std::error::Error as StdError;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::RangeInclusive;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode, Uri, Version};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pm_model::{
    BodyInfo, BodyPart, ContentKind, FlowDetail, FlowState, FlowSummary, Header, HttpVersion, RequestHead,
    ResponseHead, Scheme, Timing, TlsInfo, TlsStatus,
};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

pub use sniff::{Sniffed, classify};
pub use tee::TeeBody;

use crate::Engine;
use crate::client::PeerClient;
use crate::io::content::kind_from_content_type;
use crate::recorder::{FlowRecorder, content_encoding};

type BoxError = Box<dyn StdError + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, BoxError>;
type UpstreamClient = Client<HttpConnector, TeeBody<Incoming>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Where the proxy listens.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub ip: IpAddr,
    /// Tried in order; the first free port wins.
    pub ports: RangeInclusive<u16>,
}

impl Default for ProxyConfig {
    /// Loopback only: LAN clients need the allow-list from M9.
    fn default() -> Self {
        ProxyConfig {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ports: 9090..=9099,
        }
    }
}

/// A running listener. Dropping it stops accepting connections; open
/// connections finish on their own.
pub(crate) struct ProxyHandle {
    pub addr: SocketAddr,
    _stop: watch::Sender<()>,
}

/// Binds the first free port in `config.ports`.
pub(crate) async fn bind(config: &ProxyConfig) -> std::io::Result<TcpListener> {
    let mut last_err = None;
    for port in config.ports.clone() {
        match TcpListener::bind((config.ip, port)).await {
            Ok(listener) => return Ok(listener),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => last_err = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no ports to try")))
}

/// Starts the accept loop on the current tokio runtime.
pub(crate) fn spawn(engine: Weak<Engine>, listener: TcpListener) -> std::io::Result<ProxyHandle> {
    let addr = listener.local_addr()?;
    let (stop_tx, mut stop_rx) = watch::channel(());
    let ctx = Arc::new(ProxyCtx {
        engine,
        client: upstream_client(),
        addr,
    });
    tokio::spawn(async move {
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        tokio::spawn(serve_connection(ctx.clone(), stream, peer));
                    }
                    // Out of file descriptors and the like: back off briefly.
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                _ = stop_rx.changed() => return,
            }
        }
    });
    Ok(ProxyHandle { addr, _stop: stop_tx })
}

struct ProxyCtx {
    engine: Weak<Engine>,
    client: UpstreamClient,
    /// The listener's own address, to refuse requests that would loop.
    addr: SocketAddr,
}

/// The upstream client talks to origin servers directly. hyper-util's
/// connector never consults `HTTP_PROXY` or the system proxy settings, which
/// matters: once Proxymate is the system proxy (M4), using it here would
/// send every request back to ourselves.
fn upstream_client() -> UpstreamClient {
    let mut connector = HttpConnector::new();
    connector.set_nodelay(true);
    connector.set_connect_timeout(Some(CONNECT_TIMEOUT));
    Client::builder(TokioExecutor::new())
        .http1_preserve_header_case(true)
        .pool_idle_timeout(Duration::from_secs(90))
        .build(connector)
}

async fn serve_connection(ctx: Arc<ProxyCtx>, mut stream: TcpStream, peer: SocketAddr) {
    match sniff::sniff(&stream).await {
        Some(kind) if kind.is_http() => {}
        Some(Sniffed::Garbage) => {
            let _ = stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nconnection: close\r\ncontent-length: 0\r\n\r\n")
                .await;
            return;
        }
        // TLS straight at the proxy port can't be answered in HTTP; just close.
        _ => return,
    }
    let _ = stream.set_nodelay(true);
    let client = match ctx.engine.upgrade() {
        Some(engine) => PeerClient::spawn(&engine, peer, ctx.addr.port()),
        None => PeerClient::unknown(),
    };
    let service = service_fn(move |req| handle(ctx.clone(), client.clone(), req));
    let _ = hyper::server::conn::http1::Builder::new()
        .preserve_header_case(true)
        .half_close(false)
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades()
        .await;
}

async fn handle(
    ctx: Arc<ProxyCtx>,
    client: Arc<PeerClient>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    if req.method() == Method::CONNECT {
        return Ok(mitm::connect(ctx, client, req).await);
    }
    let response = match req.uri().scheme_str() {
        Some("http") if !targets_self(req.uri(), ctx.addr) => forward(&ctx, &client, req).await,
        Some("http") | None => info_page(ctx.addr),
        Some(_) => text_response(
            StatusCode::NOT_IMPLEMENTED,
            "Proxymate: send HTTPS requests through a CONNECT tunnel.\n",
        ),
    };
    Ok(response)
}

async fn forward(ctx: &ProxyCtx, client: &Arc<PeerClient>, mut req: Request<Incoming>) -> Response<ProxyBody> {
    let websocket = ws::is_upgrade(&req);
    let recorder = ctx.engine.upgrade().and_then(|engine| {
        let uri = req.uri();
        let origin = Origin {
            scheme: if websocket { Scheme::Ws } else { Scheme::Http },
            host: uri.host().unwrap_or_default().to_owned(),
            port: uri.port_u16().unwrap_or(80),
            url: uri.to_string(),
            tls: TlsStatus::None,
            tls_info: None,
            client: client.clone(),
        };
        record_request(&engine, &req, origin)
    });
    let client_upgrade = websocket.then(|| hyper::upgrade::on(&mut req));
    let (mut parts, body) = req.into_parts();
    if websocket {
        ws::prepare_request(&mut parts.headers);
    } else {
        strip_hop_by_hop(&mut parts.headers);
    }
    let body = TeeBody::new(body, recorder.clone(), BodyPart::Request);
    if let Some(recorder) = &recorder {
        recorder.request_sent();
    }
    let upstream = match ctx.client.request(Request::from_parts(parts, body)).await {
        Ok(response) => response,
        Err(err) => {
            let message = error_chain(&err);
            if let Some(recorder) = &recorder {
                recorder.fail(message.clone());
            }
            return text_response(
                StatusCode::BAD_GATEWAY,
                &format!("Proxymate: upstream error: {message}\n"),
            );
        }
    };
    if let Some(recorder) = &recorder {
        let content_kind = content_kind(upstream.headers());
        recorder.response(response_head(&upstream), content_kind);
    }
    if let Some(client_upgrade) = client_upgrade
        && upstream.status() == StatusCode::SWITCHING_PROTOCOLS
    {
        return ws::switch(recorder, client_upgrade, upstream);
    }
    let (mut parts, body) = upstream.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    let body = TeeBody::new(body, recorder, BodyPart::Response);
    Response::from_parts(parts, body.map_err(BoxError::from).boxed_unsync())
}

/// Where a recorded request went, beyond what its head says.
struct Origin {
    scheme: Scheme,
    host: String,
    port: u16,
    /// The absolute URL, as shown in the inspector.
    url: String,
    tls: TlsStatus,
    tls_info: Option<TlsInfo>,
    /// The app the connection came from.
    client: Arc<PeerClient>,
}

/// Starts recording `req`, unless recording is paused.
fn record_request(engine: &Arc<Engine>, req: &Request<Incoming>, origin: Origin) -> Option<FlowRecorder> {
    let uri = req.uri();
    let summary = FlowSummary {
        id: 0,
        state: FlowState::Active,
        scheme: origin.scheme,
        method: req.method().as_str().into(),
        host: origin.host.as_str().into(),
        port: origin.port,
        path: uri.path().to_owned(),
        query: uri.query().unwrap_or_default().to_owned(),
        status: None,
        content_kind: content_kind(req.headers()),
        client: None,
        started_at_ms: now_ms(),
        duration_ms: None,
        req_size: 0,
        resp_size: 0,
        edited: false,
        http_version: version(req.version()),
        tls: origin.tls,
        graphql_op: None,
    };
    let headers = headers(req.headers());
    let detail = FlowDetail {
        id: 0,
        request: RequestHead {
            method: req.method().to_string(),
            url: origin.url,
            version: version(req.version()),
            headers,
        },
        response: None,
        request_body: BodyInfo::default(),
        response_body: BodyInfo::default(),
        error: None,
        tls: origin.tls_info,
        timing: Timing::default(),
    };
    let mut detail = detail;
    detail.request_body.encoding = content_encoding(&detail.request.headers);
    origin.client.begin_flow(engine, summary, detail)
}

fn response_head(resp: &Response<Incoming>) -> ResponseHead {
    let status = resp.status();
    let reason = resp
        .extensions()
        .get::<hyper::ext::ReasonPhrase>()
        .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned())
        .or_else(|| status.canonical_reason().map(str::to_owned))
        .unwrap_or_default();
    ResponseHead {
        status: status.as_u16(),
        reason,
        version: version(resp.version()),
        headers: headers(resp.headers()),
    }
}

fn headers(map: &HeaderMap) -> Vec<Header> {
    map.iter()
        .map(|(name, value)| Header::new(name.as_str(), String::from_utf8_lossy(value.as_bytes())))
        .collect()
}

fn content_kind(headers: &HeaderMap) -> ContentKind {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map_or(ContentKind::None, kind_from_content_type)
}

fn version(v: Version) -> HttpVersion {
    match v {
        Version::HTTP_10 | Version::HTTP_09 => HttpVersion::Http10,
        Version::HTTP_2 => HttpVersion::Http2,
        _ => HttpVersion::Http11,
    }
}

/// Removes headers that apply to one hop only (RFC 9110 §7.6.1), including
/// any named in `Connection`. hyper frames each side itself.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in listed {
        headers.remove(name);
    }
    // `te: trailers` must reach h2 upstreams: gRPC requires it.
    let te_trailers = headers
        .get(header::TE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"trailers"));
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    for name in ["keep-alive", "proxy-connection"] {
        headers.remove(name);
    }
    if te_trailers {
        headers.insert(header::TE, HeaderValue::from_static("trailers"));
    }
}

/// True if `uri` points at this proxy's own listener, which would loop.
fn targets_self(uri: &Uri, addr: SocketAddr) -> bool {
    if uri.port_u16().unwrap_or(80) != addr.port() {
        return false;
    }
    match uri.host().map(|h| h.trim_start_matches('[').trim_end_matches(']')) {
        Some("localhost") => true,
        Some(host) => host
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip == addr.ip() || ip.is_loopback() || ip.is_unspecified()),
        None => true,
    }
}

fn info_page(addr: SocketAddr) -> Response<ProxyBody> {
    text_response(
        StatusCode::OK,
        &format!(
            "Proxymate is running.\n\nSet http://{addr} as the HTTP proxy of the client you want to inspect, e.g.\n\n  curl -x http://{addr} http://example.com/\n"
        ),
    )
}

fn text_response(status: StatusCode, text: &str) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from(text.to_owned()))
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// `err` and its sources, joined, so "client error (Connect)" comes with
/// the "Connection refused" underneath it.
fn error_chain(err: &dyn StdError) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(err) = source {
        let text = err.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = err.source();
    }
    message
}

pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers_are_stripped() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive, x-private"));
        headers.insert("x-private", HeaderValue::from_static("1"));
        headers.insert("proxy-connection", HeaderValue::from_static("keep-alive"));
        headers.insert(header::TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        strip_hop_by_hop(&mut headers);
        assert_eq!(headers.keys().map(|k| k.as_str()).collect::<Vec<_>>(), vec!["accept"]);

        let mut headers = HeaderMap::new();
        headers.insert(header::TE, HeaderValue::from_static("trailers"));
        strip_hop_by_hop(&mut headers);
        assert_eq!(headers.get(header::TE).unwrap(), "trailers");
        headers.insert(header::TE, HeaderValue::from_static("gzip"));
        strip_hop_by_hop(&mut headers);
        assert!(headers.get(header::TE).is_none());
    }

    #[test]
    fn requests_to_the_proxy_itself_are_detected() {
        let addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
        assert!(targets_self(&"http://127.0.0.1:9090/".parse().unwrap(), addr));
        assert!(targets_self(&"http://localhost:9090/x".parse().unwrap(), addr));
        assert!(targets_self(&"http://[::1]:9090/".parse().unwrap(), addr));
        assert!(!targets_self(&"http://127.0.0.1:9091/".parse().unwrap(), addr));
        assert!(!targets_self(&"http://example.com:9090/".parse().unwrap(), addr));
    }

    #[test]
    fn error_chain_includes_sources() {
        let inner = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
        let outer = std::io::Error::other(inner);
        assert_eq!(error_chain(&outer), "Connection refused");
    }
}
