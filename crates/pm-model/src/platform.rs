use serde::{Deserialize, Serialize};
use specta::Type;

/// One client app in the sidebar, with its flow count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AppCount {
    /// What the Client column shows, e.g. `Safari` or `curl`.
    pub name: String,
    pub bundle_id: Option<String>,
    /// True if `pmicon://localhost/<name>` serves an icon.
    pub has_icon: bool,
    /// A LAN device (shown as its nickname and IP), not an app on this Mac.
    #[serde(default)]
    pub remote: bool,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HostCount {
    pub host: String,
    pub count: u32,
}

/// A registrable domain (`example.com`) and the hosts under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DomainNode {
    pub domain: String,
    /// Flows to the domain and all its hosts.
    pub count: u32,
    /// Hosts under the domain, the domain itself included if it was seen.
    /// Empty when the only host is the domain.
    pub hosts: Vec<HostCount>,
}

/// The sidebar's Apps and Domains, aggregated over every flow in the store.
/// Pushed when it changes, at most once per publisher tick.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct SidebarSnapshot {
    pub total: u32,
    /// Sorted by name, case-insensitively.
    pub apps: Vec<AppCount>,
    /// Sorted by domain.
    pub domains: Vec<DomainNode>,
}

/// What the OS trust store says about the Proxymate CA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeychainTrust {
    Trusted,
    NotTrusted,
    /// Explicitly set to "Never Trust".
    Denied,
    /// No trust store integration on this OS yet.
    Unsupported,
}

/// CA trust as the certificate wizard and the toolbar show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TrustStatus {
    pub keychain: KeychainTrust,
    /// A real handshake through our own MITM, checked with the OS verifier,
    /// succeeded. This is what browsers will see.
    pub verified: bool,
    /// Why verification failed, if it did.
    pub detail: Option<String>,
}

/// Whether the OS proxy points at Proxymate.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct SystemProxyStatus {
    /// Proxymate turned it on and it still points at the listener.
    pub enabled: bool,
    /// Active network services whose web or secure web proxy points at us.
    pub services: Vec<String>,
    pub supported: bool,
    /// The last enable/disable/read failure.
    pub error: Option<String>,
}
