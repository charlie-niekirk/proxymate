//! Everyday workflow (M17): header columns, Repeat and Copy as code.
//! Selections, labels and comments live with the flow types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::BodyPart;
use crate::flow::FlowId;

/// At most this many header columns.
pub const MAX_HEADER_COLUMNS: usize = 8;

/// A flow table column showing one request or response header.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
pub struct HeaderColumn {
    pub part: BodyPart,
    /// Matched case-insensitively.
    pub name: String,
}

/// How `Engine::repeat` sends a request again and again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct RepeatOpts {
    /// 1 to 10,000.
    pub iterations: u32,
    /// Requests in flight at once, 1 to 64.
    pub concurrency: u32,
    /// Pause before each request (per worker), in milliseconds.
    #[serde(default)]
    pub delay_ms: u32,
    /// Run each request through the rules, as `SendOpts::apply_rules`.
    #[serde(default = "yes")]
    pub apply_rules: bool,
}

fn yes() -> bool {
    true
}

impl Default for RepeatOpts {
    fn default() -> Self {
        RepeatOpts {
            iterations: 10,
            concurrency: 1,
            delay_ms: 0,
            apply_rules: true,
        }
    }
}

/// Handle to a running repeat, for its progress and to cancel it.
pub type RepeatId = u32;

/// How many responses had one status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct StatusCount {
    /// `None` counts failed requests (no response).
    pub status: Option<u16>,
    pub count: u32,
}

/// A repeat's progress, pushed as it goes; the last one has `finished`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type, JsonSchema)]
pub struct RepeatProgress {
    pub id: RepeatId,
    /// Requests that have finished (answered or failed).
    pub done: u32,
    pub total: u32,
    /// Requests that got no response.
    pub failed: u32,
    /// By status, ascending; failures (`status: null`) first.
    pub statuses: Vec<StatusCount>,
    /// Durations of the finished requests, in milliseconds.
    pub min_ms: Option<u32>,
    pub p50_ms: Option<u32>,
    pub p95_ms: Option<u32>,
    pub max_ms: Option<u32>,
    /// The first and last flow the repeat recorded.
    pub first_flow: Option<FlowId>,
    pub last_flow: Option<FlowId>,
    pub finished: bool,
    pub cancelled: bool,
    /// Why it stopped early (the request couldn't be sent at all).
    pub error: Option<String>,
}

/// Languages Copy as code writes (`pm-core/src/io/snippets.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeLanguage {
    Curl,
    /// JavaScript `fetch`.
    Fetch,
    /// Python `requests`.
    Python,
    Httpie,
    /// Go `net/http`.
    Go,
    /// Swift `URLSession`.
    Swift,
    /// Kotlin OkHttp.
    Kotlin,
    #[serde(rename = "powershell")]
    PowerShell,
}

impl CodeLanguage {
    pub const ALL: [CodeLanguage; 8] = [
        CodeLanguage::Curl,
        CodeLanguage::Fetch,
        CodeLanguage::Python,
        CodeLanguage::Httpie,
        CodeLanguage::Go,
        CodeLanguage::Swift,
        CodeLanguage::Kotlin,
        CodeLanguage::PowerShell,
    ];
}
