//! Network plumbing (M18): upstream proxies, reverse proxies, client
//! certificates and DNS overrides, plus the route a flow took (`Via`).
//!
//! Host patterns are globs like the SSL lists: `*` matches across dots and
//! matching ignores case. Passwords never live in these rows: a save hands
//! them to the secret store (the keychain on macOS) and reads come back
//! with `password: None` and `has_password` set.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Everything under Settings › Network, saved as one `app.sqlite` row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct NetworkSettings {
    pub upstream: UpstreamSettings,
    pub reverse_proxies: Vec<ReverseProxy>,
    pub client_certs: Vec<ClientCert>,
    pub dns: Vec<DnsOverride>,
}

/// Where upstream connections go: the first enabled rule whose hosts match
/// wins; a bypassed host or one no rule matches goes direct (or through
/// the proxy set before Proxymate, if asked).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct UpstreamSettings {
    pub rules: Vec<UpstreamRule>,
    /// Hosts that always connect directly.
    pub bypass: Vec<String>,
    /// Hosts no rule matches use the proxy the system had before Proxymate
    /// became the system proxy (or the system proxy itself, while it isn't
    /// Proxymate), honouring its bypass list.
    pub use_system_previous: bool,
}

impl Default for UpstreamSettings {
    fn default() -> Self {
        UpstreamSettings {
            rules: Vec::new(),
            bypass: ["localhost", "127.0.0.1", "::1", "*.local"].map(str::to_owned).to_vec(),
            use_system_previous: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UpstreamRule {
    pub enabled: bool,
    /// Host globs; empty matches nothing.
    pub hosts: Vec<String>,
    pub proxy: UpstreamProxy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpstreamProxy {
    Direct,
    /// `CONNECT` for TLS and WebSockets, absolute form for plain HTTP.
    Http(ProxyServer),
    /// Names are resolved by the SOCKS server.
    Socks5(ProxyServer),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ProxyServer {
    pub host: String,
    pub port: u16,
    /// Basic auth (HTTP) or username/password auth (SOCKS5).
    pub username: Option<String>,
    /// Write only. `None` keeps the saved password, `Some("")` deletes it.
    pub password: Option<String>,
    /// A password is saved in the secret store.
    pub has_password: bool,
}

/// A local listener that forwards everything to one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ReverseProxy {
    pub enabled: bool,
    /// Bound on loopback.
    pub listen_port: u16,
    /// `http://` or `https://` origin, optionally with a path prefix.
    pub target_url: String,
    /// Forward the client's `Host` header instead of the target's.
    pub preserve_host: bool,
    /// Serve HTTPS (a Proxymate leaf for `localhost` and its addresses).
    pub tls: bool,
}

/// A client certificate presented to upstreams that ask for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ClientCert {
    pub enabled: bool,
    /// Host globs; the first enabled match is used.
    pub hosts: Vec<String>,
    pub identity: ClientIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientIdentity {
    /// A `.p12`/`.pfx` file.
    Pkcs12 {
        path: String,
        /// Write only, like [`ProxyServer::password`].
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        has_password: bool,
    },
    /// A PEM certificate chain and an unencrypted PEM key (`key_path`
    /// empty: the key is in the certificate file).
    Pem { cert_path: String, key_path: String },
}

/// Connect to `ip` for hosts matching `host`, without changing SNI or `Host`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DnsOverride {
    pub enabled: bool,
    pub host: String,
    pub ip: String,
}

/// How a flow's upstream connection was made. Last field of `FlowDetail`
/// (session format 5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct Via {
    /// The upstream proxy, e.g. `http://proxy.corp:3128` or
    /// `socks5://127.0.0.1:1080`; `None` for a direct connection.
    pub proxy: Option<String>,
    /// The address the TCP connection went to (the proxy's, when proxied).
    pub addr: Option<String>,
    /// The DNS override that applied, as `pattern → ip`.
    pub dns_override: Option<String>,
    /// The client certificate presented: its subject.
    pub client_cert: Option<String>,
    /// The reverse proxy listener the request arrived on.
    pub reverse_port: Option<u16>,
}

impl Via {
    pub fn is_direct(&self) -> bool {
        self.proxy.is_none()
    }
}

/// A listener besides the main one, for the toolbar and `get_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ListenerStatus {
    pub kind: ListenerKind,
    /// `ip:port`, once bound.
    pub addr: Option<String>,
    /// What it forwards to (reverse proxies).
    pub target: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ListenerKind {
    /// SOCKS5 on the main port (`socks5h://`).
    Socks5,
    Reverse,
    ReverseTls,
}
