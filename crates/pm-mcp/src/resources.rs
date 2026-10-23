//! MCP resources: the status, the rule and script lists, the script API's
//! type definitions, and the breakpoint queue
//! (subscribable: each hit sends `resources/updated`), plus templates for a
//! flow, its bodies and its WebSocket messages.

use base64::Engine as _;
use pm_core::Engine;
use pm_model::{BodyPart, FlowId, WsQuery};
use rmcp::ErrorData;
use rmcp::model::{ReadResourceResult, Resource, ResourceContents, ResourceTemplate};
use serde_json::{Value, json};

pub(crate) const STATUS_URI: &str = "proxymate://status";
pub(crate) const RULES_URI: &str = "proxymate://rules";
pub(crate) const BREAKPOINTS_URI: &str = "proxymate://breakpoints";
pub(crate) const SCRIPTS_URI: &str = "proxymate://scripts";
pub(crate) const SCRIPTS_API_URI: &str = "proxymate://scripts/api";
const FLOW_PREFIX: &str = "proxymate://flows/";
/// Bodies read as resources are cut here; `get_flow_body` pages further.
const BODY_MAX: usize = 1024 * 1024;
/// WebSocket messages listed per read.
const WS_MAX: u32 = 500;

const JSON: &str = "application/json";

pub(crate) fn list() -> Vec<Resource> {
    vec![
        Resource::new(STATUS_URI, "status")
            .with_title("Proxymate status")
            .with_description("Where the proxy listens, recording, counts of flows, rules and paused breakpoints.")
            .with_mime_type(JSON),
        Resource::new(RULES_URI, "rules")
            .with_title("Rules")
            .with_description("Every rule in evaluation order. Updated when a rule changes.")
            .with_mime_type(JSON),
        Resource::new(SCRIPTS_URI, "scripts")
            .with_title("Scripts")
            .with_description("Every script, with its source. Updated when a script changes.")
            .with_mime_type(JSON),
        Resource::new(SCRIPTS_API_URI, "scripts-api")
            .with_title("Script API")
            .with_description("proxymate.d.ts: the TypeScript definitions of the JavaScript API scripts use.")
            .with_mime_type("application/typescript"),
        Resource::new(BREAKPOINTS_URI, "breakpoints")
            .with_title("Paused flows")
            .with_description(
                "Flows paused at a breakpoint. Subscribe to be told on every hit; resolve with resolve_breakpoint.",
            )
            .with_mime_type(JSON),
    ]
}

pub(crate) fn templates() -> Vec<ResourceTemplate> {
    let template = |uri: &str, name: &str, title: &str, description: &str, mime: Option<&str>| {
        let mut t = ResourceTemplate::new(uri, name)
            .with_title(title)
            .with_description(description);
        if let Some(mime) = mime {
            t = t.with_mime_type(mime);
        }
        t
    };
    vec![
        template(
            "proxymate://flows/{id}",
            "flow",
            "Flow",
            "A flow's summary, heads, body metadata, timing and applied rules.",
            Some(JSON),
        ),
        template(
            "proxymate://flows/{id}/request/body",
            "request-body",
            "Request body",
            "The decoded request body (first 1 MB), with its Content-Type.",
            None,
        ),
        template(
            "proxymate://flows/{id}/response/body",
            "response-body",
            "Response body",
            "The decoded response body (first 1 MB), with its Content-Type.",
            None,
        ),
        template(
            "proxymate://flows/{id}/ws",
            "websocket-messages",
            "WebSocket messages",
            "The first 500 messages of a WebSocket flow (use get_websocket_messages for more).",
            Some(JSON),
        ),
    ]
}

fn not_found(uri: &str, why: impl std::fmt::Display) -> ErrorData {
    ErrorData::resource_not_found(format!("{uri}: {why}"), None)
}

fn json_contents(uri: &str, value: Value) -> ReadResourceResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    ReadResourceResult::new(vec![ResourceContents::text(text, uri).with_mime_type(JSON)])
}

pub(crate) fn read(engine: &Engine, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    match uri {
        STATUS_URI => {
            let status = engine.proxy_status();
            return Ok(json_contents(
                uri,
                json!({
                    "proxy": status,
                    "flows": engine.total(),
                    "rules": engine.rules().len(),
                    "paused_breakpoints": engine.breakpoints().len(),
                    "ca": engine.ca_info(),
                    "mcp": engine.mcp_settings(),
                }),
            ));
        }
        RULES_URI => return Ok(json_contents(uri, json!(engine.rules()))),
        SCRIPTS_URI => return Ok(json_contents(uri, json!(engine.scripts()))),
        SCRIPTS_API_URI => {
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(engine.script_type_definitions(), uri).with_mime_type("application/typescript"),
            ]));
        }
        BREAKPOINTS_URI => return Ok(json_contents(uri, json!(engine.breakpoints()))),
        _ => {}
    }
    let rest = uri
        .strip_prefix(FLOW_PREFIX)
        .ok_or_else(|| not_found(uri, "unknown resource"))?;
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let id: FlowId = id.parse().map_err(|_| not_found(uri, "not a flow id"))?;
    let summary = engine.get_flow(id).ok_or_else(|| not_found(uri, "no such flow"))?;
    match tail {
        "" => {
            let detail = engine
                .flow_detail(id)
                .map_err(|e| not_found(uri, e))?
                .ok_or_else(|| not_found(uri, "no detail yet"))?;
            Ok(json_contents(uri, json!({ "summary": summary, "detail": detail })))
        }
        "request/body" | "response/body" => {
            let part = if tail.starts_with("request") {
                BodyPart::Request
            } else {
                BodyPart::Response
            };
            let detail = engine.flow_detail(id).ok().flatten();
            let mime = detail
                .as_ref()
                .and_then(|d| match part {
                    BodyPart::Request => d.request_body.mime.clone(),
                    BodyPart::Response => d.response_body.mime.clone(),
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            let body = engine
                .body_head(id, part, true, Some(BODY_MAX))
                .or_else(|_| engine.body_head(id, part, false, Some(BODY_MAX)))
                .map_err(|e| not_found(uri, e))?;
            let contents = match String::from_utf8(body.bytes) {
                Ok(text) => ResourceContents::text(text, uri),
                Err(bytes) => {
                    ResourceContents::blob(base64::engine::general_purpose::STANDARD.encode(bytes.as_bytes()), uri)
                }
            };
            Ok(ReadResourceResult::new(vec![contents.with_mime_type(mime)]))
        }
        "ws" => {
            let page = engine
                .websocket_messages(id, &WsQuery::default(), 0, WS_MAX)
                .map_err(|e| not_found(uri, e))?;
            Ok(json_contents(uri, json!(page)))
        }
        _ => Err(not_found(uri, "unknown resource")),
    }
}
