//! A gRPC upstream (tonic) over TLS from a [`TestCa`]: `shop.v1.Orders`
//! from `proto/shop.proto`, with a unary call, a server stream, an error
//! status with details, gzip and zstd messages (when the client accepts
//! them) and gRPC-web on the same port (h1 or h2).
//!
//! The clients frame messages by hand and speak h2 (or h1 for gRPC-web)
//! through the proxy's CONNECT, so tests see exactly what went over the
//! wire, trailers included.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use http::{HeaderMap, Request};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use prost::Message;
use rustls::ServerConfig;
use rustls::pki_types::CertificateDer;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tonic::codec::CompressionEncoding;
use tonic::{Code, Response, Status};

use crate::client::{tls_config, tls_through};
use crate::tls::TestCa;

/// Generated from `proto/shop.proto`.
pub mod shop {
    #![allow(clippy::all)]
    tonic::include_proto!("shop.v1");
}

use shop::orders_server::{Orders, OrdersServer};
use shop::{GetOrderRequest, LineItem, ListOrdersRequest, Order};

/// The schema, for importing into an engine.
pub const SHOP_PROTO: &str = include_str!("../proto/shop.proto");

/// The `total_cents` every order has: past 2^53.
pub const TOTAL_CENTS: i64 = (1 << 60) + 7;

/// The order `GetOrder` (and each `ListOrders` item) returns.
pub fn order(id: &str) -> Order {
    Order {
        id: id.to_owned(),
        total_cents: TOTAL_CENTS,
        items: vec![
            LineItem {
                sku: "tea-earl-grey".into(),
                quantity: 2,
            },
            LineItem {
                sku: "mug".into(),
                quantity: 1,
            },
        ],
        state: shop::order::State::Shipped as i32,
        placed_at: Some(prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 0,
        }),
        token: vec![0xde, 0xad, 0xbe, 0xef],
        rating: 4.5,
    }
}

#[derive(Default)]
struct Shop;

type OrderStream = Pin<Box<dyn Stream<Item = Result<Order, Status>> + Send>>;

#[tonic::async_trait]
impl Orders for Shop {
    async fn get_order(&self, request: tonic::Request<GetOrderRequest>) -> Result<Response<Order>, Status> {
        let id = request.into_inner().id;
        if id == "missing" {
            return Err(Status::with_details(
                Code::NotFound,
                format!("no order “{id}”"),
                not_found_details(&id).into(),
            ));
        }
        Ok(Response::new(order(&id)))
    }

    type ListOrdersStream = OrderStream;

    async fn list_orders(
        &self,
        request: tonic::Request<ListOrdersRequest>,
    ) -> Result<Response<Self::ListOrdersStream>, Status> {
        let ListOrdersRequest { count, interval_ms } = request.into_inner();
        let stream = futures_util::stream::unfold(0u32, move |i| async move {
            if i >= count {
                return None;
            }
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(u64::from(interval_ms))).await;
            }
            Some((Ok(order(&format!("o{}", i + 1))), i + 1))
        });
        Ok(Response::new(Box::pin(stream)))
    }
}

/// A `google.rpc.Status` with one `ResourceInfo`-shaped detail, encoded by
/// hand (the testkit has no googleapis types).
fn not_found_details(id: &str) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, number: u8, bytes: &[u8]) {
        out.push(number << 3 | 2);
        prost::encoding::encode_varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }
    let mut info = Vec::new();
    field(&mut info, 1, b"shop.v1.Order");
    field(&mut info, 2, id.as_bytes());
    let mut any = Vec::new();
    field(&mut any, 1, b"type.googleapis.com/google.rpc.ResourceInfo");
    field(&mut any, 2, &info);
    let mut status = vec![0x08, Code::NotFound as u8];
    field(&mut status, 2, format!("no order “{id}”").as_bytes());
    field(&mut status, 3, &any);
    status
}

pub struct GrpcUpstream {
    addr: SocketAddr,
    ca: TestCa,
    task: JoinHandle<()>,
}

impl GrpcUpstream {
    /// Listens on `127.0.0.1:0`.
    pub async fn start() -> GrpcUpstream {
        GrpcUpstream::bind("127.0.0.1:0").await
    }

