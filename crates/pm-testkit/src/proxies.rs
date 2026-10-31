//! Proxies to chain Proxymate to (M18): an HTTP proxy that tunnels
//! `CONNECT` and forwards absolute-form requests, optionally behind Basic
//! auth, and a SOCKS5 server, optionally with username/password auth. Both
//! keep a log of what they were asked for. Plus a SOCKS5 client, to talk to
//! Proxymate's own SOCKS5 listener the way `curl --socks5-hostname` does.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use bytes::Bytes;
use http::{Request, Response, StatusCode, header};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// One request an [`HttpProxy`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRequest {
    /// `CONNECT`, `GET`, …
    pub method: String,
    /// `host:port` for CONNECT, the absolute URL otherwise.
    pub target: String,
    /// The `Proxy-Authorization` header, if sent.
    pub authorization: Option<String>,
    /// Whether it got past the auth check.
    pub allowed: bool,
}

/// An HTTP/1.1 forward proxy on a loopback ephemeral port.
pub struct HttpProxy {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<ProxyRequest>>>,
    task: JoinHandle<()>,
}

impl HttpProxy {
    /// A proxy that lets everyone through.
    pub async fn start() -> HttpProxy {
        Self::serve(None).await
    }

    /// A proxy that answers 407 unless `Proxy-Authorization` is Basic
    /// `user:password`.
    pub async fn with_basic_auth(user: &str, password: &str) -> HttpProxy {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        Self::serve(Some(format!("Basic {token}"))).await
    }

    async fn serve(expected: Option<String>) -> HttpProxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind HTTP proxy");
        let addr = listener.local_addr().expect("HTTP proxy addr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = (Arc::new(expected), log.clone());
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (expected, log) = shared.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req| handle(expected.clone(), log.clone(), req));
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades()
                        .await;
                });
            }
        });
        HttpProxy { addr, log, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Everything asked so far, oldest first.
    pub fn requests(&self) -> Vec<ProxyRequest> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn empty(status: StatusCode) -> Response<ProxyBody> {
    let mut response = Response::new(Empty::new().map_err(|never| match never {}).boxed());
    *response.status_mut() = status;
    response
}

fn text(status: StatusCode, body: &str) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(Bytes::from(body.to_owned()))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
}

