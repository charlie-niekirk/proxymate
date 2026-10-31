//! The one dialer: every upstream TCP connection Proxymate makes goes
//! through [`dial`], whether for the MITM, a passthrough tunnel, the
//! plain-HTTP pool ([`Connector`]), `Engine::send`, WebSockets or a rule
//! that sends a request to another origin. The trust probe and the
//! onboarding server answer by themselves and never dial.
//!
//! In order: a DNS override replaces the target's address (SNI and `Host`
//! stay as they were), then Settings › Network picks the hop (direct, an
//! HTTP proxy or a SOCKS5 proxy), then the loop guard refuses a hop that
//! is one of our own listeners. Client certificates are added by the TLS
//! layer above (`mitm::Dial`). The route taken is returned as a [`Via`]
//! for the flow's detail.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use base64::Engine as _;
use http::{HeaderValue, Uri};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use pm_model::{Timing, Via};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::Engine;
use crate::network::{Hop, Server};

/// Resolving a name gives up after this long.
const DNS_TIMEOUT: Duration = Duration::from_secs(15);
/// An upstream proxy's answer to CONNECT or the SOCKS handshake.
const PROXY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest CONNECT response head read from an upstream proxy.
const MAX_PROXY_HEAD: usize = 16 * 1024;

/// What the connection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// A byte stream to the target: TLS, WebSockets, tunnels. HTTP
    /// upstreams get `CONNECT`.
    Tunnel,
    /// Plain HTTP requests: HTTP upstreams get them in absolute form.
    Http,
}

pub(crate) struct Dialed {
    pub stream: TcpStream,
    pub timing: Timing,
    pub via: Via,
    /// Plain HTTP through an HTTP proxy: requests go in absolute form, with
    /// this `Proxy-Authorization` (if the proxy has credentials).
    pub absolute_form: Option<Option<HeaderValue>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialErrorKind {
    Resolve,
    Refused,
    Unreachable,
    Timeout,
    /// The upstream proxy refused or failed, or it would loop.
    Proxy,
}

#[derive(Debug)]
pub(crate) struct DialError {
    pub kind: DialErrorKind,
    pub message: String,
}

impl DialError {
    fn new(kind: DialErrorKind, message: impl Into<String>) -> DialError {
        DialError {
            kind,
            message: message.into(),
        }
    }

