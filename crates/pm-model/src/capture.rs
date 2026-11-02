//! Capture targets and noise (M21): intercepted launches, the recording
//! allow/block lists and auto-save.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The toolbar's "No Caching" rule: strips conditional request headers and
/// cache validators, and asks for fresh responses.
pub const NO_CACHE_RULE_ID: &str = "builtin-no-cache";
/// The toolbar's "Block Cookies" rule: strips `Cookie` and `Set-Cookie`.
pub const NO_COOKIES_RULE_ID: &str = "builtin-no-cookies";

/// Something the Intercept menu can start, already set up for the proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InterceptTarget {
    Terminal,
    Iterm,
    Chrome,
    Edge,
    Brave,
    Arc,
    Firefox,
}

impl InterceptTarget {
    pub const ALL: [InterceptTarget; 7] = [
        InterceptTarget::Terminal,
        InterceptTarget::Iterm,
        InterceptTarget::Chrome,
        InterceptTarget::Edge,
        InterceptTarget::Brave,
        InterceptTarget::Arc,
        InterceptTarget::Firefox,
    ];

    pub fn is_terminal(self) -> bool {
        matches!(self, InterceptTarget::Terminal | InterceptTarget::Iterm)
    }
}

/// One entry of the Intercept menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InterceptApp {
    pub target: InterceptTarget,
    /// `Google Chrome`, `Terminal`.
    pub name: String,
    /// False if the app isn't in `/Applications` (or `~/Applications`).
    pub installed: bool,
}

/// A running intercepted launch. Its flows (and those of every process
/// it starts) show as `<App> (Intercepted)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct InterceptSession {
    pub id: u32,
    pub target: InterceptTarget,
    pub name: String,
    /// The root of the tracked process tree; `None` until a terminal's
    /// shell has said who it is.
    pub pid: Option<u32>,
    /// Milliseconds since the Unix epoch.
    pub started_at_ms: f64,
    /// What the user should know, e.g. how the browser trusts the CA.
    pub notes: Vec<String>,
}

/// Which hosts are recorded. Unrecorded traffic is still proxied (the SSL
/// policy and rules still apply); it just leaves no flow.
///
/// Both lists are host globs (`*.example.com` also matches `example.com`),
/// case-insensitive. With `only` set, other hosts aren't recorded;
/// `never` wins over `only`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct RecordingSettings {
    pub only: Vec<String>,
    pub never: Vec<String>,
}

/// Settings › Storage › Auto-save: the live capture is saved every
/// `interval_minutes` to `folder` as `Proxymate Autosave <time>.proxymate`,
/// keeping the newest `keep` of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct AutoSaveSettings {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub folder: String,
    pub keep: u32,
}

impl Default for AutoSaveSettings {
    fn default() -> Self {
        AutoSaveSettings {
            enabled: false,
            interval_minutes: 10,
            folder: String::new(),
            keep: 5,
        }
    }
}

/// How auto-save is going, in the storage status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AutoSaveStatus {
    /// A save is being written now.
    pub saving: bool,
    /// When the last save finished (ms since the Unix epoch), and where.
    pub last_at_ms: Option<u64>,
    pub last_path: Option<String>,
    /// Why the last save failed, if it did.
    pub error: Option<String>,
}
