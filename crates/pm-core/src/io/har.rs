//! HAR 1.2 import and export, with our own serde types.
//!
//! Export writes one entry per flow: bodies decoded (text as is, binary as
//! base64), timings from the flow's phases, and a WebSocket flow's messages
//! as Chrome's `_webSocketMessages`. Import reads what browsers and other
//! tools write, Chrome DevTools' extensions included; fields it doesn't
//! know are ignored and missing ones get defaults. Imported bodies are the
//! decoded text the HAR holds, so they are stored without a
//! content-encoding even if the headers name one.

use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use pm_model::{
    BodyInfo, ContentKind, FlowDetail, FlowState, FlowSummary, Header, HttpVersion, RequestHead, ResponseHead, Scheme,
    SentBy, Timing, TlsStatus, WsDirection, WsMessage, WsOpcode,
};
use serde::{Deserialize, Serialize};

use super::content::{self, GRAPHQL_MAX, SNIFF_LEN};
use super::decode;
use crate::store::{SavedFlow, SavedWs};

/// Characters of a text message shown in the WebSocket list.
const PREVIEW_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Har {
    pub log: Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub creator: Creator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<Creator>,
    #[serde(default)]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Creator {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    #[serde(default)]
    pub started_date_time: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub page_timings: PageTimings,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTimings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_content_load: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_load: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pageref: Option<String>,
    pub started_date_time: String,
    /// Total time in milliseconds.
    #[serde(default)]
    pub time: f64,
    pub request: Request,
    pub response: Response,
    #[serde(default)]
    pub cache: Cache,
    #[serde(default)]
    pub timings: Timings,
    #[serde(rename = "serverIPAddress", default, skip_serializing_if = "Option::is_none")]
    pub server_ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Chrome DevTools: the messages of a WebSocket.
    #[serde(rename = "_webSocketMessages", default, skip_serializing_if = "Option::is_none")]
    pub web_socket_messages: Option<Vec<WebSocketMessage>>,
    /// Chrome DevTools: `websocket`, `fetch`, `document`, …
    #[serde(rename = "_resourceType", default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub http_version: String,
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub headers: Vec<NameValue>,
    #[serde(default)]
    pub query_string: Vec<NameValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_data: Option<PostData>,
    #[serde(default = "unknown")]
    pub headers_size: i64,
    #[serde(default = "unknown")]
    pub body_size: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub http_version: String,
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub headers: Vec<NameValue>,
    #[serde(default)]
    pub content: Content,
    #[serde(rename = "redirectURL", default)]
    pub redirect_url: String,
    #[serde(default = "unknown")]
    pub headers_size: i64,
    #[serde(default = "unknown")]
    pub body_size: i64,
    /// Chrome DevTools: bytes on the wire, headers included.
    #[serde(rename = "_transferSize", default, skip_serializing_if = "Option::is_none")]
    pub transfer_size: Option<i64>,
    /// Chrome DevTools: why the request failed (`net::ERR_…`).
    #[serde(rename = "_error", default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    #[serde(default)]
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<i64>,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `base64` for binary bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostData {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    #[serde(default)]
    pub text: Option<String>,
    /// Not in HAR 1.2, but written by several tools (and by us) for binary
    /// request bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Param {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NameValue {
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
}

/// Always empty: we don't record the browser cache.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cache {}

/// Phase times in milliseconds; -1 when the phase doesn't apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timings {
    #[serde(default = "unknown_f")]
    pub blocked: f64,
    #[serde(default = "unknown_f")]
    pub dns: f64,
    /// Includes `ssl`.
    #[serde(default = "unknown_f")]
    pub connect: f64,
    #[serde(default)]
    pub send: f64,
    #[serde(default)]
    pub wait: f64,
    #[serde(default)]
    pub receive: f64,
    #[serde(default = "unknown_f")]
    pub ssl: f64,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            blocked: -1.0,
            dns: -1.0,
            connect: -1.0,
            send: 0.0,
            wait: 0.0,
            receive: 0.0,
            ssl: -1.0,
        }
    }
}

/// Chrome DevTools' record of one WebSocket message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSocketMessage {
    /// `send` or `receive`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Seconds since the Unix epoch.
    pub time: f64,
    pub opcode: u8,
    /// Text as is; binary payloads as base64.
    #[serde(default)]
    pub data: String,
}

fn unknown() -> i64 {
    -1
}

fn unknown_f() -> f64 {
    -1.0
}

// ---- Export ----

/// A HAR document of `flows`.
pub(crate) fn export(flows: impl IntoIterator<Item = SavedFlow>) -> Har {
    let mut log = new_log();
    log.entries = flows.into_iter().map(|f| entry(&f)).collect();
    Har { log }
}

