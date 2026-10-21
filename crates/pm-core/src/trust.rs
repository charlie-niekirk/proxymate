//! Is the Proxymate CA trusted? The OS trust store gives one answer; a
//! real handshake through our own MITM, checked by the OS verifier, gives
//! the one browsers will act on.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// The host the trust check asks the MITM for. `.test` never resolves, and
/// the proxy answers it itself without dialling or recording anything.
pub const PROBE_HOST: &str = "verify.proxymate.test";

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// CONNECTs to `proxy` for [`PROBE_HOST`] and runs a TLS handshake with
/// the platform verifier and no extra roots. `Ok` means the OS trusts the
/// leaf our CA issued.
pub(crate) async fn handshake_check(proxy: SocketAddr) -> Result<(), String> {
    tokio::time::timeout(PROBE_TIMEOUT, handshake(proxy))
        .await
        .unwrap_or_else(|_| Err("the trust check timed out".into()))
}

async fn handshake(proxy: SocketAddr) -> Result<(), String> {
    let mut tcp = TcpStream::connect(proxy)
        .await
        .map_err(|e| format!("can't reach the proxy at {proxy}: {e}"))?;
    let connect = format!("CONNECT {PROBE_HOST}:443 HTTP/1.1\r\nHost: {PROBE_HOST}:443\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.map_err(|e| e.to_string())?;
    let head = read_head(&mut tcp).await.map_err(|e| e.to_string())?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!(
            "the proxy refused the trust check: {}",
            head.lines().next().unwrap_or("")
        ));
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| e.to_string())?;
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = ServerName::try_from(PROBE_HOST).map_err(|e| e.to_string())?;
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|e| describe(&e))?;
    let _ = tls.shutdown().await;
    Ok(())
}

/// The response head of the CONNECT, read byte by byte so nothing of the
/// TLS stream after it is consumed.
async fn read_head(tcp: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::with_capacity(128);
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 4096 {
            return Err(std::io::Error::other("CONNECT response too long"));
        }
        let byte = tcp.read_u8().await?;
        head.push(byte);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn describe(err: &std::io::Error) -> String {
    let rustls = err.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>());
    match rustls {
        Some(rustls::Error::InvalidCertificate(cert)) => {
            format!("the system doesn't trust the Proxymate CA yet ({cert:?})")
        }
        _ => format!("TLS handshake failed: {err}"),
    }
}
