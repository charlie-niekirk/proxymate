//! Pure data types shared by every Proxymate component.
//!
//! Everything here derives `serde` and `specta::Type`, so the same definitions
//! drive the Tauri commands, the generated TypeScript bindings and (from M7) the
//! MCP schemas. No I/O, no async, no engine logic.

mod filter;
mod flow;
mod view;

pub use filter::{ContentGroup, FilterSpec, ProtocolFilter, SortDir, SortKey, SortSpec};
pub use flow::{ContentKind, FlowId, FlowState, FlowSummary, Scheme};
pub use view::{FlowRow, RowsPage, ViewDelta, ViewId, ViewInfo};
