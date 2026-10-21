//! The proxy listener.
//!
//! Each accepted connection is admitted (LAN clients need the user's OK,
//! see `devices`), sniffed, then served by hyper with header case
//! preserved. Absolute-form requests are forwarded through a pooled
//! upstream client and recorded; CONNECT goes to [`mitm`], which decrypts
//! or tunnels it; requests addressed to the proxy itself (origin form, our
//! own address, or `proxy.mate`) get the [`onboarding`] server, which also
//! stops them looping. WebSocket handshakes on either path switch to the
//! [`ws`] relay; every other request goes through [`exchange`], which
//! applies the rules.

mod body;
mod exchange;
mod io;
mod mitm;
pub mod onboarding;
mod send;
mod sniff;
mod tee;
mod ws;

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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};

pub use send::APP_CLIENT;
pub(crate) use send::Sender;
pub use sniff::{Sniffed, classify};
pub use tee::TeeBody;

use body::RuleBody;
use exchange::{Exchange, Route};

use crate::Engine;
use crate::client::PeerClient;
use crate::devices::{Admission, ONBOARDING_HOST};
use crate::io::content::kind_from_content_type;
use crate::recorder::{FlowRecorder, content_encoding};

type BoxError = Box<dyn StdError + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, BoxError>;
/// A request body on its way upstream, recorded as it goes.
type UpstreamBody = TeeBody<RuleBody>;
type UpstreamClient = Client<HttpConnector, UpstreamBody>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A service error: hyper closes the connection (h1) or resets the stream
/// (h2) instead of answering. How a `Block` rule's reset and a
/// breakpoint's Abort reach the client.
#[derive(Debug)]
pub(crate) struct Reset(String);

impl Reset {
    pub fn new(reason: impl Into<String>) -> Reset {
        Reset(reason.into())
    }
}

impl std::fmt::Display for Reset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for Reset {}

/// Where the proxy listens.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub ip: IpAddr,
    /// Tried in order; the first free port wins.
    pub ports: RangeInclusive<u16>,
}

impl Default for ProxyConfig {
    /// Loopback only. Turning on LAN access (device setup) listens on
    /// every interface instead.
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
    stop: watch::Sender<()>,
    closed: oneshot::Receiver<()>,
}

impl ProxyHandle {
    /// Stops accepting and waits until the port is free again.
    pub async fn stop(self) {
        drop(self.stop);
        let _ = self.closed.await;
    }
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
    let (closed_tx, closed) = oneshot::channel();
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
                _ = stop_rx.changed() => break,
            }
        }
        drop(listener);
        let _ = closed_tx.send(());
    });
    Ok(ProxyHandle {
        addr,
        stop: stop_tx,
        closed,
    })
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
    let Some(engine) = ctx.engine.upgrade() else {
        return;
    };
    let client = match engine.admit(peer.ip()).await {
        Admission::Local => PeerClient::spawn(&engine, peer, ctx.addr.port()),
        Admission::Remote(app) => PeerClient::known(Some(app)),
        Admission::Refused => return refuse(stream).await,
    };
    drop(engine);
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
    let service = service_fn(move |req| handle(ctx.clone(), client.clone(), req));
    let _ = hyper::server::conn::http1::Builder::new()
        .preserve_header_case(true)
        .half_close(false)
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades()
        .await;
}

