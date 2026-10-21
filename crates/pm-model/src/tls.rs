use serde::{Deserialize, Serialize};
use specta::Type;

/// How a flow's TLS was handled. Drives the SSL column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TlsStatus {
    /// Plain HTTP.
    #[default]
    None,
    /// Decrypted by the MITM.
    Intercepted,
    /// CONNECT relayed without decrypting (SSL exclude list or bypass).
    Passthrough,
    /// The client rejected our certificate (an `unknown_ca`/`bad_certificate`
    /// alert, or a close before any request). Either the Proxymate CA isn't
    /// trusted by that client or the app pins its certificates.
    LikelyPinned,
}

/// One certificate of the upstream chain, as shown in the TLS tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CertSummary {
    pub subject: String,
    pub issuer: String,
    /// Hex, colon separated.
    pub serial: String,
    /// RFC 3339.
    pub not_before: String,
    /// RFC 3339.
    pub not_after: String,
    /// DNS names and IP addresses from the subjectAltName extension.
    pub san: Vec<String>,
    /// SHA-256 of the DER, hex.
    pub sha256: String,
}

/// TLS facts for an intercepted connection, shared by every flow on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TlsInfo {
    /// Server name the client sent, if any (IP literals have none).
    pub sni: Option<String>,
    /// ALPN protocols the client offered, in its order.
    pub client_alpn: Vec<String>,
    /// Protocol negotiated with the upstream and mirrored to the client.
    pub alpn: Option<String>,
    /// Upstream protocol version, e.g. `TLSv1.3`. Empty if the upstream
    /// handshake failed.
    pub version: String,
    /// Upstream cipher suite, e.g. `TLS13_AES_128_GCM_SHA256`.
    pub cipher: String,
    /// False if the upstream certificate was accepted without verification
    /// (the host is in the "allow insecure upstream" list).
    pub upstream_verified: bool,
    /// The upstream's certificate chain, leaf first.
    pub chain: Vec<CertSummary>,
}

/// Phase timings of one flow, in milliseconds. Connection phases are `None`
/// when the request reused a connection (or, for plain HTTP, came from the
/// pool).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct Timing {
    pub dns_ms: Option<f64>,
    pub connect_ms: Option<f64>,
    /// The upstream TLS handshake.
    pub tls_ms: Option<f64>,
    /// Request handed to the upstream until the response head arrived.
    pub ttfb_ms: Option<f64>,
    /// Response head until the end of the response body.
    pub transfer_ms: Option<f64>,
    /// True if the upstream connection was opened for an earlier flow.
    pub reused_connection: bool,
}

/// Which CONNECT hosts are decrypted. Patterns are host globs (`*` matches
/// across dots, matching is case-insensitive), without ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SslSettings {
    /// Hosts to decrypt. `*` means everything not excluded.
    pub include: Vec<String>,
    /// Hosts never decrypted, e.g. Apple services and OCSP responders.
    pub exclude: Vec<String>,
    /// Hosts added with "Add to SSL bypass", typically pinned apps.
    pub bypass: Vec<String>,
    /// Hosts whose upstream certificate is accepted without verification.
    pub insecure_upstream: Vec<String>,
    /// Client apps never decrypted, by bundle id (or name, for tools with no
    /// bundle). Matching is case-insensitive.
    #[serde(default)]
    pub exclude_apps: Vec<String>,
}

impl Default for SslSettings {
    fn default() -> Self {
        let exclude = [
            // Apple and iCloud pin, and breaking them breaks the Mac.
            "apple.com",
            "*.apple.com",
            "icloud.com",
            "*.icloud.com",
            "*.icloud-content.com",
            "*.apple-cloudkit.com",
            "*.mzstatic.com",
            "*.cdn-apple.com",
            // Revocation checks must reach the real responder.
            "ocsp*",
            "*.ocsp.*",
            // Other well-known pinners.
            "*.whatsapp.net",
            "*.whatsapp.com",
            "*.signal.org",
            "*.dropbox.com",
        ];
        SslSettings {
            include: vec!["*".into()],
            exclude: exclude.into_iter().map(String::from).collect(),
            bypass: Vec::new(),
            insecure_upstream: Vec::new(),
            exclude_apps: Vec::new(),
        }
    }
}

/// The Proxymate root CA, for the certificate popover and wizard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CaInfo {
    pub common_name: String,
    /// PEM file of the CA certificate, e.g. for `curl --cacert`.
    pub cert_path: String,
    /// RFC 3339.
    pub not_before: String,
    /// RFC 3339.
    pub not_after: String,
    /// SHA-256 of the DER, hex.
    pub sha256: String,
}
