//! CONNECT: decrypt or tunnel.
//!
//! After `200 Connection Established` the SSL lists decide. Excluded hosts
//! are spliced byte for byte and recorded as one tunnel flow. Everything
//! else is intercepted, upstream first:
//!
//! 1. Read the client's ClientHello (SNI, ALPN) with `LazyConfigAcceptor`.
//! 2. Connect upstream offering the same ALPN, limited to `h2` and
//!    `http/1.1`, and see what the upstream picks.
//! 3. Finish the client handshake offering exactly that protocol, so both
//!    sides speak the same HTTP version and frames never need translating.
//! 4. Serve the decrypted connection with hyper and forward each request
//!    over the upstream connection, recording it as an HTTPS flow.
//!
//! If the upstream handshake fails, the client still gets a handshake and a
//! readable 502 page per request. If the client refuses our certificate,
//! the CONNECT is recorded as "likely pinned". Plain HTTP inside a CONNECT
//! is served as HTTP.

use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http::header::{self, HeaderValue};
use http::uri::{Authority, PathAndQuery};
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::client::conn::{http1 as client_h1, http2 as client_h2};
use hyper::ext::ReasonPhrase;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pm_model::{
    BodyInfo, BodyPart, ContentKind, FlowDetail, FlowState, FlowSummary, HttpVersion, RequestHead, ResponseHead,
    Scheme, Timing, TlsInfo, TlsStatus,
};
use rustls::AlertDescription;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTls;
use tokio_rustls::{LazyConfigAcceptor, TlsConnector};

use super::io::{Counters, Counting, Prefixed};
use super::{
    BoxError, CONNECT_TIMEOUT, Origin, ProxyBody, ProxyCtx, TeeBody, content_kind, error_chain, forward, headers,
    now_ms, record_request, response_head, strip_hop_by_hop, text_response,
};
use crate::Engine;
use crate::recorder::FlowRecorder;
use crate::tls::{H2, HTTP11, TlsContext, summarize};

/// A tunnel whose client says nothing for this long is dropped.
const FIRST_BYTES_TIMEOUT: Duration = Duration::from_secs(30);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// A client that finishes the handshake, then closes within this long
/// without sending a byte most likely checked our certificate after the
/// handshake and rejected it (pinning). Idle preconnects live far longer.
const PINNED_CLOSE_WINDOW: Duration = Duration::from_secs(2);
const TUNNEL_PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

type UpstreamBody = TeeBody<Incoming>;

/// Where the CONNECT asked to go.
#[derive(Clone)]
struct Target {
    host: String,
    port: u16,
    head: RequestHead,
}

/// Handles a CONNECT request and returns its response; the tunnel itself
/// runs in a spawned task once hyper hands over the connection.
pub(super) async fn connect(ctx: Arc<ProxyCtx>, req: Request<Incoming>) -> Response<ProxyBody> {
    let Some(authority) = req.uri().authority().cloned() else {
        return text_response(StatusCode::BAD_REQUEST, "Proxymate: CONNECT needs host:port\n");
    };
    let Some(engine) = ctx.engine.upgrade() else {
        return text_response(StatusCode::SERVICE_UNAVAILABLE, "Proxymate is shutting down\n");
    };
    let target = Target {
        host: authority
            .host()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port: authority.port_u16().unwrap_or(443),
        head: RequestHead {
            method: "CONNECT".into(),
            url: authority.to_string(),
            version: super::version(req.version()),
            headers: headers(req.headers()),
        },
    };
    if !engine.ssl_policy().intercepts(&target.host) {
        return passthrough(&engine, req, target).await;
    }
    drop(engine);
    tokio::spawn(async move {
        if let Ok(upgraded) = hyper::upgrade::on(req).await {
            intercept(ctx, TokioIo::new(upgraded), target).await;
        }
    });
    connection_established()
}

fn connection_established() -> Response<ProxyBody> {
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed_unsync());
    response
        .extensions_mut()
        .insert(ReasonPhrase::from_static(b"Connection Established"));
    response
}

