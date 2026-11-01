//! Charles session import (M20): the JSON (`.chlsj`) and XML (`.chlsx`)
//! exports open as documents, like HAR files. Charles's own `.chls` is Java
//! serialisation and isn't read; [`BINARY_UNSUPPORTED`] says how to export
//! one of the others.
//!
//! Both formats describe the same transaction: `scheme`/`protocol`,
//! `host`, `actualPort`/`port`, `path`, `query`, the method and protocol
//! version, a status (`COMPLETE`, `FAILED`…), times, and a request and
//! response with their header lines and a body. A body given as `text` was
//! decoded by Charles, so it is stored without a content-encoding; one
//! given base64 (`encoded`, or `encoding="base64"`) is what came over the
//! wire unless it says `decoded`.

use base64::Engine as _;
use pm_model::{
    BodyInfo, ContentKind, FlowDetail, FlowState, FlowSummary, Header, HttpVersion, RequestHead, ResponseHead, Scheme,
    Timing, TlsStatus,
};
use serde::Deserialize;

use super::content::{self, SNIFF_LEN};
use super::decode;
use super::har::{parse_time, parse_version};
use crate::store::SavedFlow;

/// Shown when a binary `.chls` session is opened.
pub const BINARY_UNSUPPORTED: &str = "Binary Charles sessions (.chls) can't be read. In Charles, choose File › \
     Export Session… and pick JSON Session File (.chlsj) or XML Session File (.chlsx), then open that.";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a Charles JSON session: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Charles XML session: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("the Charles session has no transactions")]
    Empty,
}

/// What a file that isn't a saved session looks like.
pub(crate) enum Sniffed {
    /// A top-level JSON array: Charles JSON.
    Json,
    /// XML (Charles XML).
    Xml,
    /// Java serialisation or gzip: a binary `.chls`.
    Binary,
    /// Anything else (HAR is a JSON object).
    Other,
}

pub(crate) fn sniff(bytes: &[u8]) -> Sniffed {
    if bytes.starts_with(&[0xAC, 0xED]) || bytes.starts_with(&[0x1F, 0x8B]) {
        return Sniffed::Binary;
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'[') => Sniffed::Json,
        Some(b'<') => Sniffed::Xml,
        _ => Sniffed::Other,
    }
}

/// One transaction, whichever format it came from.
#[derive(Default)]
struct Transaction {
    status: String,
    method: String,
    version: String,
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
    query: String,
    tunnel: bool,
    error: Option<String>,
    start: Option<String>,
    /// Milliseconds since the epoch, when the format gives it as a number.
    start_ms: Option<f64>,
    total_ms: Option<f64>,
    dns_ms: Option<f64>,
    connect_ms: Option<f64>,
    ssl_ms: Option<f64>,
    latency_ms: Option<f64>,
    response_ms: Option<f64>,
    request: Option<Message>,
    response: Option<Message>,
}

#[derive(Default)]
struct Message {
    status: Option<u16>,
    reason: String,
    headers: Vec<Header>,
    body: Vec<u8>,
    /// The body is as it came over the wire, in this content-encoding.
    encoding: Option<String>,
    mime: Option<String>,
    size: Option<u64>,
}

// ---- JSON ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonTxn {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    protocol_version: Option<String>,
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<i64>,
    #[serde(default)]
    actual_port: Option<i64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    tunnel: bool,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    times: Option<JsonTimes>,
    #[serde(default)]
    durations: Option<JsonDurations>,
    #[serde(default)]
    request: Option<JsonMessage>,
    #[serde(default)]
    response: Option<JsonMessage>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonTimes {
    #[serde(default)]
    start: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonDurations {
    #[serde(default)]
    total: Option<f64>,
    #[serde(default)]
    dns: Option<f64>,
    #[serde(default)]
    connect: Option<f64>,
    #[serde(default)]
    ssl: Option<f64>,
    #[serde(default)]
    latency: Option<f64>,
    #[serde(default)]
    response: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonMessage {
    #[serde(default)]
    status: Option<i64>,
    #[serde(default)]
    sizes: Option<JsonSizes>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    content_encoding: Option<String>,
    #[serde(default)]
    header: Option<JsonHeader>,
    #[serde(default)]
    body: Option<JsonBody>,
}

#[derive(Deserialize, Default)]
struct JsonSizes {
    #[serde(default)]
    body: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonHeader {
    #[serde(default)]
    first_line: Option<String>,
    #[serde(default)]
    headers: Vec<JsonNameValue>,
}

#[derive(Deserialize)]
struct JsonNameValue {
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize, Default)]
struct JsonBody {
    #[serde(default)]
    text: Option<String>,
    /// Base64.
    #[serde(default)]
    encoded: Option<String>,
    #[serde(default)]
    decoded: bool,
}

/// The flows of a Charles JSON session, in its order.
pub(crate) fn import_json(bytes: &[u8]) -> Result<Vec<SavedFlow>, ImportError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let txns: Vec<JsonTxn> = serde_json::from_slice(bytes)?;
    if txns.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(txns.into_iter().map(|t| flow(from_json(t))).collect())
}

fn from_json(t: JsonTxn) -> Transaction {
    let port = |p: Option<i64>| p.and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0);
    let d = t.durations.unwrap_or_default();
    let known = |v: Option<f64>| v.filter(|v| *v >= 0.0);
    Transaction {
        status: t.status.unwrap_or_default(),
        method: t.method.unwrap_or_default(),
        version: t.protocol_version.unwrap_or_default(),
        scheme: t.scheme.unwrap_or_default(),
        host: t.host.unwrap_or_default(),
        port: port(t.actual_port).or(port(t.port)),
        path: t.path.unwrap_or_default(),
        query: t.query.unwrap_or_default(),
        tunnel: t.tunnel,
        error: t.error_message,
        start: t.times.and_then(|t| t.start),
        start_ms: None,
        total_ms: known(d.total),
        dns_ms: known(d.dns),
        connect_ms: known(d.connect),
        ssl_ms: known(d.ssl),
        latency_ms: known(d.latency),
        response_ms: known(d.response),
        request: t.request.map(json_message),
        response: t.response.map(json_message),
    }
}