fn new_log() -> Log {
    Log {
        version: "1.2".into(),
        creator: Creator {
            name: "Proxymate".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        browser: None,
        pages: Vec::new(),
        entries: Vec::new(),
        comment: None,
    }
}

/// Writes `flows` as a HAR file, one entry per line, without holding them
/// all in memory. `progress` hears how many are written.
pub(crate) fn write(
    path: &Path,
    flows: impl Iterator<Item = io::Result<SavedFlow>>,
    progress: &mut dyn FnMut(u32),
) -> io::Result<u32> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let partial = dir.join(format!(".{}.partial", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut out = BufWriter::new(std::fs::File::create(&partial)?);
        let mut head = serde_json::to_string(&Har { log: new_log() }).map_err(io::Error::other)?;
        // `{"log":{…,"entries":[]}}`: open the entries array.
        let tail = head.split_off(head.rfind("[]").expect("an empty entries array") + 1);
        out.write_all(head.as_bytes())?;
        let mut n = 0u32;
        for flow in flows {
            out.write_all(if n == 0 { b"\n" } else { b",\n" })?;
            serde_json::to_writer(&mut out, &entry(&flow?)).map_err(io::Error::other)?;
            n += 1;
            if n.is_multiple_of(super::session::PROGRESS_EVERY) {
                progress(n);
            }
        }
        out.write_all(b"\n")?;
        out.write_all(tail.as_bytes())?;
        out.write_all(b"\n")?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&partial, path)?;
        progress(n);
        Ok(n)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn entry(flow: &SavedFlow) -> Entry {
    let s = &flow.summary;
    let d = &flow.detail;
    let request_headers = name_values(&d.request.headers);
    let request_body = decoded(&flow.request_body, d.request_body.encoding.as_deref());
    let post_data = (!request_body.is_empty()).then(|| {
        let mime_type = header(&d.request.headers, "content-type")
            .unwrap_or_default()
            .to_owned();
        let (text, encoding) = text_or_base64(&request_body);
        let params = match content::kind_from_content_type(&mime_type) {
            ContentKind::FormUrlEncoded => form_params(&text),
            _ => Vec::new(),
        };
        PostData {
            mime_type,
            params,
            text: Some(text),
            encoding,
        }
    });
    let response = match &d.response {
        Some(r) => {
            let body = decoded(&flow.response_body, d.response_body.encoding.as_deref());
            let (text, encoding) = text_or_base64(&body);
            let wire = d.response_body.size as i64;
            Response {
                status: i64::from(r.status),
                status_text: r.reason.clone(),
                http_version: version_text(r.version).into(),
                cookies: set_cookies(&r.headers),
                headers: name_values(&r.headers),
                content: Content {
                    size: body.len() as i64,
                    compression: d
                        .response_body
                        .encoding
                        .is_some()
                        .then(|| body.len() as i64 - flow.response_body.len() as i64),
                    mime_type: header(&r.headers, "content-type").unwrap_or("x-unknown").to_owned(),
                    text: (!body.is_empty()).then_some(text),
                    encoding: if body.is_empty() { None } else { encoding },
                },
                redirect_url: header(&r.headers, "location").unwrap_or_default().to_owned(),
                headers_size: -1,
                body_size: wire,
                transfer_size: None,
                error: d.error.clone(),
            }
        }
        None => Response {
            status: 0,
            status_text: String::new(),
            http_version: String::new(),
            cookies: Vec::new(),
            headers: Vec::new(),
            content: Content {
                size: 0,
                compression: None,
                mime_type: "x-unknown".into(),
                text: None,
                encoding: None,
            },
            redirect_url: String::new(),
            headers_size: -1,
            body_size: -1,
            transfer_size: None,
            error: Some(d.error.clone().unwrap_or_else(|| "no response".into())),
        },
    };
    let t = &d.timing;
    let ms = |v: Option<f64>| v.map_or(-1.0, round3);
    let timings = Timings {
        blocked: -1.0,
        dns: ms(t.dns_ms),
        connect: match (t.connect_ms, t.tls_ms) {
            (None, None) => -1.0,
            (connect, tls) => round3(connect.unwrap_or(0.0) + tls.unwrap_or(0.0)),
        },
        send: 0.0,
        wait: t.ttfb_ms.map_or(0.0, round3),
        receive: t.transfer_ms.map_or(0.0, round3),
        ssl: ms(t.tls_ms),
    };
    let phases: f64 = [
        timings.blocked,
        timings.dns,
        timings.connect,
        timings.send,
        timings.wait,
        timings.receive,
    ]
    .iter()
    .filter(|v| **v > 0.0)
    .sum();
    Entry {
        pageref: None,
        started_date_time: format_time(s.started_at_ms),
        time: s.duration_ms.map_or(round3(phases), f64::from),
        request: Request {
            method: d.request.method.clone(),
            url: d.request.url.clone(),
            http_version: version_text(d.request.version).into(),
            cookies: cookies(&d.request.headers),
            headers: request_headers,
            query_string: query_params(&d.request.url),
            post_data,
            headers_size: -1,
            body_size: if flow.request_body.is_empty() && d.request_body.size == 0 {
                0
            } else {
                d.request_body.size as i64
            },
        },
        response,
        cache: Cache {},
        timings,
        server_ip_address: None,
        connection: None,
        web_socket_messages: flow.ws.as_ref().map(|ws| {
            // What went over the wire: a dropped message never did.
            ws.messages
                .iter()
                .filter(|(m, _)| !m.dropped)
                .map(|(m, payload)| WebSocketMessage {
                    kind: match m.direction {
                        WsDirection::Sent => "send",
                        WsDirection::Received => "receive",
                    }
                    .into(),
                    time: m.time_ms / 1000.0,
                    opcode: opcode_number(m.opcode),
                    data: match m.opcode {
                        WsOpcode::Binary => base64::engine::general_purpose::STANDARD.encode(payload),
                        _ => String::from_utf8_lossy(payload).into_owned(),
                    },
                })
                .collect()
        }),
        resource_type: s.scheme.is_websocket().then(|| "websocket".into()),
        // The user's note wins over where the request came from.
        comment: s.comment.as_deref().map(str::to_owned).or_else(|| {
            s.sent_by.map(|by| {
                match by {
                    SentBy::User => "Sent from Proxymate",
                    SentBy::Agent => "Sent by an agent through Proxymate",
                    SentBy::Repeat => "Repeated by Proxymate",
                }
                .into()
            })
        }),
    }
}

/// The body decoded, or as recorded if it doesn't decode.
fn decoded(body: &[u8], encoding: Option<&str>) -> Vec<u8> {
    match encoding {
        Some(encoding) if !body.is_empty() => decode::decode(encoding, body).unwrap_or_else(|_| body.to_vec()),
        _ => body.to_vec(),
    }
}

fn text_or_base64(body: &[u8]) -> (String, Option<String>) {
    match std::str::from_utf8(body) {
        Ok(text) => (text.to_owned(), None),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(body),
            Some("base64".into()),
        ),
    }
}

fn name_values(headers: &[Header]) -> Vec<NameValue> {
    headers
        .iter()
        .map(|h| NameValue {
            name: h.name.clone(),
            value: h.value.clone(),
        })
        .collect()
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn cookies(headers: &[Header]) -> Vec<Cookie> {
    headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("cookie"))
        .flat_map(|h| h.value.split(';'))
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            Some(Cookie {
                name: name.trim().into(),
                value: value.trim().into(),
                path: None,
                domain: None,
                expires: None,
                http_only: None,
                secure: None,
            })
        })
        .collect()
}