/// Splices the tunnel without decrypting it, as one tunnel flow. The
/// upstream is dialled before answering, so a refused connection still
/// gets a 502.
async fn passthrough(engine: &Arc<Engine>, req: Request<Incoming>, target: Target) -> Response<ProxyBody> {
    let recorder = record_connect(engine, &target, TlsStatus::Passthrough, None);
    let (tcp, timing) = match dial_tcp(&target.host, target.port).await {
        Ok(dialled) => dialled,
        Err(message) => {
            if let Some(recorder) = &recorder {
                recorder.fail(message.clone());
            }
            return text_response(StatusCode::BAD_GATEWAY, &format!("Proxymate: {message}\n"));
        }
    };
    if let Some(recorder) = &recorder {
        recorder.connection_timing(&timing);
        recorder.response(connect_response_head(), ContentKind::None);
    }
    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(req).await else {
            if let Some(recorder) = &recorder {
                recorder.fail("the client went away before the tunnel opened");
            }
            return;
        };
        let counters = Arc::new(Counters::default());
        let mut client = Counting::new(TokioIo::new(upgraded), counters.clone());
        let mut upstream = tcp;
        let mut ticker = tokio::time::interval(TUNNEL_PROGRESS_INTERVAL);
        let relay = tokio::io::copy_bidirectional(&mut client, &mut upstream);
        tokio::pin!(relay);
        loop {
            tokio::select! {
                _ = &mut relay => break,
                _ = ticker.tick() => {
                    if let Some(recorder) = &recorder {
                        recorder.tunnel_progress(counters.read(), counters.written());
                    }
                }
            }
        }
        if let Some(recorder) = &recorder {
            recorder.tunnel_closed(counters.read(), counters.written());
        }
    });
    connection_established()
}

