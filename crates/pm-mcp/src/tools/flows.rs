//! Flow tools: status, listing, search, details, bodies, waiting, export.

use super::*;

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Order {
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ListFlows {
    /// Filter DSL, e.g. `host:api.example.com status:5xx`. Empty lists everything.
    #[serde(default)]
    pub(super) filter: String,
    /// Only flows with a larger id (newer than this one).
    #[serde(default)]
    pub(super) since_id: Option<FlowId>,
    /// 1–500, default 50.
    #[serde(default)]
    pub(super) limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[serde(default)]
    pub(super) cursor: Option<String>,
    #[serde(default)]
    pub(super) sort: Order,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SearchFlows {
    /// Text to find (case-insensitive), or a regex with `regex: true`.
    pub(super) query: String,
    #[serde(default)]
    pub(super) regex: bool,
    /// Where to look. Default: everywhere.
    #[serde(default)]
    pub(super) scopes: Vec<SearchScope>,
    /// Only search flows matching this filter DSL.
    #[serde(default)]
    pub(super) filter: String,
    /// 1–100, default 20.
    #[serde(default)]
    pub(super) limit: Option<u32>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Include {
    /// Request and response headers.
    Headers,
    Timing,
    Tls,
    /// Rules that changed the flow.
    Rules,
    /// The first 2 KB of each body as text, when it is text.
    BodyPreview,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetFlow {
    pub(super) id: FlowId,
    /// Default: headers, timing, rules.
    #[serde(default)]
    pub(super) include: Option<Vec<Include>>,
}

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Part {
    Request,
    #[default]
    Response,
}

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum BodyFormat {
    /// Pretty JSON for JSON bodies, text for other text, base64 for binary.
    #[default]
    Auto,
    /// As UTF-8 text (invalid bytes replaced).
    Text,
    /// Parsed and pretty-printed; an error if it isn't JSON.
    Json,
    Base64,
    /// gRPC, protobuf, MessagePack or CBOR decoded to JSON messages.
    Decoded,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct GetFlowBody {
    pub(super) id: FlowId,
    #[serde(default)]
    pub(super) part: Part,
    /// Undo Content-Encoding (default true).
    #[serde(default = "yes")]
    pub(super) decode: bool,
    #[serde(default)]
    pub(super) format: BodyFormat,
    /// RFC 6901 pointer into a JSON body, e.g. `/data/items/0`.
    #[serde(default)]
    pub(super) json_pointer: Option<String>,
    /// Byte offset into the (formatted) body.
    #[serde(default)]
    pub(super) offset: u64,
    /// Default 65536, at most 1048576.
    #[serde(default)]
    pub(super) max_bytes: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct WaitForFlow {
    /// Filter DSL the flow must match, e.g. `host:api.example.com path:/login`.
    #[serde(default)]
    pub(super) filter: String,
    /// Default 30000, at most 120000.
    #[serde(default)]
    pub(super) timeout_ms: Option<u64>,
    /// Only flows with a larger id. Default: flows that start after this call.
    #[serde(default)]
    pub(super) after_id: Option<FlowId>,
    /// Return as soon as a matching flow starts, not when it finishes.
    #[serde(default)]
    pub(super) include_active: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct SetRecording {
    pub(super) recording: bool,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct FlowIdParam {
    pub(super) id: FlowId,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ExportHar {
    /// These flows, in this order. Without, the flows matching `filter`.
    #[serde(default)]
    pub(super) ids: Option<Vec<FlowId>>,
    /// Filter DSL; empty exports everything (up to `limit`).
    #[serde(default)]
    pub(super) filter: String,
    /// 1–500, default 50.
    #[serde(default)]
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub(super) struct ExportCode {
    pub(super) id: FlowId,
    pub(super) language: CodeLanguage,
}

pub(super) fn url_of(f: &FlowSummary) -> String {
    let port = if f.is_default_port() {
        String::new()
    } else {
        format!(":{}", f.port)
    };
    let query = if f.query.is_empty() {
        String::new()
    } else {
        format!("?{}", f.query)
    };
    format!("{}://{}{port}{}{query}", f.scheme.as_str(), f.host, f.path)
}

/// A flow summary as agents see it.
pub(super) fn brief(f: &FlowSummary) -> Value {
    json!({
        "id": f.id,
        "method": &*f.method,
        "url": url_of(f),
        "status": f.status,
        "state": f.state,
        "content_kind": f.content_kind,
        "client": f.client.as_deref(),
        "started_at_ms": f.started_at_ms,
        "duration_ms": f.duration_ms,
        "request_size": f.req_size,
        "response_size": f.resp_size,
        "http_version": f.http_version,
        "tls": f.tls,
        "edited": f.edited,
        "graphql_operation": f.graphql_op.as_deref(),
        "grpc_status": f.grpc_status,
        "pinned": f.pinned,
        "label": f.label,
        "comment": f.comment.as_deref(),
    })
}

pub(super) fn get_status(ctx: &Ctx<'_>) -> ToolResult {
    let engine = ctx.engine;
    let rules = engine.rules();
    let ca = engine.ca_info();
    Ok(json!({
        "proxy": engine.proxy_status(),
        "recording": engine.is_recording(),
        "flows": engine.total(),
        "rules": rules.iter().filter(|r| !r.id.starts_with("builtin-")).count(),
        "enabled_rules": rules.iter().filter(|r| r.enabled).count(),
        "agent_rules": rules.iter().filter(|r| matches!(r.origin, RuleOrigin::Agent { .. })).count(),
        "paused_breakpoints": engine.breakpoints().len(),
        "ca": { "common_name": ca.common_name, "cert_path": ca.cert_path },
        "mcp": { "read_only": engine.mcp_settings().read_only, "client": ctx.client, "session": ctx.session },
    }))
}

pub(super) fn list_flows(ctx: &Ctx<'_>, p: ListFlows) -> ToolResult {
    let page: FlowPage = ctx.engine.find_flows(
        &p.filter,
        p.since_id,
        cursor_id(&p.cursor)?,
        limit(p.limit, 50, 500),
        p.sort == Order::NewestFirst,
    )?;
    Ok(json!({
        "flows": page.flows.iter().map(brief).collect::<Vec<_>>(),
        "matched": page.matched,
        "next_cursor": page.next.map(|id| id.to_string()),
    }))
}

pub(super) async fn search_flows(ctx: &Ctx<'_>, p: SearchFlows) -> ToolResult {
    let scopes = if p.scopes.is_empty() {
        vec![
            SearchScope::Url,
            SearchScope::RequestHeaders,
            SearchScope::ResponseHeaders,
            SearchScope::RequestBody,
            SearchScope::ResponseBody,
        ]
    } else {
        p.scopes
    };
    let cursor = cursor_id(&p.cursor)?;
    let max = limit(p.limit, 20, 100);
    // Reading bodies is blocking work.
    let page = blocking(ctx, move |e| {
        e.search_flows(&p.query, p.regex, &scopes, &p.filter, cursor, max)
    })
    .await?;
    Ok(json!({
        "hits": page.hits.iter().map(|h| json!({
            "flow": brief(&h.flow),
            "matches": h.matches.iter().map(|m| json!({ "scope": m.scope, "snippet": m.snippet })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "scanned": page.scanned,
        "next_cursor": page.next.map(|id| id.to_string()),
    }))
}

pub(super) fn detail_of(ctx: &Ctx<'_>, id: FlowId) -> Result<(FlowSummary, FlowDetail), ToolError> {
    let summary = ctx
        .engine
        .get_flow(id)
        .ok_or_else(|| ToolError(format!("no flow {id}")))?;
    let detail = ctx
        .engine
        .flow_detail(id)?
        .ok_or_else(|| ToolError(format!("flow {id} has no detail yet")))?;
    Ok((summary, detail))
}

pub(super) fn get_flow(ctx: &Ctx<'_>, p: GetFlow) -> ToolResult {
    let (summary, detail) = detail_of(ctx, p.id)?;
    let include = p
        .include
        .unwrap_or_else(|| vec![Include::Headers, Include::Timing, Include::Rules]);
    let has = |i| include.contains(&i);
    let head = |headers: &[Header]| -> Value {
        if has(Include::Headers) {
            json!(headers)
        } else {
            json!(headers.len())
        }
    };
    let mut out = json!({
        "flow": brief(&summary),
        "request": {
            "method": detail.request.method,
            "url": detail.request.url,
            "version": detail.request.version,
            "headers": head(&detail.request.headers),
            "body": detail.request_body,
        },
        "response": detail.response.as_ref().map(|r| json!({
            "status": r.status,
            "reason": r.reason,
            "version": r.version,
            "headers": head(&r.headers),
            "body": detail.response_body,
            "trailers": detail.trailers,
        })),
        "error": detail.error,
    });
    if let Ok(Some(op)) = ctx.engine.flow_operation_in(LIVE_STORE, p.id) {
        out["operation"] = json!(op);
    }
    if has(Include::Timing) {
        out["timing"] = json!(detail.timing);
    }
    if has(Include::Tls) {
        out["tls"] = json!(detail.tls);
    }
    if has(Include::Rules) {
        out["rules"] = json!(detail.rules);
    }
    if has(Include::BodyPreview) {
        for (key, part) in [
            ("request_preview", BodyPart::Request),
            ("response_preview", BodyPart::Response),
        ] {
            let preview = ctx
                .engine
                .body_head(p.id, part, true, Some(2048))
                .ok()
                .filter(|b| !b.bytes.is_empty())
                .and_then(|b| String::from_utf8(b.bytes).ok());
            out[key] = json!(preview);
        }
    }
    if summary.scheme.is_websocket() {
        out["websocket"] = json!("Use get_websocket_messages for the messages.");
    }
    Ok(out)
}

pub(super) async fn get_flow_body(ctx: &Ctx<'_>, p: GetFlowBody) -> ToolResult {
    if p.format == BodyFormat::Decoded {
        return decoded_body(ctx, p).await;
    }
    let (_, detail) = detail_of(ctx, p.id)?;
    let part: BodyPart = p.part.into();
    let info = match part {
        BodyPart::Request => &detail.request_body,
        BodyPart::Response => &detail.response_body,
    };
    let max = p.max_bytes.map_or(BODY_DEFAULT, |m| (m as usize).min(BODY_MAX));
    let needs_parse = p.json_pointer.is_some() || p.format == BodyFormat::Json;
    let auto_json = p.format == BodyFormat::Auto && info.kind == pm_model::ContentKind::Json;
    // Formatting needs the whole body; plain reads only what's asked for.
    let read_max = if needs_parse || auto_json {
        JSON_PARSE_MAX
    } else {
        (p.offset as usize).saturating_add(max)
    };
    let id = p.id;
    let body = blocking(ctx, move |e| e.body_head(id, part, p.decode, Some(read_max))).await?;

    let mut format = p.format;
    let mut bytes = body.bytes;
    let mut note = None;
    if needs_parse || (auto_json && !body.cut) {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => {
                let value = match &p.json_pointer {
                    Some(pointer) => value
                        .pointer(pointer)
                        .cloned()
                        .ok_or_else(|| ToolError(format!("nothing at JSON pointer {pointer:?}")))?,
                    None => value,
                };
                bytes = serde_json::to_vec_pretty(&value)?;
                format = BodyFormat::Text;
            }
            Err(err) if needs_parse => {
                let why = if body.cut {
                    format!("the body is over {} MB, too big to parse", JSON_PARSE_MAX >> 20)
                } else {
                    err.to_string()
                };
                return Err(ToolError(format!("the body isn't JSON: {why}")));
            }
            Err(_) => note = Some("labelled JSON but doesn't parse; returned as is"),
        }
    }
    let total = bytes.len();
    let start = (p.offset as usize).min(total);
    let end = (start + max).min(total);
    let slice = &bytes[start..end];
    let (text, encoding) = match format {
        BodyFormat::Base64 => (base64::engine::general_purpose::STANDARD.encode(slice), "base64"),
        BodyFormat::Text | BodyFormat::Json | BodyFormat::Decoded => {
            (String::from_utf8_lossy(slice).into_owned(), "utf8")
        }
        BodyFormat::Auto => match std::str::from_utf8(slice) {
            Ok(s) => (s.to_owned(), "utf8"),
            // A cut can split a character; that's still text.
            Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > slice.len() => {
                (String::from_utf8_lossy(slice).into_owned(), "utf8")
            }
            Err(_) => (base64::engine::general_purpose::STANDARD.encode(slice), "base64"),
        },
    };
    Ok(json!({
        "id": p.id,
        "part": part,
        "content_kind": info.kind,
        "mime": info.mime,
        "content_encoding": info.encoding,
        "decoded_from": body.decoded_from,
        "size_on_wire": info.size,
        "captured_truncated": info.truncated,
        "complete": info.complete,
        "offset": start,
        "returned_bytes": slice.len(),
        "total_bytes": if body.cut && !needs_parse { None } else { Some(total) },
        "more": end < total || (body.cut && !needs_parse),
        "next_offset": (end < total || body.cut).then_some(end),
        "encoding": encoding,
        "note": note,
        "body": text,
    }))
}

/// `format: decoded`: the same decoding as the inspector's viewers
/// (`Engine::decode_body_in`). Pages by message: `offset` is the first
/// message, and messages are added until `max_bytes` of JSON.
pub(super) async fn decoded_body(ctx: &Ctx<'_>, p: GetFlowBody) -> ToolResult {
    let part: BodyPart = p.part.into();
    let id = p.id;
    let decoded = blocking(ctx, move |e| e.decode_body_in(LIVE_STORE, id, part)).await?;
    let max = p.max_bytes.map_or(BODY_DEFAULT, |m| (m as usize).min(BODY_MAX));
    let start = (p.offset as usize).min(decoded.messages.len());
    let mut used = 0;
    let mut messages = Vec::new();
    for m in &decoded.messages[start..] {
        if used > 0 && used + m.json.len() > max {
            break;
        }
        used += m.json.len();
        let value: Value = serde_json::from_str(&m.json).unwrap_or(Value::Null);
        let value = match &p.json_pointer {
            Some(pointer) => value.pointer(pointer).cloned().unwrap_or(Value::Null),
            None => value,
        };
        let mut out = json!({
            "index": m.index,
            "compressed": m.compressed,
            "size": m.size,
            "schema_less": m.schema_less,
            "value": value,
        });
        if !m.badges.is_empty() {
            let badges: serde_json::Map<String, Value> = m
                .badges
                .iter()
                .map(|b| {
                    (
                        if b.pointer.is_empty() {
                            "/".into()
                        } else {
                            b.pointer.clone()
                        },
                        json!(b.label),
                    )
                })
                .collect();
            out["badges"] = Value::Object(badges);
        }
        if let Some(error) = &m.error {
            out["error"] = json!(error);
        }
        messages.push(out);
    }
    let end = start + messages.len();
    Ok(json!({
        "id": p.id,
        "part": part,
        "format": decoded.format,
        "message_type": decoded.message_type,
        "status": decoded.status,
        "trailers": decoded.trailers,
        "total_messages": decoded.messages.len(),
        "offset": start,
        "messages": messages,
        "more": end < decoded.messages.len(),
        "next_offset": (end < decoded.messages.len()).then_some(end),
        "pending_bytes": decoded.pending_bytes,
        "error": decoded.error,
    }))
}

pub(super) async fn wait_for_flow(ctx: &Ctx<'_>, p: WaitForFlow) -> ToolResult {
    let query = ctx
        .engine
        .parse_query(&p.filter)
        .map_err(|e| ToolError(e.to_string()))?;
    let after = match p.after_id {
        Some(after) => Some(after),
        // Flows that start after this call: everything up to the newest now.
        None => blocking(ctx, |e| Ok(e.find_flows("", None, None, 1, true).ok()))
            .await?
            .and_then(|newest| newest.flows.first().map(|f| f.id)),
    };
    let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(WAIT_DEFAULT_MS).min(WAIT_MAX_MS));
    let deadline = tokio::time::Instant::now() + timeout;
    let query = Arc::new(query);
    let include_active = p.include_active;
    loop {
        // A scan of the whole store (deep terms read bodies): off the async threads.
        let q = query.clone();
        let found = blocking(ctx, move |e| Ok(e.first_flow_matching(&q, after, include_active))).await?;
        if let Some(flow) = found {
            return Ok(json!({ "flow": brief(&flow), "timed_out": false }));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(json!({ "flow": null, "timed_out": true, "after_id": after }));
        }
        tokio::select! {
            _ = ctx.ct.cancelled() => return Err(ToolError("cancelled".into())),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

pub(super) fn export_har(ctx: &Ctx<'_>, p: ExportHar) -> ToolResult {
    let ids = match p.ids {
        Some(ids) => ids,
        None => ctx
            .engine
            .find_flows(&p.filter, None, None, limit(p.limit, 50, 500), false)?
            .flows
            .iter()
            .map(|f| f.id)
            .collect(),
    };
    if ids.is_empty() {
        return Err(ToolError("no flows to export".into()));
    }
    let har = ctx
        .engine
        .har_json(LIVE_STORE, &FlowSelection::Ids { ids: ids.clone() })?;
    if har.len() > HAR_MAX {
        return Err(ToolError(format!(
            "the HAR is {} KB, over the {} KB limit: export fewer flows (ids or a tighter filter)",
            har.len() / 1024,
            HAR_MAX / 1024
        )));
    }
    let har: Value = serde_json::from_str(&har)?;
    Ok(json!({ "flows": ids.len(), "har": har }))
}

impl From<Part> for BodyPart {
    fn from(p: Part) -> BodyPart {
        match p {
            Part::Request => BodyPart::Request,
            Part::Response => BodyPart::Response,
        }
    }
}
