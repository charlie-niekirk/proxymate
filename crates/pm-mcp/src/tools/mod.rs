//! The MCP tool surface. Each tool's input schema comes from its parameter
//! struct's `schemars` derive, which embeds the `pm-model` types (`Matcher`,
//! `Action`, `BreakpointResolution`), so agents send exactly what the
//! engine takes. Every tool calls only the `Engine` facade.
//!
//! Lists page with an opaque `cursor`; bodies and payloads are capped.

mod flows;
mod rules;
mod scripts;
mod send;
mod websocket;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use pm_core::{Engine, FlowPage};
use pm_model::{
    Action, BodyPart, BreakpointId, BreakpointResolution, FlowDetail, FlowId, FlowSummary, Header, MapLocalSource,
    Matcher, Pattern, Phase, PresetKind, Rule, RuleOrigin, RulePatch, Scheme, SearchScope, WsDirection, WsQuery,
};
use pm_model::{CodeLanguage, FlowSelection, LIVE_STORE, RepeatOpts, RequestSpec, SendOpts, SendOrigin, StoreId};
use pm_model::{Script, ScriptKind, ScriptTestSpec, ServeKey, ServeMiss, ServePick, ViewConfig};
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use flows::*;
use rules::*;
use scripts::*;
use send::*;
use websocket::*;

/// Bodies and payloads are returned up to this by default…
const BODY_DEFAULT: usize = 64 * 1024;
/// …and never more than this per call.
const BODY_MAX: usize = 1024 * 1024;
/// JSON is parsed (for `format: json` and `json_pointer`) up to this size.
const JSON_PARSE_MAX: usize = 32 * 1024 * 1024;
/// HAR documents returned inline stop here; bigger exports should go to a file in the app.
const HAR_MAX: usize = 4 * 1024 * 1024;
const WAIT_DEFAULT_MS: u64 = 30_000;
const WAIT_MAX_MS: u64 = 120_000;

/// Everything a tool call needs.
pub(crate) struct Ctx<'a> {
    pub engine: &'a Arc<Engine>,
    /// `clientInfo.name` from `initialize`.
    pub client: &'a str,
    pub session: &'a str,
    /// Cancelled when the client cancels the request.
    pub ct: &'a CancellationToken,
}

/// An error the agent reads.
#[derive(Debug)]
pub(crate) struct ToolError(pub String);

impl<E: std::fmt::Display> From<E> for ToolError {
    fn from(err: E) -> Self {
        ToolError(err.to_string())
    }
}

type ToolResult = Result<Value, ToolError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Read,
    /// Adds something; undoable.
    Write,
    /// Removes or overrides something.
    Destructive,
}

impl Kind {
    pub fn mutates(self) -> bool {
        self != Kind::Read
    }
}

struct Def {
    name: &'static str,
    kind: Kind,
    description: &'static str,
    schema: fn() -> Arc<JsonObject>,
}

macro_rules! def {
    ($name:literal, $kind:ident, $params:ty, $description:expr) => {
        Def {
            name: $name,
            kind: Kind::$kind,
            description: $description,
            schema: schema_for_type::<$params>,
        }
    };
}

const FILTER_HELP: &str = "Filter DSL: space-separated terms, all must match; `-` negates. host:*.example.com \
path:/api/* method:POST,PUT status:4xx|404|400-499 ct:json|form|xml|js|css|graphql|document|media|html|image \
grpc:0|not_found|!0 (gRPC status; !0 = failed calls) ct:grpc|protobuf|msgpack|cbor \
app:Safari state:active|completed|failed|blocked|paused|tunnel is:edited is:pinned dur>500 dur<2s size>1mb \
reqsize>10kb id>120 op:listPets|op:get*|op:* (an imported OpenAPI operationId). Bare words (or \"quoted phrases\") search the URL. Header and body terms read each flow's \
detail, so they are slower: header:x-request-id (any header line; reqheader:/resheader: for one side), \
body:\"order_id\" (decoded body, first 4 MB; reqbody:/resbody:).";

