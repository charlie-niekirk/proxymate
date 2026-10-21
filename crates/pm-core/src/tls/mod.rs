//! TLS interception: the root CA and the leaves it issues, the three
//! server configs the MITM mirrors the upstream's ALPN choice into, the
//! upstream client configs, and the policy deciding what is decrypted.

pub mod ca;
mod cert_summary;
mod policy;

use std::sync::Arc;

use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};

pub use ca::{CaError, CertAuthority};
pub use cert_summary::summarize;
pub use policy::{PolicyError, SslPolicy};

pub const H2: &[u8] = b"h2";
pub const HTTP11: &[u8] = b"http/1.1";

/// The DER of every certificate in a PEM file, e.g. for
/// `EngineConfig::extra_upstream_roots`.
pub fn certs_from_pem_file(path: &std::path::Path) -> std::io::Result<Vec<Vec<u8>>> {
    use rustls::pki_types::pem::PemObject;
    CertificateDer::pem_file_iter(path)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .map(|cert| {
            cert.map(|c| c.to_vec())
                .map_err(|e| std::io::Error::other(e.to_string()))
        })
        .collect()
}

/// Everything the MITM needs that depends on the CA. Rebuilt as a whole
/// when the CA is regenerated.
pub struct TlsContext {
    pub ca: Arc<CertAuthority>,
    provider: Arc<CryptoProvider>,
    server_h2: Arc<ServerConfig>,
    server_h1: Arc<ServerConfig>,
    server_plain: Arc<ServerConfig>,
    verifier: Arc<dyn ServerCertVerifier>,
    client: ClientConfig,
}

impl TlsContext {
    /// `extra_roots` are trusted for upstreams on top of the OS trust store
    /// (tests use them for their own CA).
    pub fn new(ca: CertAuthority, extra_roots: Vec<CertificateDer<'static>>) -> Result<TlsContext, rustls::Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let ca = Arc::new(ca);
        let verifier: Arc<dyn ServerCertVerifier> = Arc::new(if extra_roots.is_empty() {
            rustls_platform_verifier::Verifier::new(provider.clone())?
        } else {
            rustls_platform_verifier::Verifier::new_with_extra_roots(extra_roots, provider.clone())?
        });
        let client = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        let server =
            |alpn: &[&[u8]]| -> Result<Arc<ServerConfig>, rustls::Error> { server_config(&provider, &ca, None, alpn) };
        Ok(TlsContext {
            server_h2: server(&[H2])?,
            server_h1: server(&[HTTP11])?,
            server_plain: server(&[])?,
            ca,
            provider,
            verifier,
            client,
        })
    }

    /// The server config that offers the client exactly `alpn`, the
    /// protocol the upstream picked (`None`: no ALPN). Leaves come from
    /// the client's SNI; a client that sends none (an IP literal) gets one
    /// for `host`, at the cost of building a config.
    pub fn server_config(
        &self,
        alpn: Option<&[u8]>,
        sni: Option<&str>,
        host: &str,
    ) -> Result<Arc<ServerConfig>, rustls::Error> {
        if sni.is_none() {
            let alpn: &[&[u8]] = match alpn {
                Some(H2) => &[H2],
                Some(HTTP11) => &[HTTP11],
                _ => &[],
            };
            return server_config(&self.provider, &self.ca, Some(host), alpn);
        }
        Ok(match alpn {
            Some(H2) => self.server_h2.clone(),
            Some(HTTP11) => self.server_h1.clone(),
            _ => self.server_plain.clone(),
        })
    }

    /// A client config for one upstream connection offering `alpn`. The
    /// returned recorder captures the upstream's chain even when it fails
    /// verification, for the TLS tab and the error message.
    pub fn client_config(&self, alpn: Vec<Vec<u8>>, insecure: bool) -> (Arc<ClientConfig>, Arc<ChainRecorder>) {
        let recorder = Arc::new(ChainRecorder {
            inner: self.verifier.clone(),
            insecure,
            chain: Mutex::new(Vec::new()),
        });
        let mut config = self.client.clone();
        config.alpn_protocols = alpn;
        config.dangerous().set_certificate_verifier(recorder.clone());
        (Arc::new(config), recorder)
    }
}

fn server_config(
    provider: &Arc<CryptoProvider>,
    ca: &Arc<CertAuthority>,
    fallback: Option<&str>,
    alpn: &[&[u8]],
) -> Result<Arc<ServerConfig>, rustls::Error> {
    let mut config = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(ca::LeafResolver {
            ca: ca.clone(),
            fallback: fallback.map(str::to_owned),
        }));
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(config))
}

/// Wraps the real verifier for one connection: keeps the presented chain,
/// and in "allow insecure upstream" mode accepts any chain (signatures are
/// still checked, so the peer must hold the key it presents).
#[derive(Debug)]
pub struct ChainRecorder {
    inner: Arc<dyn ServerCertVerifier>,
    insecure: bool,
    chain: Mutex<Vec<CertificateDer<'static>>>,
}

impl ChainRecorder {
    /// The chain the upstream presented, leaf first; empty if it never got
    /// that far.
    pub fn chain(&self) -> Vec<CertificateDer<'static>> {
        self.chain.lock().clone()
    }
}

impl ServerCertVerifier for ChainRecorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.chain.lock() = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|c| c.clone().into_owned())
            .collect();
        if self.insecure {
            return Ok(ServerCertVerified::assertion());
        }
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
