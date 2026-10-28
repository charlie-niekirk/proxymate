use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::HttpVersion;
use crate::flow::{ContentKind, FlowId, FlowState, FlowSummary, Scheme};
use crate::send::SentBy;
use crate::tls::TlsStatus;

/// Handle to a live, filtered and sorted view over a flow store.
pub type ViewId = u32;

/// Per-tick change notification for one view, published at 20 Hz.
///
/// The UI never receives rows in a delta; it re-fetches the visible window
/// with `get_rows` when the delta says something in it may have changed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct ViewDelta {
    pub view: ViewId,
    /// Monotonic per-view sequence number, starting at 1.
    pub seq: u32,
    /// Flows in the store (unfiltered).
    pub total: u32,
    /// Rows in the view after this tick.
    pub len: u32,
    /// Rows appended at the end of the view since the previous tick.
    pub appended: u32,
    /// Ids of flows already in the view whose summary changed.
    pub changed: Vec<FlowId>,
    /// Rows were inserted, removed or reordered: drop every cached row.
    pub structural: bool,
    /// Progress of the deep search (header and body terms), while the
    /// filter has any.
    pub deep: Option<DeepProgress>,
}

/// A view's deep search: header and body predicates checked in the
/// background, with matching flows streaming into the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct DeepProgress {
    /// Flows checked so far.
    pub checked: u32,
    /// Flows to check: those matching the rest of the filter.
    pub total: u32,
    /// Flows found to match.
    pub matched: u32,
    /// False once every candidate has been checked (new flows are still
    /// checked as they finish).
    pub running: bool,
}

/// Result of creating a view or changing its filter or sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ViewInfo {
    pub view: ViewId,
    pub total: u32,
    pub len: u32,
    /// Time the engine spent refiltering and sorting, in microseconds.
    pub elapsed_us: u32,
    /// Set when the filter has header or body terms: a deep search started.
    pub deep: Option<DeepProgress>,
}

/// One flow table row in a compact positional form (a JSON array on the wire).
///
/// Field order: `id, state, scheme, method, host, port, path, query, status,
/// content_kind, client, started_at_ms, duration_ms, req_size, resp_size, edited,
/// http_version, tls, graphql_op, sent_by, pinned, grpc_status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FlowRow(
    pub FlowId,
    pub FlowState,
    pub Scheme,
    pub Arc<str>,
    pub Arc<str>,
    pub u16,
    pub String,
    pub String,
    pub Option<u16>,
    pub ContentKind,
    pub Option<Arc<str>>,
    pub f64,
    pub Option<u32>,
    pub u64,
    pub u64,
    pub bool,
    pub HttpVersion,
    pub TlsStatus,
    pub Option<Arc<str>>,
    pub Option<SentBy>,
    pub bool,
    pub Option<u8>,
);

impl From<&FlowSummary> for FlowRow {
    fn from(f: &FlowSummary) -> Self {
        FlowRow(
            f.id,
            f.state,
            f.scheme,
            f.method.clone(),
            f.host.clone(),
            f.port,
            f.path.clone(),
            f.query.clone(),
            f.status,
            f.content_kind,
            f.client.clone(),
            f.started_at_ms,
            f.duration_ms,
            f.req_size,
            f.resp_size,
            f.edited,
            f.http_version,
            f.tls,
            f.graphql_op.clone(),
            f.sent_by,
            f.pinned,
            f.grpc_status,
        )
    }
}

/// A window of rows returned by `get_rows(view, start, count)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RowsPage {
    pub view: ViewId,
    pub start: u32,
    /// View length at the time of the read.
    pub len: u32,
    pub rows: Vec<FlowRow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_row_serializes_as_a_tuple() {
        let summary = FlowSummary {
            id: 7,
            state: FlowState::Completed,
            scheme: Scheme::Https,
            method: "GET".into(),
            host: "api.example.com".into(),
            port: 443,
            path: "/v1/users".into(),
            query: "page=2".into(),
            status: Some(200),
            content_kind: ContentKind::Json,
            client: Some("Safari".into()),
            started_at_ms: 1.5,
            duration_ms: Some(42),
            req_size: 0,
            resp_size: 1024,
            edited: false,
            http_version: HttpVersion::Http2,
            tls: TlsStatus::Intercepted,
            graphql_op: Some("GetUser".into()),
            sent_by: Some(SentBy::User),
            pinned: true,
            grpc_status: Some(5),
        };
        let json = serde_json::to_string(&FlowRow::from(&summary)).unwrap();
        assert_eq!(
            json,
            r#"[7,"completed","https","GET","api.example.com",443,"/v1/users","page=2",200,"json","Safari",1.5,42,0,1024,false,"HTTP/2","intercepted","GetUser","user",true,5]"#
        );
    }
}