    pub async fn bind(addr: &str) -> GrpcUpstream {
        let ca = TestCa::new();
        let (chain, key) = ca.issue_localhost();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("server cert");
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind(addr).await.expect("bind gRPC upstream");
        let addr = listener.local_addr().expect("gRPC upstream addr");
        let service = tower::Layer::layer(
            &tonic_web::GrpcWebLayer::new(),
            OrdersServer::new(Shop)
                .accept_compressed(CompressionEncoding::Gzip)
                .accept_compressed(CompressionEncoding::Zstd)
                .send_compressed(CompressionEncoding::Gzip)
                .send_compressed(CompressionEncoding::Zstd),
        );
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let service = TowerToHyperService::new(service.clone());
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
                    let io = TokioIo::new(tls);
                    if h2 {
                        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service)
                            .await;
                    } else {
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await;
                    }
                });
            }
        });
        GrpcUpstream { addr, ca, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn ca(&self) -> &TestCa {
        &self.ca
    }

    /// DER of the issuing CA, for `EngineConfig::extra_upstream_roots`.
    pub fn ca_der(&self) -> Vec<u8> {
        self.ca.der().to_vec()
    }
}

impl Drop for GrpcUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// What a call through the proxy saw.
#[derive(Debug)]
pub struct GrpcResponse {
    pub status: u16,
    pub headers: HeaderMap,
    /// The framed body as received.
    pub body: Bytes,
    pub trailers: Option<HeaderMap>,
}

impl GrpcResponse {
    /// `grpc-status` from the trailers, else the headers (trailers-only).
    pub fn grpc_status(&self) -> Option<u8> {
        self.trailers
            .as_ref()
            .and_then(|t| t.get("grpc-status"))
            .or_else(|| self.headers.get("grpc-status"))
            .and_then(|v| v.to_str().ok()?.parse().ok())
    }

    /// The payloads of the body's uncompressed messages.
    pub fn messages(&self) -> Vec<Bytes> {
        let mut out = Vec::new();
        let mut b = self.body.clone();
        while b.len() >= 5 {
            let len = u32::from_be_bytes([b[1], b[2], b[3], b[4]]) as usize;
            if b[0] & 0x80 == 0 {
                out.push(b.slice(5..5 + len));
            }
            b = b.slice(5 + len..);
        }
        out
    }
}

/// Frames one message, uncompressed.
pub fn frame(message: &impl Message) -> Bytes {
    let payload = message.encode_to_vec();
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(0);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    out.into()
}

/// A gRPC call over h2 to `https://localhost:<port><path>` through the
/// proxy's CONNECT, trusting only `root`. `accept` is sent as
/// `grpc-accept-encoding` (e.g. `gzip`), asking for compressed replies.
pub async fn call(
    proxy: SocketAddr,
    port: u16,
    root: &CertificateDer<'_>,
    path: &str,
    request: &impl Message,
    accept: Option<&str>,
) -> GrpcResponse {
    let tls = tls_through(proxy, "localhost", port, tls_config(root, &[b"h2"]))
        .await
        .expect("TLS through the proxy");
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
        .await
        .expect("h2 handshake");
    tokio::spawn(conn);
    let mut req = Request::post(format!("https://localhost:{port}{path}"))
        .header("content-type", "application/grpc")
        .header("te", "trailers");
    if let Some(accept) = accept {
        req = req.header("grpc-accept-encoding", accept);
    }
    let req = req.body(Full::new(frame(request))).expect("request");
    let response = sender.send_request(req).await.expect("gRPC request");
    collect(response).await
}

/// A gRPC-web call over HTTP/1.1, binary (`application/grpc-web+proto`) or
/// text (`application/grpc-web-text`, Base64).
pub async fn web_call(
    proxy: SocketAddr,
    port: u16,
    root: &CertificateDer<'_>,
    path: &str,
    request: &impl Message,
    text: bool,
) -> GrpcResponse {
    use base64::Engine as _;
    let tls = tls_through(proxy, "localhost", port, tls_config(root, &[b"http/1.1"]))
        .await
        .expect("TLS through the proxy");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .expect("h1 handshake");
    tokio::spawn(conn);
    let (content_type, body) = if text {
        let b64 = base64::engine::general_purpose::STANDARD.encode(frame(request));
        ("application/grpc-web-text", Bytes::from(b64))
    } else {
        ("application/grpc-web+proto", frame(request))
    };
    let req = Request::post(path)
        .header("host", format!("localhost:{port}"))
        .header("content-type", content_type)
        .header("x-grpc-web", "1")
        .body(Full::new(body))
        .expect("request");
    let response = sender.send_request(req).await.expect("gRPC-web request");
    collect(response).await
}

async fn collect(response: http::Response<hyper::body::Incoming>) -> GrpcResponse {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let collected = response.into_body().collect().await.expect("read body");
    let trailers = collected.trailers().cloned();
    GrpcResponse {
        status,
        headers,
        body: collected.to_bytes(),
        trailers,
    }
}