fn json_message(m: JsonMessage) -> Message {
    let header = m.header.unwrap_or_default();
    let headers: Vec<Header> = header
        .headers
        .into_iter()
        .filter(|h| !h.name.is_empty() && !h.name.starts_with(':'))
        .map(|h| Header::new(h.name, h.value))
        .collect();
    let (body, raw) = match m.body {
        Some(JsonBody {
            encoded: Some(b64),
            decoded,
            ..
        }) => (base64(&b64), !decoded),
        Some(JsonBody { text: Some(text), .. }) => (text.into_bytes(), false),
        _ => (Vec::new(), false),
    };
    let encoding = raw.then(|| {
        m.content_encoding
            .clone()
            .or_else(|| header_value(&headers, "content-encoding"))
    });
    Message {
        status: m.status.and_then(|s| u16::try_from(s).ok()).filter(|s| *s > 0),
        reason: header.first_line.as_deref().map(reason_of).unwrap_or_default(),
        encoding: encoding.flatten().and_then(|e| normal_encoding(&e)),
        mime: m.mime_type.filter(|m| !m.is_empty()),
        size: m.sizes.and_then(|s| s.body).and_then(|b| u64::try_from(b).ok()),
        headers,
        body,
    }
}

// ---- XML ----

/// The flows of a Charles XML session: every `transaction` element, in
/// document order.
pub(crate) fn import_xml(bytes: &[u8]) -> Result<Vec<SavedFlow>, ImportError> {
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(&text, options)?;
    let flows: Vec<SavedFlow> = doc
        .descendants()
        .filter(|n| n.has_tag_name("transaction"))
        .map(|n| flow(from_xml(n)))
        .collect();
    if flows.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(flows)
}

fn from_xml(t: roxmltree::Node) -> Transaction {
    let attr = |name: &str| t.attribute(name).map(str::to_owned);
    let number = |name: &str| t.attribute(name).and_then(|v| v.trim().parse::<f64>().ok());
    let known = |name: &str| number(name).filter(|v| *v >= 0.0);
    let port = |name: &str| number(name).filter(|p| *p > 0.0 && *p <= 65535.0).map(|p| p as u16);
    let child = |name: &str| t.children().find(|c| c.has_tag_name(name));
    let start_ms = number("startTimeMillis");
    let end_ms = number("endTimeMillis");
    Transaction {
        status: attr("status").unwrap_or_default(),
        method: attr("method").unwrap_or_default(),
        version: attr("protocolVersion").unwrap_or_default(),
        scheme: attr("protocol").or_else(|| attr("scheme")).unwrap_or_default(),
        host: attr("host").unwrap_or_default(),
        port: port("actualPort").or_else(|| port("port")),
        path: attr("path").unwrap_or_default(),
        query: attr("query").unwrap_or_default(),
        tunnel: attr("tunnel").is_some_and(|v| v == "true"),
        error: attr("errorMessage").or_else(|| child("error").and_then(|e| e.text()).map(str::to_owned)),
        start: attr("startTime"),
        start_ms,
        total_ms: match (start_ms, end_ms) {
            (Some(s), Some(e)) if e >= s => Some(e - s),
            _ => None,
        },
        dns_ms: known("dnsDuration"),
        connect_ms: known("connectDuration"),
        ssl_ms: known("sslDuration"),
        latency_ms: known("latency"),
        response_ms: known("responseDuration"),
        request: child("request").map(xml_message),
        response: child("response").map(xml_message),
    }
}

