//! Pure data types shared by every Proxymate component.
//!
//! Everything here derives `serde` and `specta::Type`, so the same definitions
//! drive the Tauri commands, the generated TypeScript bindings and (from M7) the
//! MCP schemas. No I/O, no async, no engine logic.

mod detail;
mod filter;
mod flow;
mod platform;
mod rules;
mod stream;
mod tls;
mod view;

pub use detail::{BodyInfo, BodyPart, FlowDetail, Header, HttpVersion, ProxyStatus, RequestHead, ResponseHead};
pub use filter::{ContentGroup, FilterSpec, ProtocolFilter, SortDir, SortKey, SortSpec};
pub use flow::{ContentKind, FlowId, FlowState, FlowSummary, Scheme};
pub use platform::{AppCount, DomainNode, HostCount, KeychainTrust, SidebarSnapshot, SystemProxyStatus, TrustStatus};
pub use rules::{
    Action, AppliedRule, BlockMode, BreakpointBody, BreakpointId, BreakpointInfo, BreakpointResolution, FlowRules,
    GLOBAL_THROTTLE_RULE_ID, KvMatch, MapLocalSource, Matcher, MessageEdits, Pattern, PatternKind, Phase, PresetKind,
    ResponseMatcher, Rule, RuleId, RuleOrigin, RuleSettings, RuleTestResult, StatusRange,
};
pub use stream::{SseEvent, SsePage, WsClose, WsDirection, WsMessage, WsOpcode, WsPage, WsQuery};
pub use tls::{CaInfo, CertSummary, SslSettings, Timing, TlsInfo, TlsStatus};
pub use view::{FlowRow, RowsPage, ViewDelta, ViewId, ViewInfo};