fn defs() -> Vec<Def> {
    vec![
        def!(
            "get_status",
            Read,
            NoParams,
            "Proxy status: where it listens, whether it records, flow, rule and paused-breakpoint counts, the CA to trust, and whether this MCP server is read-only."
        ),
        def!(
            "list_flows",
            Read,
            ListFlows,
            "List recorded flows (summaries), newest first by default, optionally filtered with the filter DSL. Page with `cursor`; `since_id` returns only flows newer than an id you have seen."
        ),
        def!(
            "search_flows",
            Read,
            SearchFlows,
            "Deep search: find flows whose URL, headers or decoded bodies (first 1 MB) contain a string (or match a regex). Newest first; returns snippets. Page with `cursor`."
        ),
        def!(
            "get_flow",
            Read,
            GetFlow,
            "One flow: summary, request and response heads, body metadata, timing, TLS, the rules that changed it and any error (with a stable `error_kind`: dns, refused, timeout, upstream_cert_untrusted, client_rejected_cert, …)."
        ),
        def!(
            "get_flow_body",
            Read,
            GetFlowBody,
            "A flow's request or response body, decoded (gzip/br/deflate/zstd) by default. JSON can be pretty-printed or narrowed with a JSON pointer. Text comes back as UTF-8, binary as base64. Use offset/max_bytes to page through big bodies. For gRPC, protobuf, MessagePack and CBOR bodies, format: decoded returns the messages as JSON (with imported .proto schemas, else schema-less with field numbers as keys) plus the gRPC status and trailers; offset is then the first message index."
        ),
        def!(
            "list_proto_schemas",
            Read,
            NoParams,
            "Imported protobuf schemas (.proto files or descriptor sets, imported in the app): their files, services and message counts, plus the \"Decode as\" mappings for protobuf that isn't gRPC. gRPC calls find their message types from the path when a schema defines the service."
        ),
        def!(
            "get_websocket_messages",
            Read,
            GetWsMessages,
            "Messages of a WebSocket flow, oldest first, optionally filtered by direction or text, with payloads (text, or base64 for binary) up to payload_max_bytes each. Flags: edited (a rule, script or breakpoint changed it; `original` has what arrived), injected (sent by Proxymate) and dropped (never forwarded)."
        ),
        def!(
            "send_websocket_message",
            Write,
            SendWsMessage,
            "Send a message into an open WebSocket flow (see get_websocket_messages' `open`): direction `sent` goes to the server as if the client sent it, `received` to the client as if the server did. It is recorded as injected and goes out after any message held ahead of it in that direction. A replayed WebSocket (replay_flow on one) has no client, so only `sent` works there."
        ),
        def!(
            "close_websocket",
            Destructive,
            FlowIdParam,
            "Close an open WebSocket flow: a normal close frame (1000) to each side."
        ),
        def!(
            "wait_for_flow",
            Read,
            WaitForFlow,
            "Wait until a flow matching the filter finishes (or starts, with include_active) and return it. By default only flows that start after this call count. Use it after triggering a request in the app under test."
        ),
        def!(
            "list_rules",
            Read,
            ListRules,
            "List rules in evaluation order (every matching rule runs, top to bottom). Agent-created rules have origin.kind = agent."
        ),
        def!("get_rule", Read, RuleIdParam, "One rule by id."),
        def!(
            "list_breakpoints",
            Read,
            ListBreakpoints,
            "Flows paused at a breakpoint, oldest first. Each waits for resolve_breakpoint (or the UI) until its deadline, then continues unmodified."
        ),
        def!(
            "get_breakpoint",
            Read,
            GetBreakpoint,
            "One paused flow: the request (and response, in the response phase) as it will be sent, and its body. A held WebSocket message has `message` set (direction and opcode); its payload is the body (also as body_base64 when binary) and `request` is the handshake."
        ),
        def!(
            "create_rule",
            Write,
            CreateRule,
            "Create a rule: a matcher plus ordered actions (SetHeader, AddHeader, RemoveHeader, ReplaceHeaderValue, SetQuery, RemoveQuery, ReplaceBody, SetBody, JsonSet, JsonRemove, FormSet, FormRemove, SetStatus, MapLocal, MapRemote, Breakpoint, Delay, Throttle, Block, Script, WsMessage, ServeFrom). It is added last, enabled, and tagged as yours. Prefer create_map_local, create_rewrite or create_breakpoint for the common cases. A WebSocket's handshake goes through the rules like a request (header and query actions, Map Remote, Block, request breakpoints; response header actions on the 101); WsMessage {direction?, filter?, op: Drop | Replace | JsonSet | JsonRemove | Delay | Breakpoint} acts on its text and binary messages."
        ),
        def!(
            "update_rule",
            Destructive,
            UpdateRule,
            "Change a rule's name, enabled flag, matcher or actions (fields left out stay as they are). Its origin is kept."
        ),
        def!("set_rule_enabled", Write, SetRuleEnabled, "Turn a rule on or off."),
        def!("delete_rule", Destructive, RuleIdParam, "Delete a rule."),
        def!(
            "delete_agent_rules",
            Destructive,
            NoParams,
            "Delete every rule created by an agent (any MCP client). Returns how many."
        ),
        def!(
            "create_map_local",
            Write,
            CreateMapLocal,
            "Mock an endpoint: requests matching `url` (or the same method, host and path as flow `flow_id`) are answered locally with this status, body and headers, without contacting the server."
        ),
        def!(
            "create_serve_from",
            Write,
            CreateServeFrom,
            "Serve offline from a recording: copies a session, HAR or Charles file (`path`), an open document or the live capture (`store`), or reuses a mock library entry (`source`), then creates one ServeFrom rule over the recording's hosts. Matching requests (same method, scheme, host, path and query in any order; optionally the body) get the recorded response byte for byte, without contacting the server; a miss passes through (default), or answers 404 or 502."
        ),
        def!(
            "list_mock_sources",
            Read,
            NoParams,
            "The mock library: recordings ServeFrom rules answer from, with their hosts and flow counts."
        ),
        def!(
            "create_breakpoint",
            Write,
            CreateBreakpoint,
            "Pause matching requests (and/or responses) so they can be inspected and edited with get_breakpoint and resolve_breakpoint (or in the UI)."
        ),
        def!(
            "create_rewrite",
            Write,
            CreateRewrite,
            "Rewrite matching traffic: set, add or remove headers, change the response status, set or remove query parameters, find-and-replace in a body, set or remove JSON values by pointer (only those bytes change; `raw` keeps exact number text), or set and remove form fields. Add any other rule actions with `actions`. Try a rule on a recorded flow first with test_rule."
        ),
        def!(
            "test_rule",
            Read,
            TestRule,
            "Dry run: what a rule (a saved one by `rule_id`, or an unsaved `rule`) would do to a recorded flow: whether it matches (and why not), the request and response after it, and notes (blocked, mapped, paused…). Nothing is sent or saved."
        ),
        def!(
            "resolve_breakpoint",
            Destructive,
            ResolveBreakpoint,
            "Let a paused flow go on: continue (optionally with edits to method, URL, status, headers or body), abort (reset the connection) or respond with your own response. The first resolution wins. A held WebSocket message: continue (edit `body`, or `body_base64` for bytes) or abort to drop it; it can't be answered."
        ),
        def!(
            "set_recording",
            Write,
            SetRecording,
            "Pause or resume recording. While paused, traffic is still forwarded but not kept."
        ),
        def!(
            "clear_flows",
            Destructive,
            NoParams,
            "Delete every recorded flow and its bodies."
        ),
        def!(
            "export_curl",
            Read,
            FlowIdParam,
            "A curl command that sends a recorded request again (method, URL, headers and body)."
        ),
        def!(
            "export_code",
            Read,
            ExportCode,
            "A recorded request as code that sends it again: curl, fetch (JavaScript), python (requests), httpie, go (net/http), swift (URLSession), kotlin (OkHttp) or powershell."
        ),
        def!(
            "export_har",
            Read,
            ExportHar,
            "Recorded flows as a HAR 1.2 document (JSON): the listed ids, or the flows matching a filter (oldest first, up to `limit`)."
        ),
        def!(
            "replay_flow",
            Write,
            ReplayFlow,
            "Send a recorded request again, optionally modified (method, URL, headers, body). The new flow is recorded and marked as sent by you; with `wait` (default) the result has its response status."
        ),
        def!(
            "repeat_flow",
            Write,
            RepeatFlow,
            "Send a recorded request again many times (up to 10,000, up to 64 at once), e.g. to check a rate limit or a flaky endpoint. Each request is recorded as a flow. Waits until all have finished and returns the counts by status, failures and min/p50/p95/max durations."
        ),
        def!(
            "list_scripts",
            Read,
            ListScripts,
            "List scripts (JavaScript run by Proxymate, with no network or file access): view scripts add decoded views of bodies to the inspector; rewrite scripts (onRequest/onResponse) and mock scripts (onMock) run from a rule's Script action. The API's type definitions are the resource proxymate://scripts/api."
        ),
        def!("get_script", Read, ScriptIdParam, "One script by id, with its source."),
        def!(
            "create_script",
            Write,
            CreateScript,
            "Create a script: an ES module exporting onView (kind view), onRequest and/or onResponse (kind rewrite) or onMock (kind mock); see proxymate://scripts/api for the API. It is evaluated once to check it. Rewrite and mock scripts run only from a rule: create_rule with a {type: \"Script\", script_id} action. View scripts run on bodies their `view` config matches. Tagged as yours. Try it first with test_script."
        ),
        def!(
            "update_script",
            Destructive,
            UpdateScript,
            "Change a script's name, source, enabled flag or view config (fields left out stay as they are). Its context.shared starts over."
        ),
        def!(
            "set_script_enabled",
            Write,
            SetScriptEnabled,
            "Turn a script on or off."
        ),
        def!(
            "delete_script",
            Destructive,
            ScriptIdParam,
            "Delete a script. Refused while a rule runs it; the error names those rules."
        ),
        def!(
            "delete_agent_scripts",
            Destructive,
            NoParams,
            "Delete every script created by an agent that no rule runs. Returns how many."
        ),
        def!(
            "test_script",
            Write,
            TestScript,
            "Dry-run a script (saved, by script_id, or unsaved source with its kind) against a recorded flow: the request and response before and after, a mock's answer, or the views, plus what it logged. Nothing is sent and nothing is saved."
        ),
        def!(
            "get_flow_views",
            Read,
            GetFlowViews,
            "What the enabled view scripts make of a flow's request or response body (e.g. a base64 field decoded): each script's views (text, or base64 for hex and image views) or its error."
        ),
        def!(
            "get_script_console",
            Read,
            GetScriptConsole,
            "The script console: what scripts logged with console.* and the errors they hit, oldest first. Pass the last seq you saw as `since` to get only newer entries."
        ),
        def!(
            "send_request",
            Write,
            SendRequest,
            "Send a new HTTP(S) request through Proxymate. It is recorded like proxied traffic and marked as sent by you; with `apply_rules` (default) rules such as Map Local apply to it."
        ),
    ]
}