fn xml_message(m: roxmltree::Node) -> Message {
    let headers_node = m.children().find(|c| c.has_tag_name("headers"));
    let first_line = headers_node
        .and_then(|h| h.children().find(|c| c.has_tag_name("first-line")))
        .and_then(|f| f.text())
        .unwrap_or_default();
    let headers: Vec<Header> = headers_node
        .into_iter()
        .flat_map(|h| h.children().filter(|c| c.has_tag_name("header")))
        .filter_map(|h| {
            let text = |name: &str| h.children().find(|c| c.has_tag_name(name)).and_then(|c| c.text());
            let name = text("name")?.trim();
            (!name.is_empty() && !name.starts_with(':')).then(|| Header::new(name, text("value").unwrap_or_default()))
        })
        .collect();
    let (body, raw) = match m.children().find(|c| c.has_tag_name("body")) {
        Some(b) => {
            let text = b.text().unwrap_or_default();
            if b.attribute("encoding")
                .is_some_and(|e| e.eq_ignore_ascii_case("base64"))
            {
                (base64(text), b.attribute("decoded") != Some("true"))
            } else {
                (text.as_bytes().to_vec(), false)
            }
        }
        None => (Vec::new(), false),
    };
    let encoding = raw.then(|| {
        m.attribute("content-encoding")
            .map(str::to_owned)
            .or_else(|| header_value(&headers, "content-encoding"))
    });
    Message {
        status: m
            .attribute("status")
            .and_then(|s| s.trim().parse().ok())
            .filter(|s| *s > 0),
        reason: reason_of(first_line),
        encoding: encoding.flatten().and_then(|e| normal_encoding(&e)),
        mime: m.attribute("mime-type").filter(|m| !m.is_empty()).map(str::to_owned),
        size: None,
        headers,
        body,
    }
}

// ---- Both ----

fn base64(text: &str) -> Vec<u8> {
    let clean: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .unwrap_or_else(|_| text.as_bytes().to_vec())
}

fn header_value(headers: &[Header], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
}

fn normal_encoding(e: &str) -> Option<String> {
    let e = e.trim().to_ascii_lowercase();
    match e.as_str() {
        "" | "identity" | "none" => None,
        "brotli" => Some("br".into()),
        _ => Some(e),
    }
}

/// `HTTP/1.1 404 Not Found` → `Not Found`.
fn reason_of(first_line: &str) -> String {
    let mut parts = first_line.trim().splitn(3, ' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(v), Some(code), Some(reason)) if v.starts_with("HTTP") && code.parse::<u16>().is_ok() => {
            reason.trim().to_owned()
        }
        _ => String::new(),
    }
}