/// The decrypting path, run on the upgraded CONNECT stream.
async fn intercept<S>(ctx: Arc<ProxyCtx>, mut io: S, target: Target)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut first = [0u8; 1];
    let n = match tokio::time::timeout(FIRST_BYTES_TIMEOUT, io.read(&mut first)).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    let io = Prefixed::new(Bytes::copy_from_slice(&first[..n]), io);
    if first[0] != 0x16 {
        return serve_plain(ctx, io, target).await;
    }

    let acceptor = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), io);
    let Ok(Ok(start)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor).await else {
        return;
    };
    let (sni, client_alpn) = {
        let hello = start.client_hello();
        let alpn: Vec<Vec<u8>> = hello
            .alpn()
            .map(|ps| ps.map(<[u8]>::to_vec).collect())
            .unwrap_or_default();
        (hello.server_name().map(str::to_owned), alpn)
    };
    let offered: Vec<Vec<u8>> = client_alpn
        .iter()
        .filter(|p| p.as_slice() == H2 || p.as_slice() == HTTP11)
        .cloned()
        .collect();
    let Some(engine) = ctx.engine.upgrade() else {
        return;
    };
    let tls = engine.tls();
    let policy = engine.ssl_policy();
    let insecure =
        policy.insecure_upstream(&target.host) || sni.as_deref().is_some_and(|s| policy.insecure_upstream(s));
    drop((engine, policy));

    let dial = Dial {
        tls: tls.clone(),
        host: target.host.clone(),
        port: target.port,
        sni: sni.clone(),
        offered: offered.clone(),
        insecure,
    };
    let upstream = dial.connect().await;
    let mut info = TlsInfo {
        sni: sni.clone(),
        client_alpn: client_alpn
            .iter()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
        alpn: None,
        version: String::new(),
        cipher: String::new(),
        upstream_verified: !insecure,
        chain: Vec::new(),
    };
    let alpn: Option<Vec<u8>> = match &upstream {
        Ok(up) => {
            info.alpn = up.alpn.as_deref().map(|p| String::from_utf8_lossy(p).into_owned());
            info.version = up.version.clone();
            info.cipher = up.cipher.clone();
            info.chain = up.chain.clone();
            up.alpn.clone()
        }
        Err(failure) => {
            info.chain = failure.chain.clone();
            // Nothing to mirror: let the client speak whatever it prefers
            // to read the error page.
            [HTTP11, H2]
                .into_iter()
                .find(|p| offered.iter().any(|o| o == p))
                .map(<[u8]>::to_vec)
        }
    };

    let config = match tls.server_config(alpn.as_deref(), sni.as_deref(), &target.host) {
        Ok(config) => config,
        Err(_) => return,
    };
    let client = match tokio::time::timeout(HANDSHAKE_TIMEOUT, start.into_stream(config)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => {
            record_rejection(&ctx, &target, info, &err);
            return;
        }
        Err(_) => return,
    };
    let handshake_done = Instant::now();
    let client_h2 = client.get_ref().1.alpn_protocol() == Some(H2);

    let (sender, failure, timing) = match upstream {
        Ok(up) => {
            let timing = up.timing.clone();
            match handshake(up).await {
                Ok(sender) => (Some(sender), None, Some(timing)),
                Err(message) => (None, Some(message), Some(timing)),
            }
        }
        Err(failure) => (None, Some(failure.message), Some(failure.timing)),
    };
    let conn = Arc::new(MitmConn {
        ctx,
        target,
        dial,
        info,
        failure,
        client_h2,
        slot: tokio::sync::Mutex::new(Slot { sender, timing }),
        requests: AtomicU32::new(0),
    });

    let counters = Arc::new(Counters::default());
    let io = TokioIo::new(Counting::new(client, counters.clone()));
    let service = {
        let conn = conn.clone();
        service_fn(move |req| conn.clone().handle(req))
    };
    if client_h2 {
        // Server push is never initiated, and extended CONNECT (RFC 8441,
        // ws-over-h2) is not advertised: both are hyper's defaults.
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(io, service)
            .await;
    } else {
        let _ = hyper::server::conn::http1::Builder::new()
            .preserve_header_case(true)
            .half_close(false)
            .serve_connection(io, service)
            .with_upgrades()
            .await;
    }

    if conn.requests.load(Ordering::Relaxed) > 0 {
        return;
    }
    if let Some(failure) = &conn.failure {
        // The client never asked for anything, so no request flow carries
        // the upstream failure; record it on the CONNECT.
        if let Some(engine) = conn.ctx.engine.upgrade()
            && let Some(recorder) =
                record_connect(&engine, &conn.target, TlsStatus::Intercepted, Some(conn.info.clone()))
        {
            recorder.fail(failure.clone());
        }
    } else if counters.read() == 0 && handshake_done.elapsed() < PINNED_CLOSE_WINDOW {
        record_pinned(
            &conn.ctx,
            &conn.target,
            conn.info.clone(),
            "The client closed the connection right after the TLS handshake, without sending a request.",
        );
    }
}

/// A client that refused our handshake: likely pinned if it rejected the
/// certificate, otherwise a plain failure.
fn record_rejection(ctx: &ProxyCtx, target: &Target, info: TlsInfo, err: &std::io::Error) {
    let alert = err
        .get_ref()
        .and_then(|e| e.downcast_ref::<rustls::Error>())
        .and_then(|e| match e {
            rustls::Error::AlertReceived(alert) => Some(*alert),
            _ => None,
        });
    let reason = match alert {
        Some(AlertDescription::UnknownCA) => "It sent an unknown_ca alert.".to_owned(),
        Some(AlertDescription::BadCertificate) => "It sent a bad_certificate alert.".to_owned(),
        Some(AlertDescription::CertificateUnknown) => "It sent a certificate_unknown alert.".to_owned(),
        None if matches!(
            err.kind(),
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
        ) =>
        {
            "It closed the connection during the TLS handshake.".to_owned()
        }
        _ => {
            if let Some(engine) = ctx.engine.upgrade()
                && let Some(recorder) = record_connect(&engine, target, TlsStatus::Intercepted, Some(info))
            {
                recorder.fail(format!("TLS handshake with the client failed: {err}"));
            }
            return;
        }
    };
    record_pinned(ctx, target, info, &reason);
}