/// The tools a client sees; read-only mode leaves out every tool that
/// changes something.
pub(crate) fn list(read_only: bool) -> Vec<Tool> {
    defs()
        .into_iter()
        .filter(|d| !read_only || !d.kind.mutates())
        .map(|d| {
            let mut description = d.description.to_owned();
            if matches!(d.name, "list_flows" | "search_flows" | "wait_for_flow") {
                description = format!("{description}\n\n{FILTER_HELP}");
            }
            let annotations = ToolAnnotations::new()
                .read_only(d.kind == Kind::Read)
                .destructive(d.kind == Kind::Destructive)
                .open_world(false);
            let mut schema = Value::Object((*(d.schema)()).clone());
            portable(&mut schema);
            let Value::Object(schema) = schema else { unreachable!() };
            Tool::new(d.name, description, Arc::new(schema)).annotate(annotations)
        })
        .collect()
}

/// Rewrites a schema into the subset every client accepts: an optional
/// value is just its type (absent means none; every optional input has a
/// serde default), not `["string", "null"]` or `anyOf [T, null]`, and "any
/// JSON value" is `{}` rather than a bare `true`.
fn portable(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Array(types)) = map.get("type") {
                let rest: Vec<Value> = types.iter().filter(|t| *t != "null").cloned().collect();
                if rest.len() == 1 && rest.len() < types.len() {
                    map.insert("type".into(), rest[0].clone());
                }
            }
            for key in ["anyOf", "oneOf"] {
                if let Some(Value::Array(options)) = map.get(key) {
                    let rest: Vec<Value> = options
                        .iter()
                        .filter(|o| o.get("type") != Some(&json!("null")))
                        .cloned()
                        .collect();
                    if rest.len() == 1 && rest.len() < options.len() {
                        map.remove(key);
                        if let Value::Object(only) = &rest[0] {
                            for (k, v) in only {
                                map.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                        }
                    }
                }
            }
            if map.get("default") == Some(&Value::Null) {
                map.remove("default");
            }
            for value in map.values_mut() {
                if *value == Value::Bool(true) {
                    *value = json!({});
                } else {
                    portable(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(portable),
        _ => {}
    }
}

pub(crate) fn kind(name: &str) -> Option<Kind> {
    defs().into_iter().find(|d| d.name == name).map(|d| d.kind)
}

/// Every tool's name, read-only ones first. For docs and tests.
pub fn tool_names() -> Vec<(&'static str, bool)> {
    defs().into_iter().map(|d| (d.name, d.kind.mutates())).collect()
}

fn parse<T: DeserializeOwned>(args: JsonObject) -> Result<T, ToolError> {
    serde_json::from_value(Value::Object(args)).map_err(|e| ToolError(format!("invalid arguments: {e}")))
}

pub(crate) async fn call(ctx: &Ctx<'_>, name: &str, args: JsonObject) -> ToolResult {
    match name {
        "search_flows" => search_flows(ctx, parse(args)?).await,
        "get_flow_body" => get_flow_body(ctx, parse(args)?).await,
        "wait_for_flow" => wait_for_flow(ctx, parse(args)?).await,
        "test_rule" => test_rule(ctx, parse(args)?).await,
        "repeat_flow" => repeat_flow(ctx, parse(args)?).await,
        "replay_flow" => replay_flow(ctx, parse(args)?).await,
        "create_script" => create_script(ctx, parse(args)?).await,
        "update_script" => update_script(ctx, parse(args)?).await,
        "set_script_enabled" => {
            let p: SetScriptEnabled = parse(args)?;
            Ok(json!(
                blocking(ctx, move |e| e.set_script_enabled(&p.id, p.enabled)).await?
            ))
        }
        "delete_script" => {
            let p: ScriptIdParam = parse(args)?;
            blocking(ctx, move |e| e.delete_script(&p.id)).await?;
            Ok(json!({ "deleted": true }))
        }
        "delete_agent_scripts" => Ok(json!({ "deleted": blocking(ctx, |e| e.delete_agent_scripts()).await? })),
        "test_script" => test_script(ctx, parse(args)?).await,
        "get_flow_views" => {
            let p: GetFlowViews = parse(args)?;
            let part = p.part.unwrap_or(BodyPart::Response);
            Ok(json!({ "results": ctx.engine.run_view_scripts(LIVE_STORE, p.id, part).await? }))
        }
        "send_request" => {
            let p: SendRequest = parse(args)?;
            send(ctx, p.request, p.apply_rules, p.wait).await
        }

        // Every other tool is a sync engine call: it runs on the blocking
        // pool (R2), so a refilter or a body read waiting for the store
        // lock never stalls the async threads that serve MCP and traffic.
        _ => {
            let name = name.to_owned();
            on_pool(ctx, move |ctx| call_sync(ctx, &name, args)).await
        }
    }
}

fn call_sync(ctx: &Ctx<'_>, name: &str, args: JsonObject) -> ToolResult {
    match name {
        "get_status" => get_status(ctx),
        "list_flows" => list_flows(ctx, parse(args)?),
        "get_flow" => get_flow(ctx, parse(args)?),
        "list_proto_schemas" => Ok(json!({
            "schemas": ctx.engine.proto_schemas(),
            "mappings": ctx.engine.proto_mappings(),
        })),
        "get_websocket_messages" => get_ws_messages(ctx, parse(args)?),
        "send_websocket_message" => {
            let p: SendWsMessage = parse(args)?;
            let message = pm_model::WsSend {
                direction: ws_direction(p.direction),
                payload: p.payload,
                binary: p.binary,
            };
            ctx.engine.ws_send(p.id, &message)?;
            Ok(json!({ "queued": true, "id": p.id }))
        }
        "close_websocket" => {
            let p: FlowIdParam = parse(args)?;
            ctx.engine.ws_close(p.id)?;
            Ok(json!({ "closing": p.id }))
        }
        "list_rules" => list_rules(ctx, parse(args)?),
        "get_rule" => {
            let p: RuleIdParam = parse(args)?;
            let rule = ctx
                .engine
                .rule(&p.id)
                .ok_or_else(|| ToolError(format!("no rule with id {}", p.id)))?;
            Ok(json!(rule))
        }
        "list_breakpoints" => list_breakpoints(ctx, parse(args)?),
        "get_breakpoint" => get_breakpoint(ctx, parse(args)?),
        "create_rule" => create_rule(ctx, parse(args)?),
        "update_rule" => update_rule(ctx, parse(args)?),
        "set_rule_enabled" => {
            let p: SetRuleEnabled = parse(args)?;
            Ok(json!(ctx.engine.set_rule_enabled(&p.id, p.enabled)?))
        }
        "delete_rule" => {
            let p: RuleIdParam = parse(args)?;
            ctx.engine.delete_rule(&p.id)?;
            Ok(json!({ "deleted": p.id }))
        }
        "delete_agent_rules" => Ok(json!({ "deleted": ctx.engine.delete_agent_rules()? })),
        "create_map_local" => create_map_local(ctx, parse(args)?),
        "create_serve_from" => create_serve_from(ctx, parse(args)?),
        "list_mock_sources" => Ok(json!({ "sources": ctx.engine.mock_sources() })),
        "create_breakpoint" => create_breakpoint(ctx, parse(args)?),
        "create_rewrite" => create_rewrite(ctx, parse(args)?),
        "resolve_breakpoint" => {
            let p: ResolveBreakpoint = parse(args)?;
            ctx.engine.resolve_breakpoint(p.id, p.resolution)?;
            Ok(json!({ "resolved": p.id }))
        }
        "set_recording" => {
            let p: SetRecording = parse(args)?;
            ctx.engine.set_recording(p.recording);
            Ok(json!({ "recording": ctx.engine.is_recording() }))
        }
        "clear_flows" => {
            let before = ctx.engine.total();
            ctx.engine.clear();
            Ok(json!({ "cleared": before }))
        }
        "export_curl" => {
            let p: FlowIdParam = parse(args)?;
            Ok(json!({ "id": p.id, "command": ctx.engine.curl_command(LIVE_STORE, p.id)? }))
        }
        "export_code" => {
            let p: ExportCode = parse(args)?;
            let code = ctx.engine.code_snippet(LIVE_STORE, p.id, p.language)?;
            Ok(json!({ "id": p.id, "language": p.language, "code": code }))
        }
        "export_har" => export_har(ctx, parse(args)?),
        "list_scripts" => list_scripts(ctx, parse(args)?),
        "get_script" => {
            let p: ScriptIdParam = parse(args)?;
            ctx.engine
                .script(&p.id)
                .map(|s| json!(s))
                .ok_or_else(|| ToolError(format!("no script with id {}", p.id)))
        }
        "get_script_console" => {
            let p: GetScriptConsole = parse(args)?;
            let entries = ctx.engine.script_console(p.since, limit(p.limit, 200, 2000) as u32);
            Ok(json!({ "entries": entries }))
        }
        other => Err(ToolError(format!("unknown tool {other:?}"))),
    }
}

/// Runs a sync tool on the blocking pool with its own copy of the context.
async fn on_pool(ctx: &Ctx<'_>, f: impl FnOnce(&Ctx<'_>) -> ToolResult + Send + 'static) -> ToolResult {
    let client = ctx.client.to_owned();
    let session = ctx.session.to_owned();
    let ct = ctx.ct.clone();
    ctx.engine
        .blocking(move |engine| {
            f(&Ctx {
                engine,
                client: &client,
                session: &session,
                ct: &ct,
            })
        })
        .await
        .map_err(|e| ToolError(format!("the tool failed: {e}")))?
}

#[derive(Deserialize, JsonSchema)]
pub(crate) struct NoParams {}

fn yes() -> bool {
    true
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OriginFilter {
    User,
    Preset,
    Agent,
}

fn cursor_id(cursor: &Option<String>) -> Result<Option<FlowId>, ToolError> {
    cursor
        .as_deref()
        .filter(|c| !c.is_empty())
        .map(|c| c.parse().map_err(|_| ToolError(format!("invalid cursor {c:?}"))))
        .transpose()
}

fn cursor_offset(cursor: &Option<String>) -> Result<usize, ToolError> {
    Ok(cursor_id(cursor)?.unwrap_or(0) as usize)
}

fn limit(value: Option<u32>, default: u32, max: u32) -> usize {
    value.unwrap_or(default).clamp(1, max) as usize
}

/// Applies an offset/limit window to a list.
fn window<T: Clone>(items: &[T], cursor: &Option<String>, limit: usize) -> Result<(Vec<T>, Option<String>), ToolError> {
    let start = cursor_offset(cursor)?.min(items.len());
    let end = (start + limit).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    Ok((items[start..end].to_vec(), next))
}

fn agent_origin(ctx: &Ctx<'_>) -> RuleOrigin {
    RuleOrigin::Agent {
        client: ctx.client.to_owned(),
        session: Some(ctx.session.to_owned()),
        created_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0),
    }
}

/// Runs an engine call that may block (saving a script evaluates it, a
/// scan reads bodies) off the async runtime's threads.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx<'_>,
    f: impl FnOnce(&Engine) -> Result<T, pm_core::Error> + Send + 'static,
) -> Result<T, ToolError> {
    Ok(ctx.engine.blocking(move |engine| f(engine)).await??)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_patterns_become_matchers() {
        let m = url_matcher("https://api.example.com/v1/users/*").unwrap();
        assert_eq!(m.scheme, Some(Scheme::Https));
        assert_eq!(m.host.as_ref().unwrap().value, "api.example.com");
        assert_eq!(m.path.as_ref().unwrap().kind, pm_model::PatternKind::Glob);

        let m = url_matcher("localhost:8080/login?x=1").unwrap();
        assert_eq!((m.scheme, m.port), (None, Some(8080)));
        assert_eq!(m.path.unwrap(), Pattern::exact("/login"));

        let m = url_matcher("*.example.com").unwrap();
        assert_eq!(m.host.unwrap().kind, pm_model::PatternKind::Glob);
        assert!(m.path.is_none());

        assert!(url_matcher("ftp://x/").is_err());
        assert!(url_matcher("").is_err());
    }

    #[test]
    fn every_schema_is_an_object_and_read_only_hides_writes() {
        let all = list(false);
        assert_eq!(all.len(), defs().len());
        for tool in &all {
            assert_eq!(
                tool.input_schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{}",
                tool.name
            );
            assert!(tool.annotations.is_some());
        }
        let text = serde_json::to_string(&all).unwrap();
        assert!(!text.contains("\"null\""), "nullable types left in the schemas");
        assert!(!text.contains("\"value\":true"), "bare true schema left");
        let read = list(true);
        assert!(read.iter().all(|t| kind(&t.name) == Some(Kind::Read)));
        assert!(read.len() < all.len());
    }
}