fn flow(t: Transaction) -> SavedFlow {
    let scheme = match t.scheme.to_ascii_lowercase().as_str() {
        "https" => Scheme::Https,
        "ws" => Scheme::Ws,
        "wss" => Scheme::Wss,
        _ => Scheme::Http,
    };
    let host = t.host.to_ascii_lowercase();
    let port = t.port.unwrap_or(if scheme.is_tls() { 443 } else { 80 });
    let method = match t.method.trim() {
        "" if t.tunnel => "CONNECT".to_owned(),
        "" => "GET".to_owned(),
        m => m.to_ascii_uppercase(),
    };
    let path = if t.path.is_empty() {
        "/".to_owned()
    } else {
        t.path.clone()
    };
    let query = t.query.trim_start_matches('?').to_owned();
    let version = parse_version(&t.version);
    let authority = if port == if scheme.is_tls() { 443 } else { 80 } {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    let url = match (method.as_str(), query.is_empty()) {
        ("CONNECT", _) => authority.clone(),
        (_, true) => format!("{}://{authority}{path}", scheme.as_str()),
        (_, false) => format!("{}://{authority}{path}?{query}", scheme.as_str()),
    };
    let request = t.request.unwrap_or_default();
    let response = t.response;
    let failed = t.error.is_some()
        || matches!(
            t.status.to_ascii_uppercase().as_str(),
            "FAILED" | "ERROR" | "INCOMPLETE" | "EXCEPTION"
        );
    let status = response.as_ref().and_then(|r| r.status);
    let state = if t.tunnel || method == "CONNECT" {
        FlowState::Tunnel
    } else if status.is_some() && !failed {
        FlowState::Completed
    } else {
        FlowState::Failed
    };
    let error = match (&t.error, state) {
        (Some(e), _) if !e.is_empty() => Some(e.clone()),
        (_, FlowState::Failed) => Some(format!("Charles recorded this transaction as {}.", t.status)),
        _ => None,
    };

    let detect = |m: &Message| {
        let decoded = match &m.encoding {
            Some(e) if !m.body.is_empty() => decode::decode(e, &m.body).unwrap_or_else(|_| m.body.clone()),
            _ => m.body.clone(),
        };
        let ct = header_value(&m.headers, "content-type").or_else(|| m.mime.clone());
        let head = &decoded[..decoded.len().min(SNIFF_LEN)];
        (content::detect(ct.as_deref(), head), ct, decoded)
    };
    let (request_detected, request_type, request_decoded) = detect(&request);
    let graphql = (request_decoded.len() <= content::GRAPHQL_MAX)
        .then(|| content::graphql_request(request_type.as_deref(), &request_decoded, &query))
        .flatten();
    let request_kind = if graphql.is_some() {
        ContentKind::GraphQl
    } else {
        request_detected.kind
    };
    let response_detected = response.as_ref().map(|r| detect(r).0);
    let response_kind = response_detected.as_ref().map_or(ContentKind::None, |d| d.kind);

    let start = t
        .start_ms
        .or_else(|| t.start.as_deref().and_then(parse_time))
        .unwrap_or(0.0);
    let body_info = |m: &Message, detected: content::Detected| BodyInfo {
        size: m.size.unwrap_or(0).max(m.body.len() as u64),
        captured: m.body.len() as u64,
        truncated: false,
        encoding: m.encoding.clone(),
        complete: true,
        kind: if m.body.is_empty() {
            ContentKind::None
        } else {
            detected.kind
        },
        mime: detected.mime,
    };
    let summary = FlowSummary {
        id: 0,
        state,
        scheme,
        method: method.as_str().into(),
        host: host.as_str().into(),
        port,
        path: path.clone(),
        query: query.clone(),
        status: status.filter(|_| state != FlowState::Tunnel),
        content_kind: content::flow_kind(request_kind, response_kind),
        client: None,
        started_at_ms: start,
        duration_ms: t.total_ms.map(|d| d.round() as u32),
        req_size: request.size.unwrap_or(request.body.len() as u64),
        resp_size: response.as_ref().map_or(0, |r| r.size.unwrap_or(r.body.len() as u64)),
        edited: false,
        http_version: version,
        tls: match (scheme.is_tls(), state) {
            (true, FlowState::Tunnel) => TlsStatus::Passthrough,
            (true, _) => TlsStatus::Intercepted,
            _ => TlsStatus::None,
        },
        graphql_op: graphql.and_then(|g| g.operation).map(Into::into),
        sent_by: None,
        pinned: false,
        grpc_status: None,
        label: None,
        comment: None,
        extra: Vec::new(),
    };
    let ssl = t.ssl_ms;
    let detail = FlowDetail {
        id: 0,
        request: RequestHead {
            method,
            url,
            version,
            headers: request.headers.clone(),
        },
        response: response.as_ref().filter(|r| r.status.is_some()).map(|r| ResponseHead {
            status: r.status.unwrap_or_default(),
            reason: r.reason.clone(),
            version: if version == HttpVersion::Http2 {
                version
            } else {
                HttpVersion::Http11
            },
            headers: r.headers.clone(),
        }),
        request_body: body_info(&request, request_detected),
        response_body: match (&response, response_detected) {
            (Some(r), Some(d)) => body_info(r, d),
            _ => BodyInfo {
                complete: true,
                ..Default::default()
            },
        },
        error,
        tls: None,
        timing: Timing {
            dns_ms: t.dns_ms,
            connect_ms: t.connect_ms,
            tls_ms: ssl,
            ttfb_ms: t.latency_ms.filter(|_| status.is_some()),
            transfer_ms: t.response_ms.filter(|_| status.is_some()),
            reused_connection: t.connect_ms.is_none() && t.dns_ms.is_none(),
        },
        rules: Default::default(),
        trailers: None,
        via: None,
    };
    SavedFlow {
        summary,
        detail,
        request_body: request.body,
        response_body: response.map(|r| r.body).unwrap_or_default(),
        ws: None,
        sse: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_formats() {
        assert!(matches!(sniff(b"  [{}]"), Sniffed::Json));
        assert!(matches!(sniff(b"\xEF\xBB\xBF<?xml"), Sniffed::Xml));
        assert!(matches!(sniff(&[0xAC, 0xED, 0, 5]), Sniffed::Binary));
        assert!(matches!(sniff(b"{\"log\":{}}"), Sniffed::Other));
    }

    #[test]
    fn reasons_come_from_the_first_line() {
        assert_eq!(reason_of("HTTP/1.1 404 Not Found"), "Not Found");
        assert_eq!(reason_of("HTTP/2.0 200 "), "");
        assert_eq!(reason_of("GET / HTTP/1.1"), "");
    }

    #[test]
    fn an_empty_session_is_an_error() {
        assert!(matches!(import_json(b"[]"), Err(ImportError::Empty)));
        assert!(matches!(
            import_xml(b"<charles-session></charles-session>"),
            Err(ImportError::Empty)
        ));
    }
}