fn record_pinned(ctx: &ProxyCtx, target: &Target, info: TlsInfo, reason: &str) {
    let Some(engine) = ctx.engine.upgrade() else {
        return;
    };
    if let Some(recorder) = record_connect(&engine, target, TlsStatus::LikelyPinned, Some(info)) {
        recorder.fail(format!(
            "The client rejected the Proxymate certificate. {reason} Either it doesn't trust the Proxymate CA \
             yet, or it pins its certificates. Add {} to the SSL bypass list to let its traffic through \
             without decrypting it.",
            target.host
        ));
    }
}

/// Records the CONNECT itself (tunnels and connection-level failures).
fn record_connect(
    engine: &Arc<Engine>,
    target: &Target,
    tls: TlsStatus,
    tls_info: Option<TlsInfo>,
) -> Option<FlowRecorder> {
    let summary = FlowSummary {
        id: 0,
        state: FlowState::Active,
        scheme: Scheme::Https,
        method: "CONNECT".into(),
        host: target.host.as_str().into(),
        port: target.port,
        path: String::new(),
        query: String::new(),
        status: None,
        content_kind: ContentKind::None,
        client: None,
        started_at_ms: now_ms(),
        duration_ms: None,
        req_size: 0,
        resp_size: 0,
        edited: false,
        http_version: target.head.version,
        tls,
        graphql_op: None,
    };
    let detail = FlowDetail {
        id: 0,
        request: target.head.clone(),
        response: None,
        request_body: BodyInfo::default(),
        response_body: BodyInfo::default(),
        error: None,
        tls: tls_info,
        timing: Timing::default(),
    };
    engine.begin_flow(summary, detail)
}

fn connect_response_head() -> ResponseHead {
    ResponseHead {
        status: 200,
        reason: "Connection Established".into(),
        version: HttpVersion::Http11,
        headers: Vec::new(),
    }
}