    /// The SOCKS5 reply code (RFC 1928 §6) for a SOCKS client.
    pub fn socks_reply(&self) -> u8 {
        match self.kind {
            DialErrorKind::Resolve => 0x04,
            DialErrorKind::Refused => 0x05,
            DialErrorKind::Unreachable => 0x03,
            DialErrorKind::Timeout => 0x06,
            DialErrorKind::Proxy => 0x01,
        }
    }
}

impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DialError {}

impl From<DialError> for String {
    fn from(err: DialError) -> String {
        err.message
    }
}

/// Connects to `host:port` the way Settings › Network says.
pub(crate) async fn dial(engine: &Arc<Engine>, host: &str, port: u16, purpose: Purpose) -> Result<Dialed, DialError> {
    let compiled = engine.network().compiled();
    let connect_timeout = Duration::from_secs(engine.proxy_settings().connect_timeout_secs.into());
    let overridden = compiled.dns_override(host).map(|(ip, label)| (ip, label.to_owned()));
    let mut via = Via {
        dns_override: overridden.as_ref().map(|(_, label)| label.clone()),
        ..Via::default()
    };
    // Where the target is, as the next hop is told.
    let target = match &overridden {
        Some((ip, _)) => ip.to_string(),
        None => host.to_owned(),
    };
    let hop = engine.upstream_hop(host, purpose == Purpose::Tunnel).await;
    via.proxy = hop.label();
    let server = match &hop {
        Hop::Direct => {
            let (stream, timing, addr) = connect(&target, port, connect_timeout, overridden.is_some()).await?;
            via.addr = Some(addr.to_string());
            return Ok(Dialed {
                stream,
                timing,
                via,
                absolute_form: None,
            });
        }
        Hop::Http(server) | Hop::Socks5(server) => server.clone(),
    };
    let label = via.proxy.clone().unwrap_or_default();
    let proxy_target = compiled
        .dns_override(&server.host)
        .map_or_else(|| server.host.clone(), |(ip, _)| ip.to_string());
    let (mut stream, mut timing, addr) = connect(&proxy_target, server.port, connect_timeout, false)
        .await
        .map_err(|e| DialError::new(e.kind, format!("the upstream proxy {label}: {}", e.message)))?;
    via.addr = Some(addr.to_string());
    if engine.is_own_endpoint(addr) {
        return Err(DialError::new(
            DialErrorKind::Proxy,
            format!(
                "the upstream proxy {label} is Proxymate itself, which would loop. Change it in Settings › Network."
            ),
        ));
    }
    let credentials = server.credentials(engine.network().secrets());
    let started = Instant::now();
    let absolute_form = match (&hop, purpose) {
        (Hop::Http(_), Purpose::Http) => Some(credentials.map(|(u, p)| basic(&u, &p))),
        (Hop::Http(_), Purpose::Tunnel) => {
            let auth = credentials.map(|(u, p)| basic(&u, &p));
            let handshake = http_connect(&mut stream, &target, port, auth.as_ref(), &label);
            within(handshake, &label).await?;
            None
        }
        (Hop::Socks5(_), _) => {
            let handshake = socks5_connect(&mut stream, &target, port, credentials.as_ref(), &label);
            within(handshake, &label).await?;
            None
        }
        (Hop::Direct, _) => unreachable!("handled above"),
    };
    // The proxy's handshake is part of connecting.
    timing.connect_ms = Some(timing.connect_ms.unwrap_or_default() + ms(started.elapsed()));
    Ok(Dialed {
        stream,
        timing,
        via,
        absolute_form,
    })
}

async fn within(handshake: impl Future<Output = Result<(), DialError>>, label: &str) -> Result<(), DialError> {
    tokio::time::timeout(PROXY_HANDSHAKE_TIMEOUT, handshake)
        .await
        .unwrap_or_else(|_| {
            Err(DialError::new(
                DialErrorKind::Timeout,
                format!("the upstream proxy {label} didn't answer in time"),
            ))
        })
}

fn basic(user: &str, password: &str) -> HeaderValue {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    HeaderValue::from_str(&format!("Basic {token}")).unwrap_or_else(|_| HeaderValue::from_static("Basic"))
}

/// DNS (unless `host` is a literal) and TCP connect, timed separately.
async fn connect(
    host: &str,
    port: u16,
    timeout: Duration,
    overridden: bool,
) -> Result<(TcpStream, Timing, SocketAddr), DialError> {
    let mut timing = Timing::default();
    let started = Instant::now();
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match literal.parse::<IpAddr>() {
        Ok(ip) => vec![(ip, port).into()],
        Err(_) => {
            let lookup = tokio::net::lookup_host((host, port));
            let addrs: Vec<SocketAddr> = match tokio::time::timeout(DNS_TIMEOUT, lookup).await {
                Ok(Ok(addrs)) => addrs.collect(),
                Ok(Err(err)) => {
                    return Err(DialError::new(
                        DialErrorKind::Resolve,
                        format!("can't resolve {host}: {err}"),
                    ));
                }
                Err(_) => {
                    return Err(DialError::new(
                        DialErrorKind::Timeout,
                        format!("resolving {host} timed out"),
                    ));
                }
            };
            timing.dns_ms = Some(ms(started.elapsed()));
            addrs
        }
    };
    if addrs.is_empty() {
        return Err(DialError::new(
            DialErrorKind::Resolve,
            format!("{host} has no addresses"),
        ));
    }
    let shown = if overridden {
        format!("{host}:{port} (DNS override)")
    } else {
        format!("{host}:{port}")
    };
    let started = Instant::now();
    let tcp = match tokio::time::timeout(timeout, TcpStream::connect(&addrs[..])).await {
        Ok(Ok(tcp)) => tcp,
        Ok(Err(err)) => {
            let kind = match err.kind() {
                io::ErrorKind::ConnectionRefused => DialErrorKind::Refused,
                io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => DialErrorKind::Unreachable,
                _ => DialErrorKind::Refused,
            };
            return Err(DialError::new(kind, format!("can't reach {shown}: {err}")));
        }
        Err(_) => {
            return Err(DialError::new(
                DialErrorKind::Timeout,
                format!("connecting to {shown} timed out"),
            ));
        }
    };
    timing.connect_ms = Some(ms(started.elapsed()));
    let _ = tcp.set_nodelay(true);
    let addr = tcp.peer_addr().unwrap_or(addrs[0]);
    Ok((tcp, timing, addr))
}

fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// `CONNECT host:port` through an HTTP proxy. The response head is read a
/// byte at a time so nothing after it is consumed.
async fn http_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    auth: Option<&HeaderValue>,
    label: &str,
) -> Result<(), DialError> {
    let target = authority(host, port);
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = auth.and_then(|a| a.to_str().ok()) {
        request.push_str("Proxy-Authorization: ");
        request.push_str(auth);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    let failed = |e: io::Error| DialError::new(DialErrorKind::Proxy, format!("the upstream proxy {label}: {e}"));
    stream.write_all(request.as_bytes()).await.map_err(failed)?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_PROXY_HEAD {
            return Err(DialError::new(
                DialErrorKind::Proxy,
                format!("the upstream proxy {label} sent an oversized answer to CONNECT"),
            ));
        }
        match stream.read(&mut byte).await.map_err(failed)? {
            0 => {
                return Err(DialError::new(
                    DialErrorKind::Proxy,
                    format!("the upstream proxy {label} closed the connection instead of answering CONNECT"),
                ));
            }
            _ => head.push(byte[0]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let status_line = text.lines().next().unwrap_or_default().trim().to_owned();
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    match status {
        200..=299 => Ok(()),
        407 if auth.is_some() => Err(DialError::new(
            DialErrorKind::Proxy,
            format!(
                "the upstream proxy {label} rejected the username and password (407). Check them in Settings › Network."
            ),
        )),
        407 => Err(DialError::new(
            DialErrorKind::Proxy,
            format!(
                "the upstream proxy {label} asks for a username and password (407). Add them in Settings › Network."
            ),
        )),
        _ => Err(DialError::new(
            DialErrorKind::Proxy,
            format!("the upstream proxy {label} answered CONNECT {target} with {status_line:?}"),
        )),
    }
}

/// The SOCKS5 client handshake (RFC 1928, username/password per RFC
/// 1929). Names are sent as names, so the proxy resolves them.
pub(crate) async fn socks5_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    credentials: Option<&(String, String)>,
    label: &str,
) -> Result<(), DialError> {
    let proxy_err = |m: String| DialError::new(DialErrorKind::Proxy, format!("the SOCKS5 proxy {label} {m}"));
    let io_err = |e: io::Error| proxy_err(format!("failed: {e}"));
    let methods: &[u8] = if credentials.is_some() { &[0x00, 0x02] } else { &[0x00] };
    let mut greeting = vec![0x05, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await.map_err(io_err)?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await.map_err(io_err)?;
    if choice[0] != 0x05 {
        return Err(proxy_err("doesn't speak SOCKS5".into()));
    }
    match (choice[1], credentials) {
        (0x00, _) => {}
        (0x02, Some((user, password))) => {
            if user.len() > 255 || password.len() > 255 {
                return Err(proxy_err("can't take a username or password over 255 bytes".into()));
            }
            let mut auth = vec![0x01, user.len() as u8];
            auth.extend_from_slice(user.as_bytes());
            auth.push(password.len() as u8);
            auth.extend_from_slice(password.as_bytes());
            stream.write_all(&auth).await.map_err(io_err)?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await.map_err(io_err)?;
            if status[1] != 0x00 {
                return Err(proxy_err(
                    "rejected the username and password. Check them in Settings › Network.".into(),
                ));
            }
        }
        (0xFF, None) => {
            return Err(proxy_err(
                "asks for a username and password. Add them in Settings › Network.".into(),
            ));
        }
        _ => return Err(proxy_err("offered no authentication method Proxymate supports".into())),
    }
    let mut request = vec![0x05, 0x01, 0x00];
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    match literal.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(0x01);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(0x04);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            if host.len() > 255 {
                return Err(proxy_err(format!("can't take a host name as long as {host}")));
            }
            request.push(0x03);
            request.push(host.len() as u8);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(io_err)?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await.map_err(io_err)?;
    if reply[1] != 0x00 {
        let why = match reply[1] {
            0x02 => "isn't allowed by the proxy's rules",
            0x03 => "network unreachable",
            0x04 => "host unreachable",
            0x05 => "connection refused",
            0x06 => "timed out",
            0x08 => "address type not supported",
            _ => "general failure",
        };
        return Err(proxy_err(format!(
            "couldn't connect to {}: {why}",
            authority(host, port)
        )));
    }
    // The bound address, which nobody needs.
    let skip = match reply[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(io_err)?;
            usize::from(len[0])
        }
        _ => return Err(proxy_err("sent a malformed reply".into())),
    };
    let mut rest = vec![0u8; skip + 2];
    stream.read_exact(&mut rest).await.map_err(io_err)?;
    Ok(())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// What a pooled connection tells each response it carries: its route and,
/// for the first response only, its setup timing.
#[derive(Clone)]
pub(crate) struct PoolInfo {
    pub via: Via,
    timing: Timing,
    claimed: Arc<AtomicBool>,
}

impl PoolInfo {
    /// The setup timing for the first flow on the connection; later ones
    /// see a reused connection.
    pub fn timing(&self) -> Timing {
        if self.claimed.swap(true, Ordering::Relaxed) {
            Timing {
                reused_connection: true,
                ..Timing::default()
            }
        } else {
            self.timing.clone()
        }
    }
}

/// hyper-util's connector for the plain-HTTP pool, dialling through
/// [`dial`]. hyper-util's own `HttpConnector` never consults `HTTP_PROXY`
/// or the system proxy settings, and neither does this one: once Proxymate
/// is the system proxy, using it here would send every request back to us.
#[derive(Clone)]
pub(crate) struct Connector {
    pub engine: Weak<Engine>,
}

pub(crate) struct PooledConn {
    io: TokioIo<TcpStream>,
    absolute_form: bool,
    info: PoolInfo,
}

impl tower_service::Service<Uri> for Connector {
    type Response = PooledConn;
    type Error = DialError;
    type Future = Pin<Box<dyn Future<Output = Result<PooledConn, DialError>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), DialError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let engine = self.engine.clone();
        Box::pin(async move {
            let Some(engine) = engine.upgrade() else {
                return Err(DialError::new(DialErrorKind::Proxy, "Proxymate is shutting down"));
            };
            let host = uri
                .host()
                .unwrap_or_default()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned();
            let port = uri.port_u16().unwrap_or(80);
            let dialed = dial(&engine, &host, port, Purpose::Http).await?;
            Ok(PooledConn {
                io: TokioIo::new(dialed.stream),
                absolute_form: dialed.absolute_form.is_some(),
                info: PoolInfo {
                    via: dialed.via,
                    timing: dialed.timing,
                    claimed: Arc::new(AtomicBool::new(false)),
                },
            })
        })
    }
}

impl Connection for PooledConn {
    fn connected(&self) -> Connected {
        Connected::new().proxy(self.absolute_form).extra(self.info.clone())
    }
}

impl hyper::rt::Read for PooledConn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl hyper::rt::Write for PooledConn {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// The `Proxy-Authorization` a plain-HTTP request to `host` needs, when it
/// goes through an HTTP upstream with credentials.
pub(crate) async fn proxy_authorization(engine: &Arc<Engine>, host: &str) -> Option<HeaderValue> {
    if engine.network().compiled().is_empty() {
        return None;
    }
    match engine.upstream_hop(host, false).await {
        Hop::Http(server) => credentials_header(&server, engine),
        _ => None,
    }
}

fn credentials_header(server: &Server, engine: &Engine) -> Option<HeaderValue> {
    server
        .credentials(engine.network().secrets())
        .map(|(u, p)| basic(&u, &p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn basic_auth_is_base64() {
        assert_eq!(basic("Aladdin", "open sesame"), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    }

    #[tokio::test]
    async fn socks5_client_sends_names_and_reads_the_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 4];
            s.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 2, 0, 2]);
            s.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0u8; 7];
            s.read_exact(&mut auth).await.unwrap();
            assert_eq!(&auth, b"\x01\x02me\x02pw");
            s.write_all(&[1, 0]).await.unwrap();
            let mut req = vec![0u8; 5 + 9 + 2];
            s.read_exact(&mut req).await.unwrap();
            assert_eq!(&req[..5], &[5, 1, 0, 3, 9]);
            assert_eq!(&req[5..14], b"a.example");
            assert_eq!(&req[14..], &443u16.to_be_bytes());
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
        });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let creds = ("me".to_owned(), "pw".to_owned());
        socks5_connect(&mut stream, "a.example", 443, Some(&creds), "socks5://test")
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_http_proxy_refusing_connect_reads_well() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).await;
            s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let err = http_connect(&mut stream, "example.com", 443, None, "http://p:1")
            .await
            .unwrap_err();
        assert!(err.message.contains("asks for a username and password"), "{err}");
    }
}
