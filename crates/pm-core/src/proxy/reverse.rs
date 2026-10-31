//! Reverse proxy listeners (M18). Each enabled `ReverseProxy` listens on
//! loopback and sends every request to its target URL, recorded like
//! proxied traffic with the target as the flow's URL and "Reverse :<port>"
//! as its client. Rules apply as usual, WebSockets are relayed, and with
//! `tls` the listener serves a Proxymate leaf for `localhost` (or the
//! address the client connected to, when it sends no SNI).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use http::header::{self, HeaderValue};
use http::{Request, StatusCode, Uri};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pm_model::ReverseProxy;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio_rustls::LazyConfigAcceptor;

use super::{Forward, ProxyCtx, Reset, forward, text_response};
use crate::Engine;
use crate::client::{ClientApp, ClientKind, PeerClient};
use crate::tls::{H2, HTTP11};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// A running reverse proxy listener.
pub(crate) struct ReverseHandle {
    pub addr: SocketAddr,
    stop: watch::Sender<()>,
    closed: oneshot::Receiver<()>,
}

impl ReverseHandle {
    pub async fn stop(self) {
        drop(self.stop);
        let _ = self.closed.await;
    }
}

/// What every connection of one listener shares.
struct Reverse {
    ctx: Arc<ProxyCtx>,
    /// `scheme://authority` of the target.
    origin: String,
    authority: String,
    /// The target's path, without a trailing slash (`""` for none).
    prefix: String,
    preserve_host: bool,
    tls: bool,
    peer: Arc<PeerClient>,
}

/// Binds `config.listen_port` on loopback and serves it.
pub(crate) async fn start(engine: &Arc<Engine>, config: &ReverseProxy) -> Result<ReverseHandle, String> {
    let url: url::Url = config
        .target_url
        .trim()
        .parse()
        .map_err(|_| format!("{:?} isn't a URL", config.target_url))?;
    let host = url.host_str().unwrap_or_default();
    let port = url.port_or_known_default().unwrap_or(80);
    let default = if url.scheme() == "https" { 443 } else { 80 };
    let authority = if port == default {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    };
    let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), config.listen_port))
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AddrInUse => format!("port {} is in use", config.listen_port),
            _ => format!("can't listen on port {}: {e}", config.listen_port),
        })?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let name = format!("Reverse :{}", addr.port());
    let app = Arc::new(ClientApp {
        name: name.as_str().into(),
        bundle_id: None,
        bundle_path: None,
        kind: ClientKind::Local,
    });
    let shared = Arc::new(Reverse {
        ctx: Arc::new(ProxyCtx {
            engine: Arc::downgrade(engine),
            addr,
        }),
        origin: format!("{}://{authority}", url.scheme()),
        authority,
        prefix: url.path().trim_end_matches('/').to_owned(),
        preserve_host: config.preserve_host,
        tls: config.tls,
        peer: PeerClient::known(Some(app)),
    });
    let (stop_tx, mut stop_rx) = watch::channel(());
    let (closed_tx, closed) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(serve(shared.clone(), stream));
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                _ = stop_rx.changed() => break,
            }
        }
        drop(listener);
        let _ = closed_tx.send(());
    });
    Ok(ReverseHandle {
        addr,
        stop: stop_tx,
        closed,
    })
}

async fn serve(shared: Arc<Reverse>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    if !shared.tls {
        return serve_http(shared, stream, false).await;
    }
    let Some(engine) = shared.ctx.engine.upgrade() else {
        return;
    };
    let tls = engine.tls();
    drop(engine);
    let local = stream.local_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    let acceptor = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), stream);
    let Ok(Ok(start)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor).await else {
        return;
    };
    let (sni, h2) = {
        let hello = start.client_hello();
        let h2 = hello.alpn().is_some_and(|mut ps| ps.any(|p| p == H2));
        let http11 = hello.alpn().is_some_and(|mut ps| ps.any(|p| p == HTTP11));
        (hello.server_name().map(str::to_owned), (h2, http11))
    };
    let alpn = match h2 {
        (true, _) => Some(H2),
        (false, true) => Some(HTTP11),
        _ => None,
    };
    let Ok(config) = tls.server_config(alpn, sni.as_deref(), &local) else {
        return;
    };
    if let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, start.into_stream(config)).await {
        let h2 = stream.get_ref().1.alpn_protocol() == Some(H2);
        serve_http(shared, stream, h2).await;
    }
}

async fn serve_http<S>(shared: Arc<Reverse>, io: S, h2: bool)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = {
        let shared = shared.clone();
        service_fn(move |req| handle(shared.clone(), req))
    };
    if h2 {
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(io), service)
            .await;
    } else {
        let _ = hyper::server::conn::http1::Builder::new()
            .preserve_header_case(true)
            .half_close(false)
            .serve_connection(TokioIo::new(io), service)
            .with_upgrades()
            .await;
    }
}

async fn handle(shared: Arc<Reverse>, mut req: Request<Incoming>) -> Result<http::Response<super::ProxyBody>, Reset> {
    let pq = req.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let url = format!("{}{}{pq}", shared.origin, shared.prefix);
    let Ok(uri) = url.parse::<Uri>() else {
        return Ok(text_response(
            StatusCode::BAD_REQUEST,
            "Proxymate: bad request target\n",
        ));
    };
    // h2 clients send `:authority`; the upstream gets a `Host` either way.
    let client_host = req.headers().get(header::HOST).cloned().or_else(|| {
        req.uri()
            .authority()
            .and_then(|a| HeaderValue::from_str(a.as_str()).ok())
    });
    let host = match (shared.preserve_host, client_host) {
        (true, Some(host)) => host,
        _ => HeaderValue::from_str(&shared.authority).unwrap_or_else(|_| HeaderValue::from_static("localhost")),
    };
    req.headers_mut().insert(header::HOST, host);
    *req.uri_mut() = uri;
    let how = Forward {
        reverse_port: Some(shared.ctx.addr.port()),
        client_tls: shared.tls,
    };
    forward(&shared.ctx, &shared.peer, req, how).await
}
