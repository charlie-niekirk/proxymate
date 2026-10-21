//! TLS test upstreams and the test CA that signs them.
//!
//! A [`TlsUpstream`] serves the same routes as [`crate::Upstream`] over
//! TLS, speaking h2 or HTTP/1.1 as ALPN decides. Its certificate is issued
//! by [`TestCa`] (pass [`TlsUpstream::ca_der`] to the engine as an extra
//! upstream root) or, with [`TlsUpstream::self_signed`], by nobody.

use std::net::SocketAddr;
use std::sync::Arc;

use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

use crate::upstream::route;

/// A throwaway root CA for test upstreams.
pub struct TestCa {
    der: CertificateDer<'static>,
    pem: String,
    issuer: Issuer<'static, KeyPair>,
}

impl TestCa {
    pub fn new() -> TestCa {
        let key = KeyPair::generate().expect("CA key");
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, "pm-testkit CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).expect("self-sign CA");
        TestCa {
            der: cert.der().clone(),
            pem: cert.pem(),
            issuer: Issuer::new(params, key),
        }
    }

    pub fn der(&self) -> &CertificateDer<'static> {
        &self.der
    }

    pub fn pem(&self) -> &str {
        &self.pem
    }

    /// A server certificate for `localhost` and `127.0.0.1`.
    pub fn issue_localhost(&self) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().expect("leaf key");
        let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).expect("leaf params");
        params.distinguished_name.push(DnType::CommonName, "localhost");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, &self.issuer).expect("sign leaf");
        (
            vec![cert.der().clone(), self.der.clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }
}

impl Default for TestCa {
    fn default() -> Self {
        TestCa::new()
    }
}

/// Protocols a [`TlsUpstream`] offers over ALPN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alpn {
    /// `h2` and `http/1.1`, preferring h2.
    Both,
    H2,
    Http11,
    /// No ALPN at all: HTTP/1.1.
    None,
}

impl Alpn {
    fn protocols(self) -> Vec<Vec<u8>> {
        match self {
            Alpn::Both => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            Alpn::H2 => vec![b"h2".to_vec()],
            Alpn::Http11 => vec![b"http/1.1".to_vec()],
            Alpn::None => Vec::new(),
        }
    }
}

pub struct TlsUpstream {
    addr: SocketAddr,
    ca: Option<TestCa>,
    task: JoinHandle<()>,
}

impl TlsUpstream {
    /// An upstream with a certificate from a fresh [`TestCa`].
    pub async fn start(alpn: Alpn) -> TlsUpstream {
        let ca = TestCa::new();
        let (chain, key) = ca.issue_localhost();
        Self::serve(chain, key, alpn, Some(ca)).await
    }

    /// An upstream with a self-signed certificate nobody trusts.
    pub async fn self_signed(alpn: Alpn) -> TlsUpstream {
        let key = KeyPair::generate().expect("key");
        let params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).expect("params");
        let cert = params.self_signed(&key).expect("self-sign");
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        Self::serve(vec![cert.der().clone()], key, alpn, None).await
    }

    async fn serve(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        alpn: Alpn,
        ca: Option<TestCa>,
    ) -> TlsUpstream {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("server cert");
        config.alpn_protocols = alpn.protocols();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind TLS upstream");
        let addr = listener.local_addr().expect("TLS upstream addr");
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
                    let io = TokioIo::new(tls);
                    if h2 {
                        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service_fn(route))
                            .await;
                    } else {
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service_fn(route))
                            .await;
                    }
                });
            }
        });
        TlsUpstream { addr, ca, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// `https://localhost:<port><path>`.
    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.addr.port())
    }

    /// The CA that issued this upstream's certificate (`None` if self-signed).
    pub fn ca(&self) -> Option<&TestCa> {
        self.ca.as_ref()
    }

    /// DER of the issuing CA, for `EngineConfig::extra_upstream_roots`.
    pub fn ca_der(&self) -> Vec<u8> {
        self.ca.as_ref().map(|ca| ca.der().to_vec()).unwrap_or_default()
    }
}

impl Drop for TlsUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
