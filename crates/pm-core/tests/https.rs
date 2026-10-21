//! End-to-end HTTPS: client → CONNECT → Proxymate MITM → pm-testkit TLS
//! upstream, plus passthrough tunnels, upstream failures and clients that
//! reject our certificate.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::{TokioExecutor, TokioIo};
use pm_core::{Engine, EngineConfig, ProxyConfig};
use pm_model::{BodyPart, FlowDetail, FlowId, FlowState, FlowSummary, HttpVersion, Scheme, TlsStatus};
use pm_testkit::client::{self, HttpsResponse, eventually, https_get, tls_config, tls_through};
use pm_testkit::upstream::JSON_BODY;
use pm_testkit::{Alpn, TestCa, TlsUpstream, Upstream};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(10);
const H2: &[u8] = b"h2";
const H1: &[u8] = b"http/1.1";

struct Proxy {
    engine: Arc<Engine>,
    addr: SocketAddr,
}

impl Proxy {
    /// A proxy that trusts `upstream`'s test CA for upstream TLS.
    async fn start(upstream: Option<&TlsUpstream>) -> Proxy {
        let dir = std::env::temp_dir()
            .join("proxymate-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let engine = Engine::with_config(EngineConfig {
            data_dir: Some(dir),
            extra_upstream_roots: upstream.map(|u| vec![u.ca_der()]).unwrap_or_default(),
        });
        let config = ProxyConfig {
            ports: 0..=0,
            ..Default::default()
        };
        let addr = engine.start_proxy(config).await.expect("proxy starts");
        Proxy { engine, addr }
    }

    /// The Proxymate CA, read back from the PEM file curl would use.
    fn ca(&self) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(self.engine.ca_info().cert_path).expect("CA PEM")
    }

    async fn get(&self, url: &str, alpn: &[&[u8]]) -> HttpsResponse {
        https_get(self.addr, url, &self.ca(), alpn).await
    }

    async fn finished(&self, id: FlowId) -> (FlowSummary, FlowDetail) {
        let summary = eventually(WAIT, || {
            self.engine.flush();
            self.engine.get_flow(id).filter(|f| f.state != FlowState::Active)
        })
        .await;
        let detail = self.engine.flow_detail(id).unwrap().expect("detail");
        (summary, detail)
    }
}

#[tokio::test]
async fn h1_over_tls_is_decrypted_and_recorded() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;

    let response = proxy.get(&upstream.url("/json?page=2"), &[H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.alpn.as_deref(), Some(H1));
    assert_eq!(response.version, http::Version::HTTP_11);
    assert_eq!(response.body, JSON_BODY.as_bytes());

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!((flow.state, flow.status), (FlowState::Completed, Some(200)));
    assert_eq!((flow.scheme, flow.tls), (Scheme::Https, TlsStatus::Intercepted));
    assert_eq!(flow.http_version, HttpVersion::Http11);
    assert_eq!((&*flow.host, flow.port), ("localhost", upstream.port()));
    assert_eq!((flow.path.as_str(), flow.query.as_str()), ("/json", "page=2"));
    assert_eq!(detail.request.url, upstream.url("/json?page=2"));

    let tls = detail.tls.expect("TLS info");
    assert_eq!(tls.sni.as_deref(), Some("localhost"));
    assert_eq!(tls.client_alpn, vec!["http/1.1"]);
    assert_eq!(tls.alpn.as_deref(), Some("http/1.1"));
    assert!(tls.version.starts_with("TLSv1."), "{}", tls.version);
    assert!(!tls.cipher.is_empty());
    assert!(tls.upstream_verified);
    assert_eq!(tls.chain.len(), 2);
    assert!(tls.chain[0].san.contains(&"localhost".to_owned()));

    let timing = detail.timing;
    assert!(timing.connect_ms.is_some() && timing.tls_ms.is_some());
    assert!(timing.ttfb_ms.is_some() && timing.transfer_ms.is_some());
    assert!(!timing.reused_connection);

    let body = proxy.engine.body(0, BodyPart::Response, true).unwrap();
    assert_eq!(body.bytes, JSON_BODY.as_bytes());
}

#[tokio::test]
async fn h2_is_mirrored_end_to_end() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;

    let response = proxy.get(&upstream.url("/version"), &[H2, H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.alpn.as_deref(), Some(H2));
    assert_eq!(response.version, http::Version::HTTP_2);
    // The upstream saw h2 too: the proxy never translates.
    assert_eq!(response.body, "HTTP/2.0");

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.http_version, HttpVersion::Http2);
    assert_eq!(flow.state, FlowState::Completed);
    assert_eq!(detail.request.version, HttpVersion::Http2);
    assert_eq!(detail.response.unwrap().version, HttpVersion::Http2);
    assert_eq!(detail.tls.unwrap().alpn.as_deref(), Some("h2"));
}

