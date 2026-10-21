use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::HttpVersion;
use crate::tls::TlsStatus;

/// Stable identifier of a flow within one live store or saved session.
///
/// Ids are assigned monotonically and never reused, so the UI can key caches
/// on them across refilters.
pub type FlowId = u32;

/// URL scheme of a flow as seen by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Http,
    Https,
    Ws,
    Wss,
}

impl Scheme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Ws => "ws",
            Scheme::Wss => "wss",
        }
    }

    pub const fn is_tls(self) -> bool {
        matches!(self, Scheme::Https | Scheme::Wss)
    }

    pub const fn is_websocket(self) -> bool {
        matches!(self, Scheme::Ws | Scheme::Wss)
    }
}

/// Lifecycle state of a flow. Drives the state dot and the status pill colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum FlowState {
    /// Request sent, response not finished yet.
    Active,
    /// Held by a breakpoint.
    Paused,
    Completed,
    Failed,
    /// Stopped by a `Block` rule action.
    Blocked,
    /// CONNECT passthrough, not decrypted.
    Tunnel,
}

/// Detected kind of a body (`pm-core/src/io/content.rs`).
///
/// The summary stores the flow's kind (the request's for GraphQL operations
/// and form posts, else the response's, else the request's) so the filter
/// chips can use it without touching bodies. Formats within a kind (PNG vs
/// WebP, MP4 vs WebM) are in [`crate::BodyInfo::mime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    /// No body, or not detected yet.
    #[default]
    None,
    Json,
    Xml,
    Html,
    FormUrlEncoded,
    Multipart,
    GraphQl,
    Javascript,
    Css,
    Text,
    Image,
    Svg,
    Video,
    Audio,
    Font,
    Pdf,
    Binary,
}

/// The compact per-flow record kept in the summary arena (~150-200 B).
///
/// Strings that repeat across flows (`method`, `host`, `client`) are interned
/// `Arc<str>`s, so cloning a summary is cheap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FlowSummary {
    pub id: FlowId,
    pub state: FlowState,
    pub scheme: Scheme,
    pub method: Arc<str>,
    pub host: Arc<str>,
    pub port: u16,
    /// Path without the query string, always starting with `/`.
    pub path: String,
    /// Raw query string without the leading `?`; empty if none.
    pub query: String,
    /// Response status code, once the response head has arrived.
    pub status: Option<u16>,
    pub content_kind: ContentKind,
    /// Client application name (macOS app detection lands in M4).
    pub client: Option<Arc<str>>,
    /// Wall-clock start time, milliseconds since the Unix epoch.
    pub started_at_ms: f64,
    /// Total duration once the flow has finished.
    pub duration_ms: Option<u32>,
    /// Request body size on the wire, in bytes.
    pub req_size: u64,
    /// Response body size on the wire, in bytes.
    pub resp_size: u64,
    /// True if any rule modified the flow.
    pub edited: bool,
    /// Protocol the client spoke (mirrored upstream for HTTPS).
    pub http_version: HttpVersion,
    pub tls: TlsStatus,
    /// Operation name of a GraphQL request, shown in the URL column.
    pub graphql_op: Option<Arc<str>>,
}

impl FlowSummary {
    /// True if `port` is the default for the scheme and can be omitted from URLs.
    pub fn is_default_port(&self) -> bool {
        matches!(
            (self.scheme, self.port),
            (Scheme::Http | Scheme::Ws, 80) | (Scheme::Https | Scheme::Wss, 443)
        )
    }
}