fn set_cookies(headers: &[Header]) -> Vec<Cookie> {
    headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|h| {
            let mut parts = h.value.split(';');
            let (name, value) = parts.next()?.trim().split_once('=')?;
            let mut cookie = Cookie {
                name: name.trim().into(),
                value: value.trim().into(),
                path: None,
                domain: None,
                expires: None,
                http_only: None,
                secure: None,
            };
            for attr in parts {
                let (key, val) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
                match key.to_ascii_lowercase().as_str() {
                    "path" => cookie.path = Some(val.into()),
                    "domain" => cookie.domain = Some(val.into()),
                    "expires" => cookie.expires = Some(val.into()),
                    "httponly" => cookie.http_only = Some(true),
                    "secure" => cookie.secure = Some(true),
                    _ => {}
                }
            }
            Some(cookie)
        })
        .collect()
}

fn query_params(url: &str) -> Vec<NameValue> {
    let Some((_, query)) = url.split_once('?') else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or_default();
    form_pairs(query)
        .into_iter()
        .map(|(name, value)| NameValue { name, value })
        .collect()
}

fn form_params(text: &str) -> Vec<Param> {
    form_pairs(text)
        .into_iter()
        .map(|(name, value)| Param {
            name,
            value: Some(value),
            file_name: None,
            content_type: None,
        })
        .collect()
}

fn form_pairs(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(name), percent_decode(value))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn version_text(version: HttpVersion) -> &'static str {
    match version {
        HttpVersion::Http10 => "HTTP/1.0",
        HttpVersion::Http11 => "HTTP/1.1",
        HttpVersion::Http2 => "HTTP/2.0",
    }
}

fn opcode_number(opcode: WsOpcode) -> u8 {
    match opcode {
        WsOpcode::Text => 1,
        WsOpcode::Binary => 2,
        WsOpcode::Close => 8,
        WsOpcode::Ping => 9,
        WsOpcode::Pong => 10,
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// `2026-10-21T09:30:00.123Z`.
pub fn format_time(ms_since_epoch: f64) -> String {
    let total_ms = ms_since_epoch.round() as i64;
    let (secs, ms) = (total_ms.div_euclid(1000), total_ms.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Milliseconds since the epoch of an RFC 3339 time (`Z` or `±hh:mm`).
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, time) = s.split_once(['T', 't', ' '])?;
    let mut date = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let (clock, offset_min) = if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        (clock, 0)
    } else {
        let at = time.rfind(['+', '-'])?;
        let (clock, offset) = time.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (oh, om) = offset[1..]
            .split_once(':')
            .unwrap_or((&offset[1..3.min(offset.len())], "0"));
        (clock, sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?))
    };
    let mut parts = clock.splitn(3, ':');
    let (h, min) = (parts.next()?.parse::<i64>().ok()?, parts.next()?.parse::<i64>().ok()?);
    let sec: f64 = parts.next()?.parse().ok()?;
    let days = days_from_civil(y, m, d);
    let secs = days * 86_400 + h * 3600 + min * 60 - offset_min * 60;
    Some(secs as f64 * 1000.0 + sec * 1000.0)
}

