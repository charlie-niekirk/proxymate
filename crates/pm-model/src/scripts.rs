//! Scripts (M14): JavaScript run in an embedded QuickJS, with no network
//! or file access. A [`Script`] is one of three kinds:
//!
//! - **Rewrite** (`onRequest`, `onResponse`): changes traffic, run by a
//!   rule's `Action::Script`.
//! - **Mock** (`onMock`): answers a request without the upstream, also run
//!   by a rule's `Action::Script`.
//! - **View** (`onView`): read-only extra inspector views of a body, run
//!   when the inspector shows a flow its [`ViewConfig`] matches.
//!
//! The JS API is documented by `proxymate.d.ts`
//! (`crates/pm-core/src/scripts/proxymate.d.ts`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::{BodyPart, RequestHead, ResponseHead};
use crate::flow::{ContentKind, FlowId};
use crate::rules::{Matcher, Phase, RuleId, RuleOrigin};

/// A script's id: a UUID.
pub type ScriptId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    /// Exports `onRequest` and/or `onResponse`.
    Rewrite,
    /// Exports `onMock`.
    Mock,
    /// Exports `onView`.
    View,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Script {
    pub id: ScriptId,
    pub name: String,
    pub kind: ScriptKind,
    /// A disabled script does nothing, even when a rule names it.
    pub enabled: bool,
    /// An ES module exporting the kind's entry points.
    pub source: String,
    pub origin: RuleOrigin,
    /// Which bodies a view script runs on (view scripts only).
    #[serde(default)]
    pub view: Option<ViewConfig>,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub updated_at_ms: f64,
}

/// Where a view script applies. Every set condition must match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ViewConfig {
    /// The flow's request (and, if set, response) must match.
    #[serde(default)]
    pub matcher: Matcher,
    /// Which bodies get the view; empty means both.
    #[serde(default)]
    pub parts: Vec<BodyPart>,
    /// Only bodies of these kinds; empty means any.
    #[serde(default)]
    pub kinds: Vec<ContentKind>,
}

/// How the inspector renders a script view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScriptViewKind {
    Json,
    Text,
    Xml,
    Html,
    Hex,
    Image,
}

/// One view a view script produced. Text kinds carry `text`; `hex` and
/// `image` carry `base64`. Each is capped at 8 MB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ScriptView {
    pub title: String,
    pub kind: ScriptViewKind,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub base64: Option<String>,
    /// The image type, e.g. `image/png` (images only).
    #[serde(default)]
    pub mime: Option<String>,
    /// The view was longer than the cap and was cut.
    #[serde(default)]
    pub truncated: bool,
}

/// What one view script made of one body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ViewResult {
    pub script_id: ScriptId,
    pub script_name: String,
    pub views: Vec<ScriptView>,
    /// The script threw or timed out.
    pub error: Option<String>,
    pub duration_ms: f64,
}

/// Which entry point a script problem came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScriptPhase {
    Request,
    Response,
    Mock,
    View,
}

impl From<Phase> for ScriptPhase {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Request => ScriptPhase::Request,
            Phase::Response => ScriptPhase::Response,
        }
    }
}

/// A script that failed on a flow (threw, timed out, returned something
/// unusable). The flow went on unmodified by that script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ScriptIssue {
    pub script_id: ScriptId,
    pub script_name: String,
    /// The rule whose action ran the script.
    pub rule_id: Option<RuleId>,
    pub phase: ScriptPhase,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

/// One line of the script console: a script's `console.*` call, or an
/// error the engine reports for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ConsoleEntry {
    /// Increases by one per entry, so readers can ask for what's new.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub time_ms: f64,
    pub script_id: ScriptId,
    pub script_name: String,
    pub level: ConsoleLevel,
    pub message: String,
    /// The flow the script ran for, if it was recorded.
    pub flow_id: Option<FlowId>,
}

/// What to dry-run: a saved script, or source that isn't saved yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ScriptTestSpec {
    /// A saved script. Ignored when `source` is given.
    #[serde(default)]
    pub script_id: Option<ScriptId>,
    /// Unsaved source; `kind` says which entry points to run.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<ScriptKind>,
    /// For view scripts: which body to view (the response by default).
    #[serde(default)]
    pub part: Option<BodyPart>,
}

/// The dry run of a script against a recorded flow. Nothing is sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ScriptTestResult {
    pub kind: ScriptKind,
    /// The first error, if the script threw or timed out.
    pub error: Option<String>,
    /// Rewrite scripts: the request before and after `onRequest`.
    pub request_before: Option<RequestHead>,
    pub request_body_before: Option<String>,
    pub request: Option<RequestHead>,
    pub request_body: Option<String>,
    /// Rewrite scripts: the response before and after `onResponse`.
    /// Mock scripts: the mocked response (`None` when `onMock` passed).
    pub response_before: Option<ResponseHead>,
    pub response_body_before: Option<String>,
    pub response: Option<ResponseHead>,
    pub response_body: Option<String>,
    /// View scripts: the views produced.
    pub views: Vec<ScriptView>,
    /// What the script logged during the run.
    pub console: Vec<ConsoleEntry>,
    pub duration_ms: f64,
    pub notes: Vec<String>,
}

/// Script limits, kept in `app.sqlite` (Settings › Advanced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
#[serde(default)]
pub struct ScriptSettings {
    /// How long `onRequest`, `onResponse` and `onMock` may run.
    pub traffic_timeout_ms: u32,
    /// How long `onView` may run.
    pub view_timeout_ms: u32,
}

impl Default for ScriptSettings {
    fn default() -> Self {
        ScriptSettings {
            traffic_timeout_ms: 1000,
            view_timeout_ms: 3000,
        }
    }
}