async fn handle(
    expected: Arc<Option<String>>,
    log: Arc<Mutex<Vec<ProxyRequest>>>,
    mut req: Request<Incoming>,
) -> Result<Response<ProxyBody>, hyper::Error> {
    let authorization = req
        .headers()
        .get(header::PROXY_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let allowed = expected
        .as_ref()
        .as_ref()
        .is_none_or(|e| authorization.as_ref() == Some(e));
    let connect = req.method() == http::Method::CONNECT;
    let target = if connect {
        req.uri().authority().map(|a| a.to_string()).unwrap_or_default()
    } else {
        req.uri().to_string()
    };
    log.lock().unwrap().push(ProxyRequest {
        method: req.method().to_string(),
        target: target.clone(),
        authorization,
        allowed,
    });
    if !allowed {
        let mut response = empty(StatusCode::PROXY_AUTHENTICATION_REQUIRED);
        response.headers_mut().insert(
            header::PROXY_AUTHENTICATE,
            header::HeaderValue::from_static("Basic realm=\"pm-testkit\""),
        );
        return Ok(response);
    }
    if connect {
        let Ok(mut upstream) = TcpStream::connect(&target).await else {
            return Ok(empty(StatusCode::BAD_GATEWAY));
        };
        tokio::spawn(async move {
            if let Ok(upgraded) = hyper::upgrade::on(req).await {
                let mut client = TokioIo::new(upgraded);
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            }
        });
        return Ok(empty(StatusCode::OK));
    }
    // Absolute form: one upstream connection per request is plenty here.
    let Some(authority) = req.uri().authority().cloned() else {
        return Ok(text(StatusCode::BAD_REQUEST, "absolute form expected\n"));
    };
    let host_port = if authority.port().is_some() {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    let Ok(stream) = TcpStream::connect(host_port).await else {
        return Ok(text(StatusCode::BAD_GATEWAY, "can't reach the upstream\n"));
    };
    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await else {
        return Ok(text(StatusCode::BAD_GATEWAY, "upstream handshake failed\n"));
    };
    tokio::spawn(conn);
    let pq = req.uri().path_and_query().map_or("/", |pq| pq.as_str()).to_owned();
    *req.uri_mut() = pq.parse().expect("path");
    req.headers_mut().remove(header::PROXY_AUTHORIZATION);
    req.headers_mut().remove("proxy-connection");
    if !req.headers().contains_key(header::HOST) {
        let host = header::HeaderValue::from_str(authority.as_str()).expect("host");
        req.headers_mut().insert(header::HOST, host);
    }
    let response = sender.send_request(req).await?;
    Ok(response.map(BodyExt::boxed))
}

/// A SOCKS5 server (RFC 1928) on a loopback ephemeral port: CONNECT only,
/// resolving names itself.
pub struct Socks5Server {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Socks5Server {
    pub async fn start() -> Socks5Server {
        Self::serve(None).await
    }

    /// Requires username/password auth (RFC 1929).
    pub async fn with_auth(user: &str, password: &str) -> Socks5Server {
        Self::serve(Some((user.to_owned(), password.to_owned()))).await
    }

    async fn serve(credentials: Option<(String, String)>) -> Socks5Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind SOCKS5 server");
        let addr = listener.local_addr().expect("SOCKS5 addr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = (Arc::new(credentials), log.clone());
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (credentials, log) = shared.clone();
                tokio::spawn(async move {
                    let _ = socks_session(stream, &credentials, &log).await;
                });
            }
        });
        Socks5Server { addr, log, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The targets asked for, as `host:port` with names as sent (so a
    /// remote resolution shows the name, not an address).
    pub fn targets(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Socks5Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn socks_session(
    mut stream: TcpStream,
    credentials: &Option<(String, String)>,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    let mut methods = vec![0u8; usize::from(head[1])];
    stream.read_exact(&mut methods).await?;
    let wanted = if credentials.is_some() { 0x02 } else { 0x00 };
    if !methods.contains(&wanted) {
        stream.write_all(&[0x05, 0xFF]).await?;
        return Ok(());
    }
    stream.write_all(&[0x05, wanted]).await?;
    if let Some((user, password)) = credentials {
        let mut ver_len = [0u8; 2];
        stream.read_exact(&mut ver_len).await?;
        let mut got_user = vec![0u8; usize::from(ver_len[1])];
        stream.read_exact(&mut got_user).await?;
        let mut len = [0u8; 1];
        stream.read_exact(&mut len).await?;
        let mut got_password = vec![0u8; usize::from(len[0])];
        stream.read_exact(&mut got_password).await?;
        let ok = got_user == user.as_bytes() && got_password == password.as_bytes();
        stream.write_all(&[0x01, if ok { 0x00 } else { 0x01 }]).await?;
        if !ok {
            return Ok(());
        }
    }
    let mut request = [0u8; 4];
    stream.read_exact(&mut request).await?;
    let host = match request[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        0x04 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            Ipv6Addr::from(ip).to_string()
        }
        _ => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            stream.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    log.lock().unwrap().push(if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    });
    let upstream = match host.parse::<IpAddr>() {
        Ok(ip) => TcpStream::connect((ip, port)).await,
        Err(_) => TcpStream::connect((host.as_str(), port)).await,
    };
    let Ok(mut upstream) = upstream else {
        stream.write_all(&[0x05, 0x05, 0, 0x01, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    };
    stream.write_all(&[0x05, 0x00, 0, 0x01, 0, 0, 0, 0, 0, 0]).await?;
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
    Ok(())
}

/// Opens a SOCKS5 connection to `host:port` through `proxy`, sending a
/// name as a name (`socks5h://`, `curl --socks5-hostname`). The error says
/// which reply code the server sent.
pub async fn socks5_connect(proxy: SocketAddr, host: &str, port: u16) -> std::io::Result<TcpStream> {
    let mut stream = TcpStream::connect(proxy).await?;
    stream.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice != [0x05, 0x00] {
        return Err(std::io::Error::other(format!("no acceptable method: {choice:?}")));
    }
    let mut request = vec![0x05, 0x01, 0x00];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(0x01);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(0x04);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.push(0x03);
            request.push(host.len() as u8);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).await?;
    if reply[1] != 0x00 {
        return Err(std::io::Error::other(format!("SOCKS5 reply {:#04x}", reply[1])));
    }
    Ok(stream)
}
