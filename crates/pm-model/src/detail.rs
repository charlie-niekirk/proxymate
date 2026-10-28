use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::flow::{ContentKind, FlowId};
use crate::rules::FlowRules;
use crate::tls::{Timing, TlsInfo};

/// One header line. h1 names arrive lower-cased from the parser; the proxy
/// still forwards them with their original case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl Header {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Header {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
pub enum HttpVersion {
    #[serde(rename = "HTTP/1.0")]
    Http10,
    #[serde(rename = "HTTP/1.1")]
    Http11,
    #[serde(rename = "HTTP/2")]
    Http2,
}

impl HttpVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            HttpVersion::Http10 => "HTTP/1.0",
            HttpVersion::Http11 => "HTTP/1.1",
            HttpVersion::Http2 => "HTTP/2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct RequestHead {
    pub method: String,
    /// Absolute URL as the client asked for it.
    pub url: String,
    pub version: HttpVersion,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ResponseHead {
    pub status: u16,
    /// Reason phrase from the status line, or the canonical one.
    pub reason: String,
    pub version: HttpVersion,
    pub headers: Vec<Header>,
}

/// What was seen and recorded of one body. The bytes themselves are fetched
/// separately (`get_body`), still content-encoded unless decoding is asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BodyInfo {
    /// Bytes that went over the wire, content-encoded.
    pub size: u64,
    /// Bytes kept by the recorder: `size` capped at the capture limit.
    pub captured: u64,
    /// True if `size` exceeded the capture limit. Forwarding is unaffected.
    pub truncated: bool,
    /// `Content-Encoding` of the body (e.g. `gzip`, `br`), if any.
    pub encoding: Option<String>,
    /// False while the body is still streaming, or if it was cut short.
    pub complete: bool,
    /// What the body holds, from its `Content-Type`, magic bytes and a text
    /// heuristic. `None` until the body has been seen.
    pub kind: ContentKind,
    /// The MIME type the body is served as for previews (`pmbody://`): the
    /// sniffed format when the bytes contradict the header, else the
    /// header's type without parameters.
    pub mime: Option<String>,
}

/// Everything about one flow beyond its summary: heads, body metadata and
/// errors. Loaded on demand for the inspector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FlowDetail {
    pub id: FlowId,
    pub request: RequestHead,
    pub response: Option<ResponseHead>,
    pub request_body: BodyInfo,
    pub response_body: BodyInfo,
    /// Why the flow failed, if it did.
    pub error: Option<String>,
    /// Present for HTTPS flows that went through the MITM.
    pub tls: Option<TlsInfo>,
    pub timing: Timing,
    /// What rules did to the flow.
    #[serde(default)]
    pub rules: FlowRules,
    /// HTTP/2 response trailers (gRPC's `grpc-status` and friends). Last:
    /// session format 3 appended it.
    #[serde(default)]
    pub trailers: Option<Vec<Header>>,
}

/// Which body of a flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BodyPart {
    Request,
    Response,
}

/// State of the proxy listener, for the toolbar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct ProxyStatus {
    /// `ip:port` the proxy listens on, once bound.
    pub listening: Option<String>,
    /// Why the proxy is not listening, if it failed to start.
    pub error: Option<String>,
    /// False while recording is paused: traffic is forwarded but not kept.
    pub recording: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_serialize_as_protocol_strings() {
        assert_eq!(serde_json::to_string(&HttpVersion::Http11).unwrap(), r#""HTTP/1.1""#);
        assert_eq!(HttpVersion::Http2.as_str(), "HTTP/2");
    }
}