// Howard Hinnant's civil calendar algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

// ---- Import ----

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a HAR file: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the HAR file has no entries")]
    Empty,
}

/// The flows of a HAR document, in its order. Entries whose URL can't be
/// understood are skipped.
pub(crate) fn import(bytes: &[u8]) -> Result<Vec<SavedFlow>, ImportError> {
    // A UTF-8 byte order mark is common in files saved on Windows.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let har: Har = serde_json::from_slice(bytes)?;
    if har.log.entries.is_empty() {
        return Err(ImportError::Empty);
    }
    Ok(har.log.entries.iter().filter_map(flow).collect())
}

fn flow(entry: &Entry) -> Option<SavedFlow> {
    let url = Url::parse(&entry.request.url)?;
    let messages = entry.web_socket_messages.as_deref().unwrap_or_default();
    let websocket = url.scheme.is_websocket()
        || entry.resource_type.as_deref() == Some("websocket")
        || entry.web_socket_messages.is_some();
    let scheme = match (websocket, url.scheme.is_tls()) {
        (true, true) => Scheme::Wss,
        (true, false) => Scheme::Ws,
        (false, true) => Scheme::Https,
        (false, false) => Scheme::Http,
    };
    let version = parse_version(&entry.request.http_version);
    let request_headers = headers(&entry.request.headers);
    let request_body = post_body(entry.request.post_data.as_ref());
    let request_type = header(&request_headers, "content-type").map(str::to_owned).or_else(|| {
        entry
            .request
            .post_data
            .as_ref()
            .map(|p| p.mime_type.clone())
            .filter(|m| !m.is_empty())
    });

    let r = &entry.response;
    let responded = r.status > 0;
    let response_headers = headers(&r.headers);
    let response_body = match (&r.content.text, r.content.encoding.as_deref()) {
        (Some(text), Some(e)) if e.eq_ignore_ascii_case("base64") => base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .unwrap_or_else(|_| text.clone().into_bytes()),
        (Some(text), _) => text.clone().into_bytes(),
        (None, _) => Vec::new(),
    };
    let response_type = header(&response_headers, "content-type")
        .map(str::to_owned)
        .or_else(|| Some(r.content.mime_type.clone()).filter(|m| !m.is_empty() && m != "x-unknown"));

    let request_detected = content::detect(
        request_type.as_deref(),
        &request_body[..request_body.len().min(SNIFF_LEN)],
    );
    let graphql = (request_body.len() <= GRAPHQL_MAX)
        .then(|| content::graphql_request(request_type.as_deref(), &request_body, &url.query))
        .flatten();
    let request_kind = if graphql.is_some() {
        ContentKind::GraphQl
    } else {
        request_detected.kind
    };
    let response_detected = content::detect(
        response_type.as_deref(),
        &response_body[..response_body.len().min(SNIFF_LEN)],
    );

    let error = r.error.clone().filter(|e| !e.is_empty());
    let state = if responded && error.is_none() {
        FlowState::Completed
    } else {
        FlowState::Failed
    };
    let t = &entry.timings;
    let known = |v: f64| (v >= 0.0).then_some(v);
    let ssl = known(t.ssl);
    let timing = Timing {
        dns_ms: known(t.dns),
        connect_ms: known(t.connect).map(|c| (c - ssl.unwrap_or(0.0)).max(0.0)),
        tls_ms: ssl,
        ttfb_ms: responded.then_some(t.wait.max(0.0)),
        transfer_ms: responded.then_some(t.receive.max(0.0)),
        reused_connection: known(t.connect).is_none() && known(t.dns).is_none(),
    };
    let req_size = match entry.request.body_size {
        n if n >= 0 => n as u64,
        _ => request_body.len() as u64,
    };
    let resp_size = match r.body_size {
        n if n >= 0 => n as u64,
        _ => response_body.len() as u64,
    };
    let ws = websocket.then(|| ws_messages(messages));
    let summary = FlowSummary {
        id: 0,
        state,
        scheme,
        method: entry.request.method.to_ascii_uppercase().into(),
        host: url.host.as_str().into(),
        port: url.port,
        path: url.path.clone(),
        query: url.query.clone(),
        status: responded.then(|| u16::try_from(r.status).unwrap_or(0)),
        content_kind: content::flow_kind(request_kind, response_detected.kind),
        client: None,
        started_at_ms: parse_time(&entry.started_date_time).unwrap_or(0.0),
        duration_ms: (entry.time >= 0.0).then(|| entry.time.round() as u32),
        req_size,
        resp_size: match &ws {
            Some(ws) => ws
                .messages
                .iter()
                .filter(|(m, _)| m.direction == WsDirection::Received)
                .map(|(m, _)| m.len)
                .sum(),
            None => resp_size,
        },
        edited: false,
        http_version: version,
        tls: if url.scheme.is_tls() {
            TlsStatus::Intercepted
        } else {
            TlsStatus::None
        },
        graphql_op: graphql.and_then(|g| g.operation).map(Arc::from),
        sent_by: None,
        pinned: false,
        grpc_status: None,
        label: None,
        // A note someone wrote; not the one Proxymate adds to what it sent.
        comment: entry
            .comment
            .as_deref()
            .filter(|c| {
                !c.is_empty()
                    && !matches!(
                        *c,
                        "Sent from Proxymate" | "Sent by an agent through Proxymate" | "Repeated by Proxymate"
                    )
            })
            .map(Arc::from),
        extra: Vec::new(),
    };
    let detail = FlowDetail {
        id: 0,
        request: RequestHead {
            method: entry.request.method.to_ascii_uppercase(),
            url: entry.request.url.clone(),
            version,
            headers: request_headers,
        },
        response: responded.then(|| ResponseHead {
            status: u16::try_from(r.status).unwrap_or(0),
            reason: r.status_text.clone(),
            version: if r.http_version.is_empty() {
                version
            } else {
                parse_version(&r.http_version)
            },
            headers: response_headers,
        }),
        request_body: body_info(req_size, &request_body, request_detected.mime.clone(), request_kind),
        response_body: body_info(
            resp_size,
            &response_body,
            response_detected.mime,
            response_detected.kind,
        ),
        error,
        tls: None,
        timing,
        rules: Default::default(),
        trailers: None,
        via: None,
    };
    Some(SavedFlow {
        summary,
        detail,
        request_body,
        response_body,
        ws,
        sse: None,
    })
}