/// Plain HTTP inside a CONNECT (e.g. `curl -p http://…`): served as HTTP
/// and forwarded like any absolute-form request.
async fn serve_plain<S>(ctx: Arc<ProxyCtx>, io: S, target: Target)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authority = if target.port == 80 {
        target.host.clone()
    } else {
        format!("{}:{}", bracket(&target.host), target.port)
    };
    let service = service_fn(move |mut req: Request<Incoming>| {
        let ctx = ctx.clone();
        let authority = authority.clone();
        async move {
            let pq = req.uri().path_and_query().map_or("/", PathAndQuery::as_str);
            match format!("http://{authority}{pq}").parse::<Uri>() {
                Ok(uri) => {
                    *req.uri_mut() = uri;
                    Ok::<_, Infallible>(forward(&ctx, req).await)
                }
                Err(_) => Ok(text_response(
                    StatusCode::BAD_REQUEST,
                    "Proxymate: bad request target\n",
                )),
            }
        }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .preserve_header_case(true)
        .serve_connection(TokioIo::new(io), service)
        .await;
}

fn bracket(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// How to (re)connect to the upstream of one intercepted connection.
struct Dial {
    tls: Arc<TlsContext>,
    host: String,
    port: u16,
    sni: Option<String>,
    offered: Vec<Vec<u8>>,
    insecure: bool,
}

struct Upstream {
    stream: ClientTls<TcpStream>,
    alpn: Option<Vec<u8>>,
    version: String,
    cipher: String,
    chain: Vec<pm_model::CertSummary>,
    timing: Timing,
}

struct DialFailure {
    message: String,
    chain: Vec<pm_model::CertSummary>,
    timing: Timing,
}

impl Dial {
    async fn connect(&self) -> Result<Upstream, Box<DialFailure>> {
        let (tcp, mut timing) = dial_tcp(&self.host, self.port).await.map_err(|message| {
            Box::new(DialFailure {
                message,
                chain: Vec::new(),
                timing: Timing::default(),
            })
        })?;
        let (config, recorder) = self.tls.client_config(self.offered.clone(), self.insecure);
        let name = self.sni.as_deref().unwrap_or(&self.host);
        let server_name = match ServerName::try_from(name.to_owned()) {
            Ok(name) => name,
            Err(_) => {
                return Err(Box::new(DialFailure {
                    message: format!("{name:?} is not a valid TLS server name"),
                    chain: Vec::new(),
                    timing,
                }));
            }
        };
        let started = Instant::now();
        let handshake = TlsConnector::from(config).connect(server_name, tcp);
        let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await;
        timing.tls_ms = Some(ms(started.elapsed()));
        let chain = || recorder.chain().iter().map(|c| summarize(c)).collect();
        let stream = match result {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                return Err(Box::new(DialFailure {
                    message: upstream_tls_error(&self.host, &err),
                    chain: chain(),
                    timing,
                }));
            }
            Err(_) => {
                return Err(Box::new(DialFailure {
                    message: format!("The TLS handshake with {} timed out.", self.host),
                    chain: chain(),
                    timing,
                }));
            }
        };
        let conn = stream.get_ref().1;
        Ok(Upstream {
            alpn: conn.alpn_protocol().map(<[u8]>::to_vec),
            version: conn.protocol_version().map(protocol_name).unwrap_or_default(),
            cipher: conn
                .negotiated_cipher_suite()
                .map(|s| format!("{:?}", s.suite()))
                .unwrap_or_default(),
            chain: chain(),
            stream,
            timing,
        })
    }
}

/// DNS and TCP connect, timed separately.
async fn dial_tcp(host: &str, port: u16) -> Result<(TcpStream, Timing), String> {
    let mut timing = Timing::default();
    let started = Instant::now();
    let addrs: Vec<std::net::SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![(ip, port).into()],
        Err(_) => {
            let lookup = tokio::net::lookup_host((host, port));
            let addrs = match tokio::time::timeout(CONNECT_TIMEOUT, lookup).await {
                Ok(Ok(addrs)) => addrs.collect(),
                Ok(Err(err)) => return Err(format!("can't resolve {host}: {err}")),
                Err(_) => return Err(format!("resolving {host} timed out")),
            };
            timing.dns_ms = Some(ms(started.elapsed()));
            addrs
        }
    };
    let started = Instant::now();
    let tcp = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addrs[..])).await {
        Ok(Ok(tcp)) => tcp,
        Ok(Err(err)) => return Err(format!("can't reach {host}:{port}: {err}")),
        Err(_) => return Err(format!("connecting to {host}:{port} timed out")),
    };
    timing.connect_ms = Some(ms(started.elapsed()));
    let _ = tcp.set_nodelay(true);
    Ok((tcp, timing))
}

/// A readable reason for an upstream handshake failure.
fn upstream_tls_error(host: &str, err: &std::io::Error) -> String {
    use rustls::{CertificateError, Error as TlsError};
    let Some(tls) = err.get_ref().and_then(|e| e.downcast_ref::<TlsError>()) else {
        return format!("The TLS handshake with {host} failed: {}", error_chain(err));
    };
    let untrusted = |detail: &str| {
        format!(
            "{host}'s certificate isn't trusted: it is self-signed or issued by an unknown CA{detail}. If you \
             trust this server, allow insecure upstream for {host}."
        )
    };
    match tls {
        TlsError::InvalidCertificate(CertificateError::UnknownIssuer) => untrusted(""),
        TlsError::InvalidCertificate(CertificateError::Expired | CertificateError::ExpiredContext { .. }) => {
            format!("{host}'s certificate has expired.")
        }
        TlsError::InvalidCertificate(
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
        ) => format!("{host}'s certificate isn't valid for that name."),
        // The macOS and Windows verifiers report trust failures as OS errors.
        TlsError::InvalidCertificate(CertificateError::Other(other)) => {
            let detail = other.to_string();
            if detail.contains("not trusted") || detail.contains("untrusted") {
                untrusted(&format!(" ({detail})"))
            } else {
                format!("{host}'s certificate was rejected: {detail}.")
            }
        }
        TlsError::InvalidCertificate(other) => format!("{host}'s certificate was rejected: {other:?}."),
        TlsError::PeerIncompatible(_) => format!(
            "{host} only offers TLS versions or ciphers Proxymate doesn't support (TLS 1.0 and 1.1 are not \
             supported): {tls}."
        ),
        other => format!("The TLS handshake with {host} failed: {other}."),
    }
}