/// A LAN client the user denied, or didn't answer for in time. HTTP
/// clients are told why; anything else is just closed.
async fn refuse(mut stream: TcpStream) {
    if !sniff::sniff(&stream).await.is_some_and(Sniffed::is_http) {
        return;
    }
    // Read what has arrived so closing doesn't reset the connection
    // before the client sees the answer.
    let mut buf = [0u8; 4096];
    let _ = tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await;
    let body = "Proxymate: this device isn't allowed to use the proxy. Allow it on the computer running Proxymate.\n";
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn handle(
    ctx: Arc<ProxyCtx>,
    client: Arc<PeerClient>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Reset> {
    if req.method() == Method::CONNECT {
        return Ok(mitm::connect(ctx, client, req).await);
    }
    match req.uri().scheme_str() {
        Some("http") if !is_onboarding(&ctx, req.uri()) => forward(&ctx, &client, req).await,
        Some("http") | None => Ok(onboarding_response(&ctx, &client, &req)),
        Some(_) => Ok(text_response(
            StatusCode::NOT_IMPLEMENTED,
            "Proxymate: send HTTPS requests through a CONNECT tunnel.\n",
        )),
    }
}

/// An absolute-form request for the onboarding host, or for this proxy
/// itself, which forwarding would loop back to us.
fn is_onboarding(ctx: &ProxyCtx, uri: &Uri) -> bool {
    if uri.host().is_some_and(|h| h.eq_ignore_ascii_case(ONBOARDING_HOST)) {
        return true;
    }
    let engine = ctx.engine.upgrade();
    targets_self(uri, ctx.addr, |ip| {
        engine.as_ref().is_some_and(|e| e.device_registry().is_local(ip))
    })
}

fn onboarding_response(ctx: &ProxyCtx, client: &PeerClient, req: &Request<Incoming>) -> Response<ProxyBody> {
    let Some(engine) = ctx.engine.upgrade() else {
        return text_response(StatusCode::SERVICE_UNAVAILABLE, "Proxymate is shutting down\n");
    };
    let path = req.uri().path();
    let page = if matches!(*req.method(), Method::GET | Method::HEAD) {
        let tls = engine.tls();
        // The address the device reached us on is the one to set as its proxy.
        let proxy = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<SocketAddr>().ok())
            .filter(|_| req.uri().scheme().is_none())
            .unwrap_or_else(|| engine.display_addr(ctx.addr));
        onboarding::route(
            path,
            &onboarding::Onboarding {
                ca: tls.ca.info(),
                der: tls.ca.cert_der(),
                proxy: Some(proxy.to_string()),
                user_agent: req
                    .headers()
                    .get(header::USER_AGENT)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default(),
            },
        )
    } else {
        onboarding::method_not_allowed()
    };
    if let Some(ip) = client.remote_ip()
        && page.status == 200
    {
        engine.device_registry().note_onboarding(ip);
    }
    let mut response = Response::new(
        Full::new(Bytes::from(page.body))
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = StatusCode::from_u16(page.status).unwrap_or(StatusCode::OK);
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(page.content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(name) = page.file_name
        && let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

async fn forward(
    ctx: &ProxyCtx,
    client: &Arc<PeerClient>,
    mut req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Reset> {
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
    if !websocket {
        let uri = req.uri();
        let exchange = Exchange {
            ctx,
            route: Route::Forward,
            peer: client,
            recorder,
            scheme: Scheme::Http,
            host: uri
                .host()
                .unwrap_or_default()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port: uri.port_u16().unwrap_or(80),
            apply_rules: true,
        };
        return exchange.run(req).await;
    }
    // Rules don't apply to WebSocket handshakes (yet).
    let client_upgrade = hyper::upgrade::on(&mut req);
    let (mut parts, body) = req.into_parts();
    ws::prepare_request(&mut parts.headers);
    let body = TeeBody::new(RuleBody::stream(body), recorder.clone(), BodyPart::Request);
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
            return Ok(text_response(
                StatusCode::BAD_GATEWAY,
                &format!("Proxymate: upstream error: {message}\n"),
            ));
        }
    };
    if let Some(recorder) = &recorder {
        let content_kind = content_kind(upstream.headers());
        recorder.response(response_head(&upstream), content_kind);
    }
    if upstream.status() == StatusCode::SWITCHING_PROTOCOLS {
        return Ok(ws::switch(recorder, client_upgrade, upstream));
    }
    let (mut parts, body) = upstream.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    let body = TeeBody::new(body, recorder, BodyPart::Response);
    Ok(Response::from_parts(parts, body.map_err(BoxError::from).boxed_unsync()))
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
        sent_by: None,
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
        rules: Default::default(),
    };
    let mut detail = detail;
    detail.request_body.encoding = content_encoding(&detail.request.headers);
    origin.client.begin_flow(engine, summary, detail)
}

fn response_head<B>(resp: &Response<B>) -> ResponseHead {
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

/// A response head built from parts (rules and breakpoints).
fn head_of(status: u16, headers: &HeaderMap, version: Version) -> ResponseHead {
    ResponseHead {
        status,
        reason: StatusCode::from_u16(status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or_default()
            .to_owned(),
        version: self::version(version),
        headers: headers_of(headers),
    }
}

pub(crate) fn headers_of(map: &HeaderMap) -> Vec<Header> {
    headers(map)
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
/// `is_local` says whether an address is one of this machine's.
fn targets_self(uri: &Uri, addr: SocketAddr, is_local: impl Fn(IpAddr) -> bool) -> bool {
    if uri.port_u16().unwrap_or(80) != addr.port() {
        return false;
    }
    match uri.host().map(|h| h.trim_start_matches('[').trim_end_matches(']')) {
        Some(host) if host.eq_ignore_ascii_case("localhost") => true,
        Some(host) => host
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip == addr.ip() || ip.is_loopback() || ip.is_unspecified() || is_local(ip)),
        None => true,
    }
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
        let lan: IpAddr = "10.0.0.5".parse().unwrap();
        let is_local = |ip: IpAddr| ip == lan;
        assert!(targets_self(&"http://127.0.0.1:9090/".parse().unwrap(), addr, is_local));
        assert!(targets_self(
            &"http://localhost:9090/x".parse().unwrap(),
            addr,
            is_local
        ));
        assert!(targets_self(&"http://[::1]:9090/".parse().unwrap(), addr, is_local));
        assert!(!targets_self(
            &"http://127.0.0.1:9091/".parse().unwrap(),
            addr,
            is_local
        ));
        assert!(!targets_self(
            &"http://example.com:9090/".parse().unwrap(),
            addr,
            is_local
        ));
        // Listening on every interface: our LAN address is us too.
        let any: SocketAddr = "0.0.0.0:9090".parse().unwrap();
        assert!(targets_self(&"http://10.0.0.5:9090/".parse().unwrap(), any, is_local));
        assert!(!targets_self(&"http://10.0.0.6:9090/".parse().unwrap(), any, is_local));
    }

    #[test]
    fn error_chain_includes_sources() {
        let inner = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
        let outer = std::io::Error::other(inner);
        assert_eq!(error_chain(&outer), "Connection refused");
    }
}