fn body_info(size: u64, body: &[u8], mime: Option<String>, kind: ContentKind) -> BodyInfo {
    BodyInfo {
        size: size.max(body.len() as u64),
        captured: body.len() as u64,
        truncated: false,
        encoding: None,
        complete: true,
        kind: if body.is_empty() { ContentKind::None } else { kind },
        mime,
    }
}

/// HTTP/2 pseudo-headers (Chrome lists `:authority` and friends) are left
/// out; the URL and status already say what they do.
fn headers(list: &[NameValue]) -> Vec<Header> {
    list.iter()
        .filter(|h| !h.name.starts_with(':'))
        .map(|h| Header::new(h.name.clone(), h.value.clone()))
        .collect()
}

fn post_body(post: Option<&PostData>) -> Vec<u8> {
    let Some(post) = post else {
        return Vec::new();
    };
    match (&post.text, post.encoding.as_deref()) {
        (Some(text), Some(e)) if e.eq_ignore_ascii_case("base64") => base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .unwrap_or_else(|_| text.clone().into_bytes()),
        (Some(text), _) => text.clone().into_bytes(),
        (None, _) => post
            .params
            .iter()
            .map(|p| format!("{}={}", p.name, p.value.as_deref().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("&")
            .into_bytes(),
    }
}

fn ws_messages(messages: &[WebSocketMessage]) -> SavedWs {
    let mut out = Vec::with_capacity(messages.len());
    let mut close = None;
    for (index, m) in messages.iter().enumerate() {
        let opcode = match m.opcode {
            2 => WsOpcode::Binary,
            8 => WsOpcode::Close,
            9 => WsOpcode::Ping,
            10 => WsOpcode::Pong,
            _ => WsOpcode::Text,
        };
        let direction = if m.kind.eq_ignore_ascii_case("send") {
            WsDirection::Sent
        } else {
            WsDirection::Received
        };
        let payload = match opcode {
            WsOpcode::Binary => base64::engine::general_purpose::STANDARD
                .decode(m.data.trim())
                .unwrap_or_else(|_| m.data.clone().into_bytes()),
            _ => m.data.clone().into_bytes(),
        };
        if opcode == WsOpcode::Close && close.is_none() {
            close = Some(pm_model::WsClose {
                code: 1005,
                reason: m.data.clone(),
                by: direction,
            });
        }
        let preview = match opcode {
            WsOpcode::Binary => String::new(),
            _ => m.data.chars().take(PREVIEW_CHARS).collect(),
        };
        out.push((
            WsMessage {
                index: index as u32,
                direction,
                opcode,
                len: payload.len() as u64,
                time_ms: m.time * 1000.0,
                truncated: false,
                close_code: None,
                preview,
                edited: false,
                injected: false,
                dropped: false,
            },
            payload,
        ));
    }
    SavedWs {
        messages: out,
        originals: Vec::new(),
        close,
    }
}

pub(crate) fn parse_version(v: &str) -> HttpVersion {
    let v = v.trim().to_ascii_lowercase();
    if v.contains("1.0") {
        HttpVersion::Http10
    } else if v.starts_with("h2") || v.starts_with("h3") || v.contains("/2") || v.contains("/3") {
        // We have no HTTP/3; h3 is closest to h2 for display.
        HttpVersion::Http2
    } else {
        HttpVersion::Http11
    }
}

/// The parts of a URL a flow summary needs. Lenient: HAR URLs aren't
/// always valid `http::Uri`s (spaces, non-ASCII).
struct Url {
    scheme: Scheme,
    host: String,
    port: u16,
    path: String,
    query: String,
}

impl Url {
    fn parse(url: &str) -> Option<Url> {
        let (scheme, rest) = url.split_once("://")?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "ws" => Scheme::Ws,
            "wss" => Scheme::Wss,
            _ => return None,
        };
        let rest = rest.split('#').next().unwrap_or_default();
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let authority = authority.rsplit('@').next().unwrap_or_default();
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => {
                let (host, after) = v6.split_once(']')?;
                (host.to_owned(), after.strip_prefix(':').and_then(|p| p.parse().ok()))
            }
            None => match authority.rsplit_once(':') {
                Some((host, port)) => (host.to_owned(), Some(port.parse().ok()?)),
                None => (authority.to_owned(), None),
            },
        };
        if host.is_empty() {
            return None;
        }
        let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
        Some(Url {
            port: port.unwrap_or(if scheme.is_tls() { 443 } else { 80 }),
            scheme,
            host: host.to_ascii_lowercase(),
            path: if path.is_empty() { "/".into() } else { path.to_owned() },
            query: query.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::WsClose;

    const CHROME: &str = include_str!("../../tests/fixtures/chrome-devtools.har");

    /// Compares with `tests/golden/har/<name>`; `UPDATE_GOLDEN=1` rewrites
    /// the file instead.
    fn golden(name: &str, actual: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/har")
            .join(name);
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{actual}\n")).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            actual,
            expected.trim_end_matches('\n'),
            "golden file {}",
            path.display()
        );
    }

    fn summary(scheme: Scheme, method: &str, host: &str, path: &str, status: Option<u16>) -> FlowSummary {
        FlowSummary {
            id: 0,
            state: if status.is_some() {
                FlowState::Completed
            } else {
                FlowState::Failed
            },
            scheme,
            method: method.into(),
            host: host.into(),
            port: if scheme.is_tls() { 443 } else { 80 },
            path: path.into(),
            query: String::new(),
            status,
            content_kind: ContentKind::Json,
            client: Some("curl".into()),
            started_at_ms: 1_792_575_667_101.0,
            duration_ms: Some(42),
            req_size: 0,
            resp_size: 0,
            edited: false,
            http_version: HttpVersion::Http11,
            tls: TlsStatus::None,
            graphql_op: None,
            sent_by: None,
            pinned: false,
            grpc_status: None,
            label: None,
            comment: None,
            extra: Vec::new(),
        }
    }

    fn detail(
        method: &str,
        url: &str,
        request: &[(&str, &str)],
        response: Option<(u16, &[(&str, &str)])>,
    ) -> FlowDetail {
        let heads = |list: &[(&str, &str)]| list.iter().map(|(n, v)| Header::new(*n, *v)).collect::<Vec<_>>();
        FlowDetail {
            id: 0,
            request: RequestHead {
                method: method.into(),
                url: url.into(),
                version: HttpVersion::Http11,
                headers: heads(request),
            },
            response: response.map(|(status, headers)| ResponseHead {
                status,
                reason: "OK".into(),
                version: HttpVersion::Http11,
                headers: heads(headers),
            }),
            request_body: BodyInfo::default(),
            response_body: BodyInfo::default(),
            error: None,
            tls: None,
            timing: Timing {
                dns_ms: Some(1.5),
                connect_ms: Some(2.25),
                tls_ms: Some(3.125),
                ttfb_ms: Some(30.0),
                transfer_ms: Some(5.0),
                reused_connection: false,
            },
            rules: Default::default(),
            trailers: None,
            via: None,
        }
    }

    fn flows() -> Vec<SavedFlow> {
        let json = br#"{"users":[{"id":1,"name":"Ada"}]}"#;
        let gzipped = {
            use std::io::Write as _;
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
            enc.write_all(json).unwrap();
            enc.finish().unwrap()
        };
        let mut get = detail(
            "GET",
            "https://api.example.com/v1/users?page=2&q=a%20b",
            &[("accept", "application/json"), ("cookie", "sid=1; theme=dark")],
            Some((
                200,
                &[
                    ("content-type", "application/json"),
                    ("content-encoding", "gzip"),
                    ("set-cookie", "sid=2; Path=/; HttpOnly; Secure"),
                ],
            )),
        );
        get.response_body.encoding = Some("gzip".into());
        get.response_body.size = gzipped.len() as u64;
        let mut post = detail(
            "POST",
            "http://upload.example.com/blob",
            &[("content-type", "application/x-www-form-urlencoded")],
            Some((
                201,
                &[("content-type", "application/octet-stream"), ("location", "/blob/7")],
            )),
        );
        post.request_body.size = 13;
        post.response_body.size = 6;
        post.timing = Timing::default();
        let ws = SavedWs {
            messages: vec![
                (ws_message(0, WsDirection::Sent, WsOpcode::Text, 5), b"hello".to_vec()),
                (
                    ws_message(1, WsDirection::Received, WsOpcode::Binary, 3),
                    vec![0, 1, 255],
                ),
                (ws_message(2, WsDirection::Sent, WsOpcode::Close, 3), b"bye".to_vec()),
            ],
            originals: Vec::new(),
            close: Some(WsClose {
                code: 1000,
                reason: "bye".into(),
                by: WsDirection::Sent,
            }),
        };
        let mut failed = detail("GET", "https://down.example.com/", &[], None);
        failed.error = Some("can't connect: Connection refused".into());
        let mut sent = summary(Scheme::Http, "POST", "upload.example.com", "/blob", Some(201));
        sent.sent_by = Some(SentBy::User);
        vec![
            SavedFlow {
                summary: summary(Scheme::Https, "GET", "api.example.com", "/v1/users", Some(200)),
                detail: get,
                request_body: Vec::new(),
                response_body: gzipped,
                ws: None,
                sse: None,
            },
            SavedFlow {
                summary: sent,
                detail: post,
                request_body: b"name=Ada&x=1%2B1".to_vec(),
                response_body: vec![0, 1, 2, 3, 254, 255],
                ws: None,
                sse: None,
            },
            SavedFlow {
                summary: summary(Scheme::Wss, "GET", "ws.example.com", "/socket", Some(101)),
                detail: detail(
                    "GET",
                    "wss://ws.example.com/socket",
                    &[("upgrade", "websocket")],
                    Some((101, &[])),
                ),
                request_body: Vec::new(),
                response_body: Vec::new(),
                ws: Some(ws),
                sse: None,
            },
            SavedFlow {
                summary: summary(Scheme::Https, "GET", "down.example.com", "/", None),
                detail: failed,
                request_body: Vec::new(),
                response_body: Vec::new(),
                ws: None,
                sse: None,
            },
        ]
    }

    fn ws_message(index: u32, direction: WsDirection, opcode: WsOpcode, len: u64) -> WsMessage {
        WsMessage {
            index,
            direction,
            opcode,
            len,
            time_ms: 1_792_575_667_500.0 + f64::from(index) * 250.0,
            truncated: false,
            close_code: None,
            preview: String::new(),
            edited: false,
            injected: false,
            dropped: false,
        }
    }

    #[test]
    fn export_matches_the_golden_file() {
        let mut har = export(flows());
        // The creator version tracks the crate version; pin it so a release bump doesn't break the golden file.
        assert_eq!(har.log.creator.version, env!("CARGO_PKG_VERSION"));
        har.log.creator.version = "0.0.0-test".into();
        let json = serde_json::to_string_pretty(&har).unwrap();
        golden("export.har", &json);
    }

    #[test]
    fn written_files_are_the_same_document() {
        let dir = std::env::temp_dir().join(format!("pm-har-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.har");
        let mut seen = Vec::new();
        let n = write(&path, flows().into_iter().map(Ok), &mut |n| seen.push(n)).unwrap();
        assert_eq!((n, seen.last().copied()), (4, Some(4)));
        let written: Har = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written, export(flows()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn times_round_trip() {
        let ms = 1_792_575_667_101.0;
        assert_eq!(format_time(ms), "2026-10-21T09:41:07.101Z");
        assert_eq!(parse_time("2026-10-21T09:41:07.101Z"), Some(ms));
        assert_eq!(parse_time("2026-10-21T11:41:07.101+02:00"), Some(ms));
        assert_eq!(parse_time("2026-10-21T04:41:07.101-05:00"), Some(ms));
        assert_eq!(format_time(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(parse_time("garbage"), None);
    }

    #[test]
    fn chrome_entries_import() {
        let flows = import(CHROME.as_bytes()).unwrap();
        assert_eq!(flows.len(), 6);

        let doc = &flows[0];
        assert_eq!(doc.summary.http_version, HttpVersion::Http2);
        assert_eq!(doc.summary.content_kind, ContentKind::Html);
        assert_eq!(doc.summary.tls, TlsStatus::Intercepted);
        assert_eq!(doc.summary.started_at_ms, 1_792_575_667_101.0);
        assert!(doc.detail.request.headers.iter().all(|h| !h.name.starts_with(':')));
        assert_eq!(doc.detail.request.headers.len(), 4);
        assert!(doc.response_body.starts_with(b"<!doctype html>"));
        assert_eq!(doc.detail.response_body.encoding, None);
        let t = &doc.detail.timing;
        assert_eq!(
            (t.dns_ms, t.connect_ms, t.tls_ms, t.ttfb_ms),
            (Some(4.5), Some(10.125), Some(20.125), Some(45.678))
        );

        assert_eq!(flows[1].summary.graphql_op.as_deref(), Some("GetUser"));
        assert_eq!(flows[1].summary.content_kind, ContentKind::GraphQl);
        assert_eq!(flows[1].summary.query, "trace=1");

        assert!(flows[2].response_body.starts_with(b"\x89PNG"));
        assert_eq!(flows[2].detail.response_body.mime.as_deref(), Some("image/png"));

        let ws = &flows[3];
        assert_eq!((ws.summary.scheme, ws.summary.status), (Scheme::Wss, Some(101)));
        let messages = &ws.ws.as_ref().unwrap().messages;
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].0.direction, WsDirection::Sent);
        assert_eq!(messages[2].0.opcode, WsOpcode::Binary);
        assert_eq!(messages[2].1, b"\x00\x01\x02binary");

        assert_eq!(flows[4].request_body, b"name=Ada+L&city=London");
        assert_eq!(flows[4].summary.content_kind, ContentKind::FormUrlEncoded);

        let failed = &flows[5];
        assert_eq!((failed.summary.state, failed.summary.status), (FlowState::Failed, None));
        assert_eq!(failed.detail.error.as_deref(), Some("net::ERR_NAME_NOT_RESOLVED"));
    }

    /// Chrome's HAR, imported and exported again, keeps what HAR can say
    /// about each request; importing that export gives the same flows.
    #[test]
    fn chrome_har_round_trips() {
        let original: Har = serde_json::from_str(CHROME).unwrap();
        let imported = import(CHROME.as_bytes()).unwrap();
        let exported = export(imported);
        assert_eq!(exported.log.entries.len(), original.log.entries.len());
        let no_pseudo = |list: &[NameValue]| {
            list.iter()
                .filter(|h| !h.name.starts_with(':'))
                .cloned()
                .collect::<Vec<_>>()
        };
        for (a, b) in original.log.entries.iter().zip(&exported.log.entries) {
            let url = &a.request.url;
            assert_eq!(a.started_date_time, b.started_date_time, "{url}");
            assert!((a.time - b.time).abs() < 1.0, "{url}: time {} vs {}", a.time, b.time);
            assert_eq!(a.request.method, b.request.method, "{url}");
            assert_eq!(a.request.url, b.request.url);
            assert_eq!(
                parse_version(&a.request.http_version),
                parse_version(&b.request.http_version),
                "{url}"
            );
            assert_eq!(no_pseudo(&a.request.headers), b.request.headers, "{url}");
            assert_eq!(a.request.query_string, b.request.query_string, "{url}");
            assert_eq!(
                a.request.post_data.as_ref().and_then(|p| p.text.clone()),
                b.request.post_data.as_ref().and_then(|p| p.text.clone()),
                "{url}"
            );
            assert_eq!(a.response.status, b.response.status, "{url}");
            assert_eq!(a.response.status_text, b.response.status_text, "{url}");
            assert_eq!(a.response.headers, b.response.headers, "{url}");
            assert_eq!(a.response.redirect_url, b.response.redirect_url, "{url}");
            assert_eq!(a.response.content.text, b.response.content.text, "{url}");
            assert_eq!(a.response.content.encoding, b.response.content.encoding, "{url}");
            assert_eq!(a.response.error, b.response.error, "{url}");
            for (x, y) in [
                (a.timings.dns, b.timings.dns),
                (a.timings.connect, b.timings.connect),
                (a.timings.ssl, b.timings.ssl),
                (a.timings.wait, b.timings.wait),
                (a.timings.receive, b.timings.receive),
            ] {
                if a.response.status > 0 {
                    assert!(
                        (x - y).abs() < 1e-9,
                        "{url}: timings {:?} vs {:?}",
                        a.timings,
                        b.timings
                    );
                }
            }
            let messages = |e: &Entry| e.web_socket_messages.clone().unwrap_or_default();
            assert_eq!(messages(a).len(), messages(b).len(), "{url}");
            for (x, y) in messages(a).iter().zip(&messages(b)) {
                assert_eq!((&x.kind, x.opcode, &x.data), (&y.kind, y.opcode, &y.data), "{url}");
                assert!((x.time - y.time).abs() < 1e-6, "{url}");
            }
        }

        let again = import(serde_json::to_string(&exported).unwrap().as_bytes()).unwrap();
        for (a, b) in import(CHROME.as_bytes()).unwrap().iter().zip(&again) {
            assert_eq!(a.summary, b.summary);
            assert_eq!(a.detail, b.detail);
            assert_eq!((&a.request_body, &a.response_body), (&b.request_body, &b.response_body));
            let ws = |f: &SavedFlow| f.ws.as_ref().map(|w| w.messages.clone());
            assert_eq!(ws(a), ws(b));
        }
    }

    #[test]
    fn junk_is_refused() {
        assert!(matches!(import(b"<html>"), Err(ImportError::Json(_))));
        assert!(matches!(
            import(br#"{"log":{"version":"1.2","entries":[]}}"#),
            Err(ImportError::Empty)
        ));
    }
}