fn protocol_name(v: rustls::ProtocolVersion) -> String {
    match v {
        rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3".into(),
        rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2".into(),
        other => format!("{other:?}"),
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// An upstream HTTP connection: h1 or h2, as negotiated.
enum Sender {
    H1(client_h1::SendRequest<UpstreamBody>),
    H2(client_h2::SendRequest<UpstreamBody>),
}

impl Sender {
    fn is_closed(&self) -> bool {
        match self {
            Sender::H1(s) => s.is_closed(),
            Sender::H2(s) => s.is_closed(),
        }
    }
}

/// Starts HTTP over a fresh upstream TLS connection.
async fn handshake(up: Upstream) -> Result<Sender, String> {
    let io = TokioIo::new(up.stream);
    if up.alpn.as_deref() == Some(H2) {
        let (sender, conn) = client_h2::Builder::new(TokioExecutor::new())
            .handshake(io)
            .await
            .map_err(|e| format!("HTTP/2 handshake with the upstream failed: {}", error_chain(&e)))?;
        tokio::spawn(conn);
        Ok(Sender::H2(sender))
    } else {
        let (sender, conn) = client_h1::Builder::new()
            .preserve_header_case(true)
            .handshake(io)
            .await
            .map_err(|e| format!("HTTP/1.1 handshake with the upstream failed: {}", error_chain(&e)))?;
        tokio::spawn(conn);
        Ok(Sender::H1(sender))
    }
}

/// One intercepted client connection and its upstream.
struct MitmConn {
    ctx: Arc<ProxyCtx>,
    target: Target,
    dial: Dial,
    info: TlsInfo,
    /// Why there is no upstream; every request gets the 502 page.
    failure: Option<String>,
    client_h2: bool,
    slot: tokio::sync::Mutex<Slot>,
    requests: AtomicU32,
}

struct Slot {
    sender: Option<Sender>,
    /// Setup timing of the current connection, until a flow claims it.
    timing: Option<Timing>,
}

impl MitmConn {
    async fn handle(self: Arc<Self>, req: Request<Incoming>) -> Result<Response<ProxyBody>, Infallible> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let authority = request_authority(&req, &self.target);
        let recorder = self.ctx.engine.upgrade().and_then(|engine| {
            let pq = req.uri().path_and_query().map_or("/", PathAndQuery::as_str);
            let origin = Origin {
                scheme: Scheme::Https,
                host: authority
                    .host()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned(),
                port: authority.port_u16().unwrap_or(443),
                url: format!("https://{authority}{pq}"),
                tls: TlsStatus::Intercepted,
                tls_info: Some(self.info.clone()),
            };
            record_request(&engine, &req, origin)
        });
        if let Some(failure) = &self.failure {
            if let Some(recorder) = &recorder {
                let mut slot = self.slot.lock().await;
                if let Some(timing) = slot.timing.take() {
                    recorder.connection_timing(&timing);
                }
                drop(slot);
                recorder.fail(failure.clone());
            }
            return Ok(error_page(&self.target.host, failure));
        }

        let (mut parts, body) = req.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        let pq = parts
            .uri
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"));
        let body = TeeBody::new(body, recorder.clone(), BodyPart::Request);
        let upstream = match self.send(parts, pq, authority, body, recorder.as_ref()).await {
            Ok(response) => response,
            Err(message) => {
                if let Some(recorder) = &recorder {
                    recorder.fail(message.clone());
                }
                return Ok(error_page(&self.target.host, &message));
            }
        };
        if let Some(recorder) = &recorder {
            recorder.response(response_head(&upstream), content_kind(upstream.headers()));
        }
        let (mut parts, body) = upstream.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        let body = TeeBody::new(body, recorder, BodyPart::Response);
        Ok(Response::from_parts(parts, body.map_err(BoxError::from).boxed_unsync()))
    }

    /// Sends one request upstream, reconnecting if the upstream closed the
    /// connection since the last one.
    async fn send(
        &self,
        mut parts: http::request::Parts,
        pq: PathAndQuery,
        authority: Authority,
        body: UpstreamBody,
        recorder: Option<&FlowRecorder>,
    ) -> Result<Response<Incoming>, String> {
        let mut slot = self.slot.lock().await;
        if slot.sender.as_ref().is_none_or(Sender::is_closed) {
            let up = self.dial.connect().await.map_err(|f| f.message)?;
            if (up.alpn.as_deref() == Some(H2)) != self.client_h2 {
                return Err("The upstream switched HTTP versions when Proxymate reconnected.".into());
            }
            slot.timing = Some(up.timing.clone());
            slot.sender = Some(handshake(up).await?);
        }
        let timing = slot.timing.take().unwrap_or(Timing {
            reused_connection: true,
            ..Timing::default()
        });
        if let Some(recorder) = recorder {
            recorder.connection_timing(&timing);
        }
        let failed = |e: hyper::Error| format!("The upstream request failed: {}", error_chain(&e));
        match slot.sender.as_mut() {
            Some(Sender::H1(sender)) => {
                parts.uri = Uri::from(pq);
                if !parts.headers.contains_key(header::HOST)
                    && let Ok(host) = HeaderValue::from_str(authority.as_str())
                {
                    parts.headers.insert(header::HOST, host);
                }
                sender.ready().await.map_err(failed)?;
                let response = sender.send_request(Request::from_parts(parts, body));
                drop(slot);
                if let Some(recorder) = recorder {
                    recorder.request_sent();
                }
                response.await.map_err(failed)
            }
            Some(Sender::H2(sender)) => {
                let mut sender = sender.clone();
                drop(slot);
                parts.uri = Uri::builder()
                    .scheme("https")
                    .authority(authority)
                    .path_and_query(pq)
                    .build()
                    .map_err(|e| e.to_string())?;
                // :authority carries it in h2.
                parts.headers.remove(header::HOST);
                sender.ready().await.map_err(failed)?;
                if let Some(recorder) = recorder {
                    recorder.request_sent();
                }
                sender
                    .send_request(Request::from_parts(parts, body))
                    .await
                    .map_err(failed)
            }
            None => Err("no upstream connection".into()),
        }
    }
}

/// The request's authority: `:authority` in h2, `Host` in h1, else the
/// CONNECT target.
fn request_authority(req: &Request<Incoming>, target: &Target) -> Authority {
    req.uri()
        .authority()
        .cloned()
        .or_else(|| {
            req.headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.parse().ok())
        })
        .or_else(|| {
            let host = bracket(&target.host);
            let authority = if target.port == 443 {
                host
            } else {
                format!("{host}:{}", target.port)
            };
            authority.parse().ok()
        })
        .unwrap_or_else(|| Authority::from_static("unknown"))
}

/// The 502 page for an upstream that couldn't be reached securely.
fn error_page(host: &str, message: &str) -> Response<ProxyBody> {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Proxymate: can't reach {host}</title>\
         <style>body{{font:15px -apple-system,system-ui,sans-serif;max-width:640px;margin:12vh auto;padding:0 24px;\
         color:#222}}h1{{font-size:20px}}p{{line-height:1.5}}code{{background:#f2f2f2;padding:2px 4px;\
         border-radius:3px}}</style></head><body><h1>Proxymate couldn't connect securely to <code>{host}</code></h1>\
         <p>{message}</p><p>This page comes from Proxymate, the HTTPS debugging proxy you are using, not from \
         {host}.</p></body></html>",
        host = escape(host),
        message = escape(message),
    );
    let mut response = Response::new(
        Full::new(Bytes::from(html))
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_page_escapes_its_input() {
        let response = error_page("<host>", "a & b");
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn upstream_errors_read_well() {
        let err = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        let message = upstream_tls_error("self-signed.test", &err);
        assert!(message.contains("allow insecure upstream"), "{message}");
    }
}
