use serde::{Deserialize, Serialize};
use specta::Type;

/// Limits on what the live capture keeps. Older flows are evicted (pinned
/// and in-flight ones excepted) once either limit is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct StorageSettings {
    /// Flows kept in the live capture; the oldest go first. 0 keeps all.
    pub max_flows: u32,
    /// Disk used by bodies and details, in MB; the oldest flows go first.
    /// 0 is unlimited.
    pub max_disk_mb: u32,
    /// WebSocket messages kept per connection; the oldest go first.
    pub max_ws_messages: u32,
}

impl Default for StorageSettings {
    fn default() -> Self {
        StorageSettings {
            max_flows: 250_000,
            max_disk_mb: 4096,
            max_ws_messages: 50_000,
        }
    }
}

/// What the live capture holds now, for Settings › Storage and the footer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct StorageStatus {
    pub flows: u32,
    pub pinned: u32,
    /// Bytes of segment files on disk.
    pub disk_bytes: u64,
    /// Flows evicted by the limits since launch.
    pub evicted: u64,
    /// The last failed write to the segment files (a full disk, say):
    /// bodies are no longer recorded until writes succeed again.
    pub error: Option<String>,
    /// Where the segment files live.
    pub dir: String,
    /// Body bytes sent and received through the proxy since launch; the
    /// footer's throughput is their rate of change.
    pub sent_bytes: u64,
    pub received_bytes: u64,
    /// Flows the recording lists (M21) kept out of the capture since launch.
    pub unrecorded: u64,
    pub autosave: crate::AutoSaveStatus,
}

/// The proxy listener and its upstream client. A port or pool change
/// restarts the listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ProxySettings {
    /// The port tried first; the next nine are fallbacks.
    pub port: u16,
    /// Seconds an idle upstream connection is kept for reuse (plain HTTP).
    pub pool_idle_secs: u32,
    /// Idle upstream connections kept per host (plain HTTP).
    pub pool_max_idle_per_host: u32,
    /// Seconds to wait for an upstream TCP connection.
    pub connect_timeout_secs: u32,
}

impl Default for ProxySettings {
    fn default() -> Self {
        ProxySettings {
            port: 9090,
            pool_idle_secs: 90,
            pool_max_idle_per_host: 32,
            connect_timeout_secs: 10,
        }
    }
}
