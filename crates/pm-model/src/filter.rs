use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::flow::ContentKind;

/// Protocol chip in the filter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolFilter {
    #[default]
    All,
    Http,
    Https,
    WebSocket,
}

/// Content chip in the filter row. Each group covers one or more [`ContentKind`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ContentGroup {
    Json,
    Form,
    Xml,
    Js,
    Css,
    GraphQl,
    Document,
    Media,
    Other,
}

impl ContentGroup {
    /// The chip a content kind belongs to.
    pub const fn of(kind: ContentKind) -> ContentGroup {
        match kind {
            ContentKind::Json => ContentGroup::Json,
            ContentKind::FormUrlEncoded | ContentKind::Multipart => ContentGroup::Form,
            ContentKind::Xml => ContentGroup::Xml,
            ContentKind::Javascript => ContentGroup::Js,
            ContentKind::Css => ContentGroup::Css,
            ContentKind::GraphQl => ContentGroup::GraphQl,
            ContentKind::Html | ContentKind::Pdf | ContentKind::Text => ContentGroup::Document,
            ContentKind::Image | ContentKind::Svg | ContentKind::Video | ContentKind::Audio | ContentKind::Font => {
                ContentGroup::Media
            }
            ContentKind::None
            | ContentKind::Binary
            | ContentKind::Grpc
            | ContentKind::Protobuf
            | ContentKind::MessagePack
            | ContentKind::Cbor => ContentGroup::Other,
        }
    }
}

/// Sortable flow table columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    /// Arrival order. The only key that supports cheap incremental appends.
    #[default]
    Id,
    Url,
    Method,
    Status,
    Duration,
    ReqSize,
    RespSize,
    /// A header column (M17): `SortSpec.extra` says which.
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Type)]
pub struct SortSpec {
    pub key: SortKey,
    pub dir: SortDir,
    /// The header column `SortKey::Extra` sorts by.
    #[serde(default)]
    pub extra: u8,
}

/// What a view shows. Shared by the filter row, the sidebar and (from M7) MCP.
///
/// `text` is the search box, in the filter DSL (`pm-core/src/store/query.rs`:
/// `host:*.example.com status:4xx dur>500 …`); plain words are matched as a
/// case-insensitive substring of `host + path + query`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct FilterSpec {
    pub text: String,
    pub protocol: ProtocolFilter,
    /// Content chips; empty means all content.
    pub content: Vec<ContentGroup>,
    /// Restrict to one host (sidebar Domains selection). `*.example.com`
    /// matches `example.com` and every host under it.
    pub host: Option<String>,
    /// Restrict to one client app (sidebar Apps selection).
    pub client: Option<String>,
}

/// Where a deep search looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    /// `host[:port]/path?query`.
    Url,
    RequestHeaders,
    ResponseHeaders,
    /// Decoded, the first 1 MB.
    RequestBody,
    /// Decoded, the first 1 MB.
    ResponseBody,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_spec_round_trips_through_json() {
        let spec = FilterSpec {
            text: "api".into(),
            protocol: ProtocolFilter::Https,
            content: vec![ContentGroup::Json, ContentGroup::GraphQl],
            host: Some("example.com".into()),
            client: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"text":"api","protocol":"https","content":["json","graphql"],"host":"example.com","client":null}"#
        );
        assert_eq!(serde_json::from_str::<FilterSpec>(&json).unwrap(), spec);
    }

    #[test]
    fn every_content_kind_has_a_group() {
        assert_eq!(ContentGroup::of(ContentKind::Multipart), ContentGroup::Form);
        assert_eq!(ContentGroup::of(ContentKind::Svg), ContentGroup::Media);
        assert_eq!(ContentGroup::of(ContentKind::None), ContentGroup::Other);
    }
}
