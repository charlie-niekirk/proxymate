//! The MCP server's settings, live status and audit log, as the Settings
//! page and the toolbar indicator see them. The tool shapes themselves live
//! in `pm-mcp` (built from the rule and breakpoint types here).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The default port of the Streamable HTTP endpoint (`/mcp`).
pub const MCP_DEFAULT_PORT: u16 = 9095;

/// Kept in `app.sqlite`. Agents get full access unless `read_only` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct McpSettings {
    /// Serve MCP on `127.0.0.1:<port>/mcp`.
    pub enabled: bool,
    /// Hide and refuse every tool that changes something (rules,
    /// breakpoints, recording, clearing).
    pub read_only: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        McpSettings {
            enabled: true,
            read_only: false,
        }
    }
}

/// An MCP client with an open session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct McpClient {
    /// Proxymate's id for the session; the client's rules carry it too.
    pub session: String,
    /// `clientInfo.name` from `initialize`, e.g. `claude-code`.
    pub name: String,
    pub version: String,
    /// Milliseconds since the Unix epoch.
    pub connected_at_ms: f64,
}

/// What the toolbar indicator and the Settings page show.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
pub struct McpStatus {
    pub settings: McpSettings,
    /// `http://127.0.0.1:<port>/mcp` while serving.
    pub url: Option<String>,
    /// Why the server isn't serving, if it should be.
    pub error: Option<String>,
    pub clients: Vec<McpClient>,
}

/// One tool call, as recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct McpAuditEntry {
    /// Increasing; also the cursor for older entries.
    pub id: i64,
    /// Milliseconds since the Unix epoch.
    pub time_ms: f64,
    pub client: String,
    pub session: Option<String>,
    pub tool: String,
    /// The arguments as JSON, cut at 4 KB.
    pub arguments: String,
    pub outcome: McpCallOutcome,
    /// The error the agent got, if the call failed or was refused.
    pub error: Option<String>,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum McpCallOutcome {
    Ok,
    Error,
    /// A mutating tool called while the server is read-only.
    Refused,
}

/// A page of the audit log, newest first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
pub struct McpAuditPage {
    pub entries: Vec<McpAuditEntry>,
    /// Pass as `before` for the next (older) page.
    pub next: Option<i64>,
}