#[tokio::test]
async fn h2_streams_share_one_upstream_connection() {
    let upstream = TlsUpstream::start(Alpn::H2).await;
    let proxy = Proxy::start(Some(&upstream)).await;

    let tls = tls_through(proxy.addr, "localhost", upstream.port(), tls_config(&proxy.ca(), &[H2]))
        .await
        .unwrap();
    let (sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(conn);
    let requests = ["/json", "/chunked", "/bytes/100000"].map(|path| {
        let mut sender = sender.clone();
        let url = upstream.url(path);
        async move {
            let req = Request::get(url).body(Full::<Bytes>::default()).unwrap();
            let response = sender.send_request(req).await.unwrap();
            assert_eq!(response.status(), 200);
            response.into_body().collect().await.unwrap().to_bytes().len()
        }
    });
    let sizes = futures_join(requests).await;
    assert_eq!(sizes[2], 100_000);

    let mut reused = 0;
    for id in 0..3 {
        let (flow, detail) = proxy.finished(id).await;
        assert_eq!(
            (flow.state, flow.http_version),
            (FlowState::Completed, HttpVersion::Http2)
        );
        reused += usize::from(detail.timing.reused_connection);
    }
    assert_eq!(reused, 2, "one flow opened the upstream connection, two reused it");
}

/// Runs the futures concurrently on one task.
async fn futures_join<F: Future<Output = usize>>(futures: [F; 3]) -> [usize; 3] {
    let [a, b, c] = futures;
    let (a, b, c) = tokio::join!(a, b, c);
    [a, b, c]
}

#[tokio::test]
async fn client_side_alpn_mirrors_the_upstream() {
    for (alpn, expected) in [(Alpn::Http11, Some(H1)), (Alpn::None, None), (Alpn::H2, Some(H2))] {
        let upstream = TlsUpstream::start(alpn).await;
        let proxy = Proxy::start(Some(&upstream)).await;
        let response = proxy.get(&upstream.url("/version"), &[H2, H1]).await;
        assert_eq!(response.status, 200, "{alpn:?}");
        assert_eq!(response.alpn.as_deref(), expected, "{alpn:?}");
    }
}

#[tokio::test]
async fn excluded_hosts_are_tunnelled_and_recorded_once() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    let settings = proxy.engine.add_ssl_bypass("localhost").unwrap();
    assert!(settings.bypass.contains(&"localhost".to_owned()));

    // End-to-end TLS with the upstream itself: only its CA is trusted.
    let root = upstream.ca().unwrap().der().clone();
    let response = https_get(proxy.addr, &upstream.url("/json"), &root, &[H2, H1]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, JSON_BODY.as_bytes());

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Tunnel);
    assert_eq!((&*flow.method, flow.tls), ("CONNECT", TlsStatus::Passthrough));
    assert!(flow.req_size > 0 && flow.resp_size > JSON_BODY.len() as u64);
    assert_eq!(detail.request.url, format!("localhost:{}", upstream.port()));
    assert!(detail.tls.is_none());
    assert_eq!(proxy.engine.total(), 1, "nothing inside the tunnel is recorded");
}

#[tokio::test]
async fn the_bypass_list_survives_a_restart() {
    let dir = std::env::temp_dir()
        .join("proxymate-tests")
        .join(uuid::Uuid::new_v4().to_string());
    let config = EngineConfig {
        data_dir: Some(dir),
        ..Default::default()
    };
    let engine = Engine::with_config(config.clone());
    engine.add_ssl_bypass("Pinned.Example.com.").unwrap();
    engine.set_insecure_upstream("dev.test", true).unwrap();
    let ca = engine.ca_info();
    drop(engine);
    let engine = Engine::with_config(config);
    let settings = engine.ssl_settings();
    assert_eq!(settings.bypass, vec!["pinned.example.com"]);
    assert_eq!(settings.insecure_upstream, vec!["dev.test"]);
    assert_eq!(engine.ca_info(), ca, "the CA is reused, not regenerated");
}

#[tokio::test]
async fn self_signed_upstreams_fail_with_a_readable_502() {
    let upstream = TlsUpstream::self_signed(Alpn::Both).await;
    let proxy = Proxy::start(None).await;

    let response = proxy.get(&upstream.url("/json"), &[H2, H1]).await;
    assert_eq!(response.status, 502);
    let page = String::from_utf8_lossy(&response.body);
    assert!(
        page.contains("<html>") && page.contains("allow insecure upstream"),
        "{page}"
    );

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!((flow.state, flow.tls), (FlowState::Failed, TlsStatus::Intercepted));
    assert_eq!(flow.path, "/json");
    let error = detail.error.expect("error");
    assert!(error.contains("self-signed or issued by an unknown CA"), "{error}");
    let tls = detail.tls.expect("TLS info");
    assert_eq!(tls.chain.len(), 1, "the rejected chain is still shown");
    assert!(detail.timing.tls_ms.is_some());

    // Allowing insecure upstream for the host lets it through.
    proxy.engine.set_insecure_upstream("localhost", true).unwrap();
    let response = proxy.get(&upstream.url("/json"), &[H2, H1]).await;
    assert_eq!(response.status, 200);
    let (flow, detail) = proxy.finished(1).await;
    assert_eq!(flow.state, FlowState::Completed);
    assert!(!detail.tls.unwrap().upstream_verified);
}

#[tokio::test]
async fn unreachable_upstreams_fail_the_flow() {
    let proxy = Proxy::start(None).await;
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let response = proxy.get(&format!("https://127.0.0.1:{closed}/"), &[H1]).await;
    assert_eq!(response.status, 502);
    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.state, FlowState::Failed);
    assert!(detail.error.unwrap().contains("can't reach 127.0.0.1"));
}

#[tokio::test]
async fn clients_rejecting_our_certificate_are_flagged_as_pinned() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;

    // A client that only trusts some other CA sends an unknown_ca alert.
    let stranger = TestCa::new();
    let result = tls_through(
        proxy.addr,
        "localhost",
        upstream.port(),
        tls_config(stranger.der(), &[H1]),
    )
    .await;
    assert!(result.is_err());

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!((flow.state, flow.tls), (FlowState::Failed, TlsStatus::LikelyPinned));
    assert_eq!(&*flow.method, "CONNECT");
    let error = detail.error.unwrap();
    assert!(error.contains("unknown_ca") && error.contains("SSL bypass"), "{error}");
}

#[tokio::test]
async fn clients_closing_right_after_the_handshake_are_flagged_as_pinned() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;

    let mut tls = tls_through(proxy.addr, "localhost", upstream.port(), tls_config(&proxy.ca(), &[H1]))
        .await
        .unwrap();
    tls.shutdown().await.unwrap();
    drop(tls);

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!(flow.tls, TlsStatus::LikelyPinned);
    assert!(detail.error.unwrap().contains("right after the TLS handshake"));
}

#[tokio::test]
async fn leaf_certificates_are_reused_across_connections() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    let leaf = || async {
        let tls = tls_through(proxy.addr, "localhost", upstream.port(), tls_config(&proxy.ca(), &[H1]))
            .await
            .unwrap();
        tls.get_ref().1.peer_certificates().unwrap()[0].clone()
    };
    let first = leaf().await;
    let second = leaf().await;
    // A fresh leaf would have a new serial and signature.
    assert_eq!(first, second);
}

#[tokio::test]
async fn regenerating_the_ca_changes_what_clients_must_trust() {
    let upstream = TlsUpstream::start(Alpn::Both).await;
    let proxy = Proxy::start(Some(&upstream)).await;
    let old = proxy.ca();
    let info = proxy.engine.regenerate_ca().unwrap();
    assert_ne!(CertificateDer::from_pem_file(&info.cert_path).unwrap(), old);

    let stale = tls_through(proxy.addr, "localhost", upstream.port(), tls_config(&old, &[H1])).await;
    assert!(stale.is_err(), "leaves now chain to the new CA");
    assert_eq!(proxy.get(&upstream.url("/json"), &[H1]).await.status, 200);
}

#[tokio::test]
async fn plain_http_inside_connect_is_served_as_http() {
    let upstream = Upstream::start().await;
    let proxy = Proxy::start(None).await;
    let mut tunnel = client::tunnel(proxy.addr, &upstream.addr().to_string()).await;
    tunnel
        .write_all(b"GET /json HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    tunnel.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with(JSON_BODY));

    let (flow, detail) = proxy.finished(0).await;
    assert_eq!((flow.scheme, flow.tls), (Scheme::Http, TlsStatus::None));
    assert_eq!(detail.request.url, upstream.url("/json"));
}
