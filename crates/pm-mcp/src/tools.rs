//! The MCP tool surface. Each tool's input schema comes from its parameter
//! struct's `schemars` derive, which embeds the `pm-model` types (`Matcher`,
//! `Action`, `BreakpointResolution`), so agents send exactly what the
//! engine takes. Every tool calls only the `Engine` facade.
//!
//! Lists page with an opaque `cursor`; bodies and payloads are capped.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use pm_core::{Engine, FlowPage};
use pm_model::{
    Action, BodyPart, BreakpointId, BreakpointResolution, FlowDetail, FlowId, FlowSummary, Header, MapLocalSource,
    Matcher, Pattern, Phase, PresetKind, Rule, RuleOrigin, Scheme, SearchScope, WsDirection, WsQuery,
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
            "One flow: summary, request and response heads, body metadata, timing, TLS, the rules that changed it and any error."
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
        "get_status" => get_status(ctx),
        "list_flows" => list_flows(ctx, parse(args)?),
        "search_flows" => search_flows(ctx, parse(args)?).await,
        "get_flow" => get_flow(ctx, parse(args)?),
        "get_flow_body" => get_flow_body(ctx, parse(args)?).await,
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
        "wait_for_flow" => wait_for_flow(ctx, parse(args)?).await,
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
        "test_rule" => test_rule(ctx, parse(args)?).await,
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
        "repeat_flow" => repeat_flow(ctx, parse(args)?).await,
        "export_har" => export_har(ctx, parse(args)?),
        "replay_flow" => replay_flow(ctx, parse(args)?).await,
        "list_scripts" => list_scripts(ctx, parse(args)?),
        "get_script" => {
            let p: ScriptIdParam = parse(args)?;
            ctx.engine
                .script(&p.id)
                .map(|s| json!(s))
                .ok_or_else(|| ToolError(format!("no script with id {}", p.id)))
        }
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
        "get_script_console" => {
            let p: GetScriptConsole = parse(args)?;
            let entries = ctx.engine.script_console(p.since, limit(p.limit, 200, 2000) as u32);
            Ok(json!({ "entries": entries }))
        }
        "send_request" => {
            let p: SendRequest = parse(args)?;
            send(ctx, p.request, p.apply_rules, p.wait).await
        }
        other => Err(ToolError(format!("unknown tool {other:?}"))),
    }
}

// ---- Parameters ----

#[derive(Deserialize, JsonSchema)]
pub(crate) struct NoParams {}

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Order {
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Deserialize, JsonSchema)]
struct ListFlows {
    /// Filter DSL, e.g. `host:api.example.com status:5xx`. Empty lists everything.
    #[serde(default)]
    filter: String,
    /// Only flows with a larger id (newer than this one).
    #[serde(default)]
    since_id: Option<FlowId>,
    /// 1–500, default 50.
    #[serde(default)]
    limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    sort: Order,
}

#[derive(Deserialize, JsonSchema)]
struct SearchFlows {
    /// Text to find (case-insensitive), or a regex with `regex: true`.
    query: String,
    #[serde(default)]
    regex: bool,
    /// Where to look. Default: everywhere.
    #[serde(default)]
    scopes: Vec<SearchScope>,
    /// Only search flows matching this filter DSL.
    #[serde(default)]
    filter: String,
    /// 1–100, default 20.
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Include {
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
struct GetFlow {
    id: FlowId,
    /// Default: headers, timing, rules.
    #[serde(default)]
    include: Option<Vec<Include>>,
}

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Part {
    Request,
    #[default]
    Response,
}

impl From<Part> for BodyPart {
    fn from(p: Part) -> BodyPart {
        match p {
            Part::Request => BodyPart::Request,
            Part::Response => BodyPart::Response,
        }
    }
}

#[derive(Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BodyFormat {
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
struct GetFlowBody {
    id: FlowId,
    #[serde(default)]
    part: Part,
    /// Undo Content-Encoding (default true).
    #[serde(default = "yes")]
    decode: bool,
    #[serde(default)]
    format: BodyFormat,
    /// RFC 6901 pointer into a JSON body, e.g. `/data/items/0`.
    #[serde(default)]
    json_pointer: Option<String>,
    /// Byte offset into the (formatted) body.
    #[serde(default)]
    offset: u64,
    /// Default 65536, at most 1048576.
    #[serde(default)]
    max_bytes: Option<u32>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Direction {
    /// Client to server.
    Sent,
    /// Server to client.
    Received,
}

#[derive(Deserialize, JsonSchema)]
struct SendWsMessage {
    /// An open WebSocket flow.
    id: FlowId,
    direction: Direction,
    /// Text as is, or base64 with `binary`.
    payload: String,
    /// Send a binary message; `payload` is base64.
    #[serde(default)]
    binary: bool,
}

#[derive(Deserialize, JsonSchema)]
struct GetWsMessages {
    id: FlowId,
    #[serde(default)]
    direction: Option<Direction>,
    /// Case-insensitive text the payload must contain.
    #[serde(default)]
    text: String,
    /// 1–500, default 50.
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
    /// Payload bytes per message, default 4096 (0 leaves payloads out).
    #[serde(default)]
    payload_max_bytes: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct WaitForFlow {
    /// Filter DSL the flow must match, e.g. `host:api.example.com path:/login`.
    #[serde(default)]
    filter: String,
    /// Default 30000, at most 120000.
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Only flows with a larger id. Default: flows that start after this call.
    #[serde(default)]
    after_id: Option<FlowId>,
    /// Return as soon as a matching flow starts, not when it finishes.
    #[serde(default)]
    include_active: bool,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OriginFilter {
    User,
    Preset,
    Agent,
}

#[derive(Deserialize, JsonSchema)]
struct ListRules {
    #[serde(default)]
    origin: Option<OriginFilter>,
    /// 1–200, default 50.
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ListScripts {
    #[serde(default)]
    kind: Option<ScriptKind>,
    #[serde(default)]
    origin: Option<OriginFilter>,
}

#[derive(Deserialize, JsonSchema)]
struct ScriptIdParam {
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct CreateScript {
    name: String,
    kind: ScriptKind,
    /// An ES module exporting the kind's entry points.
    source: String,
    #[serde(default = "yes")]
    enabled: bool,
    /// View scripts: which bodies get the view (default: every response and request body).
    #[serde(default)]
    view: Option<ViewConfig>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateScript {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    view: Option<ViewConfig>,
}

#[derive(Deserialize, JsonSchema)]
struct SetScriptEnabled {
    id: String,
    enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
struct TestScript {
    /// The recorded flow to run it against.
    flow_id: FlowId,
    /// A saved script to test.
    #[serde(default)]
    script_id: Option<String>,
    /// Unsaved source (with `kind`), tested instead of the saved script.
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    kind: Option<ScriptKind>,
    /// View scripts: which body to view (default response).
    #[serde(default)]
    part: Option<BodyPart>,
}

#[derive(Deserialize, JsonSchema)]
struct GetFlowViews {
    id: FlowId,
    /// Default response.
    #[serde(default)]
    part: Option<BodyPart>,
}

#[derive(Deserialize, JsonSchema)]
struct GetScriptConsole {
    /// Only entries with a larger seq.
    #[serde(default)]
    since: Option<u64>,
    /// 1–2000, default 200 (the newest ones).
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub(crate) struct RuleIdParam {
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct ListBreakpoints {
    /// 1–200, default 50.
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GetBreakpoint {
    id: BreakpointId,
    /// Body bytes to return as text (default 65536, at most 1048576; 0 leaves it out).
    #[serde(default)]
    max_bytes: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateRule {
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    matcher: Matcher,
    actions: Vec<Action>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateRule {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    matcher: Option<Matcher>,
    #[serde(default)]
    actions: Option<Vec<Action>>,
}

#[derive(Deserialize, JsonSchema)]
struct SetRuleEnabled {
    id: String,
    enabled: bool,
}

/// Which requests a sugar tool's rule matches: a URL pattern, or a
/// recorded flow's method, host and path.
#[derive(Deserialize, JsonSchema)]
struct Target {
    /// URL pattern: `https://api.example.com/v1/users/*`, `api.example.com/login`,
    /// `*.example.com/*`. `*` is a wildcard (it also crosses `/`); without a
    /// path every path matches. The query string is ignored.
    #[serde(default)]
    url: Option<String>,
    /// Match the same method, host and path as this recorded flow.
    #[serde(default)]
    flow_id: Option<FlowId>,
    /// Only this method (e.g. `GET`); default any (with `url`) or the flow's (with `flow_id`).
    #[serde(default)]
    method: Option<String>,
    /// Rule name; default describes the target.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateMapLocal {
    #[serde(flatten)]
    target: Target,
    /// Default 200.
    #[serde(default)]
    status: Option<u16>,
    /// The response body. With `flow_id` and no body, the flow's recorded response body.
    #[serde(default)]
    body: Option<String>,
    /// Serve this file instead of `body`.
    #[serde(default)]
    file: Option<String>,
    /// e.g. `application/json`; guessed from the body when unset.
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    headers: Vec<Header>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateServeFrom {
    /// A `.proxymate` session, HAR, or Charles `.chlsj`/`.chlsx` file to copy into the mock library.
    #[serde(default)]
    path: Option<String>,
    /// Or an open document's store id (0 is the live capture).
    #[serde(default)]
    store: Option<StoreId>,
    /// Or the id of a mock library entry (list_mock_sources).
    #[serde(default)]
    source: Option<String>,
    /// The library entry's name (default: the file or document name).
    #[serde(default)]
    name: Option<String>,
    /// What a request must share with a recorded one (default: the query counts, the body doesn't).
    #[serde(default)]
    key: Option<ServeKey>,
    /// pass_through (default), not_found (404) or bad_gateway (502).
    #[serde(default)]
    on_miss: Option<ServeMiss>,
    /// first (default), last or round_robin, when several recorded responses match.
    #[serde(default)]
    pick: Option<ServePick>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateBreakpoint {
    #[serde(flatten)]
    target: Target,
    /// Pause the request before it is sent (default true).
    #[serde(default = "yes")]
    request: bool,
    /// Pause the response before the client gets it (default false).
    #[serde(default)]
    response: bool,
}

#[derive(Deserialize, JsonSchema)]
struct Replace {
    /// Literal text to find (or a regex with `regex: true`).
    find: String,
    replace: String,
    #[serde(default)]
    regex: bool,
    /// Default response.
    #[serde(default = "response_phase")]
    phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
struct JsonEdit {
    /// RFC 6901 pointer, e.g. `/user/name`; `-` appends to an array.
    pointer: String,
    /// The new value (default null).
    #[serde(default)]
    value: Value,
    /// The new value as exact JSON text instead, e.g. `12345678901234567890` or `1.50`.
    #[serde(default)]
    raw: Option<String>,
    /// Default response.
    #[serde(default = "response_phase")]
    phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
struct JsonRemoveEdit {
    /// RFC 6901 pointer, e.g. `/user/token`.
    pointer: String,
    /// Default response.
    #[serde(default = "response_phase")]
    phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
struct NameValue {
    name: String,
    value: String,
}

#[derive(Deserialize, JsonSchema)]
struct FormField {
    name: String,
    value: String,
    /// Default request.
    #[serde(default = "request_phase")]
    phase: Phase,
}

#[derive(Deserialize, JsonSchema)]
struct FormFieldName {
    name: String,
    /// Default request.
    #[serde(default = "request_phase")]
    phase: Phase,
}

fn response_phase() -> Phase {
    Phase::Response
}

fn request_phase() -> Phase {
    Phase::Request
}

#[derive(Deserialize, JsonSchema)]
struct CreateRewrite {
    #[serde(flatten)]
    target: Target,
    /// Headers to set on the request.
    #[serde(default)]
    set_request_headers: Vec<Header>,
    /// Headers to set on the response.
    #[serde(default)]
    set_response_headers: Vec<Header>,
    #[serde(default)]
    remove_request_headers: Vec<String>,
    #[serde(default)]
    remove_response_headers: Vec<String>,
    /// Header values to add to the request, keeping existing ones (after
    /// the removals: remove a header, then add each of its new values).
    #[serde(default)]
    add_request_headers: Vec<Header>,
    #[serde(default)]
    add_response_headers: Vec<Header>,
    /// Change the response status.
    #[serde(default)]
    status: Option<u16>,
    /// Query parameters to set (every occurrence replaced, or added).
    #[serde(default)]
    set_query: Vec<NameValue>,
    #[serde(default)]
    remove_query: Vec<String>,
    #[serde(default)]
    replace_body: Vec<Replace>,
    /// JSON values to remove (before `json_set`).
    #[serde(default)]
    json_remove: Vec<JsonRemoveEdit>,
    #[serde(default)]
    json_set: Vec<JsonEdit>,
    /// Fields of a form-urlencoded body to set.
    #[serde(default)]
    form_set: Vec<FormField>,
    #[serde(default)]
    form_remove: Vec<FormFieldName>,
    /// Any further actions, run after the ones above.
    #[serde(default)]
    actions: Vec<Action>,
}

/// A rule to try without saving it.
#[derive(Deserialize, JsonSchema)]
struct DraftRule {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    matcher: Matcher,
    actions: Vec<Action>,
}

#[derive(Deserialize, JsonSchema)]
struct TestRule {
    /// A saved rule to try. Give this or `rule`.
    #[serde(default)]
    rule_id: Option<String>,
    /// An unsaved rule to try.
    #[serde(default)]
    rule: Option<DraftRule>,
    /// The recorded flow to try it on.
    flow_id: FlowId,
    /// The flow's store: 0, the live capture (default), or an opened session's.
    #[serde(default)]
    store: Option<StoreId>,
}

#[derive(Deserialize, JsonSchema)]
struct ResolveBreakpoint {
    id: BreakpointId,
    resolution: BreakpointResolution,
}

#[derive(Deserialize, JsonSchema)]
struct SetRecording {
    recording: bool,
}

#[derive(Deserialize, JsonSchema)]
struct FlowIdParam {
    id: FlowId,
}

#[derive(Deserialize, JsonSchema)]
struct ExportHar {
    /// These flows, in this order. Without, the flows matching `filter`.
    #[serde(default)]
    ids: Option<Vec<FlowId>>,
    /// Filter DSL; empty exports everything (up to `limit`).
    #[serde(default)]
    filter: String,
    /// 1–500, default 50.
    #[serde(default)]
    limit: Option<u32>,
}

/// Changes to a recorded request before it is sent again. Unset fields
/// stay as recorded.
#[derive(Deserialize, JsonSchema, Default)]
struct RequestEdits {
    #[serde(default)]
    method: Option<String>,
    /// An absolute URL.
    #[serde(default)]
    url: Option<String>,
    /// Headers to set (replacing any of the same name).
    #[serde(default)]
    set_headers: Vec<Header>,
    #[serde(default)]
    remove_headers: Vec<String>,
    /// Replaces the body (the recorded one is decoded first, so edits are text).
    #[serde(default)]
    body: Option<String>,
    /// `body` is base64.
    #[serde(default)]
    body_base64: bool,
}

#[derive(Deserialize, JsonSchema)]
struct ReplayFlow {
    id: FlowId,
    #[serde(default)]
    modify: Option<RequestEdits>,
    /// Run it through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    apply_rules: bool,
    /// Wait for the response (default true).
    #[serde(default = "yes")]
    wait: bool,
}

#[derive(Deserialize, JsonSchema)]
struct ExportCode {
    id: FlowId,
    language: CodeLanguage,
}

#[derive(Deserialize, JsonSchema)]
struct RepeatFlow {
    id: FlowId,
    /// How many times to send it, 1 to 10,000.
    iterations: u32,
    /// Requests in flight at once, 1 to 64 (default 1).
    #[serde(default = "one")]
    concurrency: u32,
    /// Pause before each request, in milliseconds (default 0).
    #[serde(default)]
    delay_ms: u32,
    /// Run each request through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    apply_rules: bool,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, JsonSchema)]
struct SendRequest {
    request: RequestSpec,
    /// Run it through the rules like proxied traffic (default true).
    #[serde(default = "yes")]
    apply_rules: bool,
    /// Wait for the response (default true).
    #[serde(default = "yes")]
    wait: bool,
}

// ---- Shapes returned ----

fn url_of(f: &FlowSummary) -> String {
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
fn brief(f: &FlowSummary) -> Value {
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

// ---- Read tools ----

fn get_status(ctx: &Ctx<'_>) -> ToolResult {
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

fn list_flows(ctx: &Ctx<'_>, p: ListFlows) -> ToolResult {
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

async fn search_flows(ctx: &Ctx<'_>, p: SearchFlows) -> ToolResult {
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
    let engine = ctx.engine.clone();
    let cursor = cursor_id(&p.cursor)?;
    let max = limit(p.limit, 20, 100);
    // Reading bodies is blocking work.
    let page =
        tokio::task::spawn_blocking(move || engine.search_flows(&p.query, p.regex, &scopes, &p.filter, cursor, max))
            .await
            .map_err(|e| ToolError(e.to_string()))??;
    Ok(json!({
        "hits": page.hits.iter().map(|h| json!({
            "flow": brief(&h.flow),
            "matches": h.matches.iter().map(|m| json!({ "scope": m.scope, "snippet": m.snippet })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "scanned": page.scanned,
        "next_cursor": page.next.map(|id| id.to_string()),
    }))
}

fn detail_of(ctx: &Ctx<'_>, id: FlowId) -> Result<(FlowSummary, FlowDetail), ToolError> {
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

fn get_flow(ctx: &Ctx<'_>, p: GetFlow) -> ToolResult {
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

async fn get_flow_body(ctx: &Ctx<'_>, p: GetFlowBody) -> ToolResult {
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
    let engine = ctx.engine.clone();
    let id = p.id;
    let body = tokio::task::spawn_blocking(move || engine.body_head(id, part, p.decode, Some(read_max)))
        .await
        .map_err(|e| ToolError(e.to_string()))??;

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
async fn decoded_body(ctx: &Ctx<'_>, p: GetFlowBody) -> ToolResult {
    let part: BodyPart = p.part.into();
    let engine = ctx.engine.clone();
    let id = p.id;
    let decoded = tokio::task::spawn_blocking(move || engine.decode_body_in(LIVE_STORE, id, part))
        .await
        .map_err(|e| ToolError(e.to_string()))??;
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

fn get_ws_messages(ctx: &Ctx<'_>, p: GetWsMessages) -> ToolResult {
    let query = WsQuery {
        direction: p.direction.map(ws_direction),
        text: p.text,
    };
    let start = cursor_offset(&p.cursor)? as u32;
    let count = limit(p.limit, 50, 500) as u32;
    let page = ctx.engine.websocket_messages(p.id, &query, start, count)?;
    let payload_max = p.payload_max_bytes.map_or(4096, |m| (m as usize).min(BODY_MAX));
    let messages: Vec<Value> = page
        .messages
        .iter()
        .map(|m| {
            let mut v = json!(m);
            if payload_max > 0 {
                if let Ok(payload) = ctx.engine.websocket_payload(p.id, m.index) {
                    let (text, encoding, cut) = payload_json(&payload, m.opcode, payload_max);
                    v["payload"] = json!(text);
                    v["payload_encoding"] = json!(encoding);
                    v["payload_cut"] = json!(cut);
                }
                if m.edited
                    && let Ok(Some(original)) = ctx.engine.websocket_original_in(LIVE_STORE, p.id, m.index)
                {
                    let (text, encoding, _) = payload_json(&original, m.opcode, payload_max);
                    v["original"] = json!(text);
                    v["original_encoding"] = json!(encoding);
                }
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("preview");
                }
            }
            v
        })
        .collect();
    let end = page.start + page.messages.len() as u32;
    Ok(json!({
        "total": page.total,
        "matched": page.len,
        "messages": messages,
        "open": page.open,
        "close": page.close,
        "next_cursor": (end < page.len).then(|| end.to_string()),
    }))
}

fn ws_direction(direction: Direction) -> WsDirection {
    match direction {
        Direction::Sent => WsDirection::Sent,
        Direction::Received => WsDirection::Received,
    }
}

/// A payload cut at `max`: UTF-8 text, or base64 for binary; and whether
/// it was cut.
fn payload_json(payload: &[u8], opcode: pm_model::WsOpcode, max: usize) -> (String, &'static str, bool) {
    let cut = &payload[..payload.len().min(max)];
    let (text, encoding) = match std::str::from_utf8(cut) {
        Ok(s) if opcode != pm_model::WsOpcode::Binary => (s.to_owned(), "utf8"),
        _ => (base64::engine::general_purpose::STANDARD.encode(cut), "base64"),
    };
    (text, encoding, payload.len() > max)
}

async fn wait_for_flow(ctx: &Ctx<'_>, p: WaitForFlow) -> ToolResult {
    let query = ctx
        .engine
        .parse_query(&p.filter)
        .map_err(|e| ToolError(e.to_string()))?;
    let after = p.after_id.or_else(|| {
        // Flows that start after this call: everything up to the newest now.
        let newest = ctx.engine.find_flows("", None, None, 1, true).ok()?;
        newest.flows.first().map(|f| f.id)
    });
    let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(WAIT_DEFAULT_MS).min(WAIT_MAX_MS));
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(flow) = ctx.engine.first_flow_matching(&query, after, p.include_active) {
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

fn list_rules(ctx: &Ctx<'_>, p: ListRules) -> ToolResult {
    let rules: Vec<Rule> = ctx
        .engine
        .rules()
        .into_iter()
        .filter(|r| match p.origin {
            None => true,
            Some(OriginFilter::User) => matches!(r.origin, RuleOrigin::User),
            Some(OriginFilter::Preset) => matches!(r.origin, RuleOrigin::Preset { .. }),
            Some(OriginFilter::Agent) => matches!(r.origin, RuleOrigin::Agent { .. }),
        })
        .collect();
    let (page, next) = window(&rules, &p.cursor, limit(p.limit, 50, 200))?;
    Ok(json!({ "rules": page, "total": rules.len(), "next_cursor": next }))
}

fn list_breakpoints(ctx: &Ctx<'_>, p: ListBreakpoints) -> ToolResult {
    let queue = ctx.engine.breakpoints();
    let (page, next) = window(&queue, &p.cursor, limit(p.limit, 50, 200))?;
    Ok(json!({ "breakpoints": page, "total": queue.len(), "next_cursor": next }))
}

fn get_breakpoint(ctx: &Ctx<'_>, p: GetBreakpoint) -> ToolResult {
    let info = ctx.engine.breakpoint(p.id).ok_or_else(|| {
        ToolError(format!(
            "no paused flow with breakpoint id {} (resolved or timed out?)",
            p.id
        ))
    })?;
    let max = p.max_bytes.map_or(BODY_DEFAULT, |m| (m as usize).min(BODY_MAX));
    let mut out = json!(info);
    if max > 0 {
        let body = ctx.engine.breakpoint_body(p.id)?;
        let cut = &body[..body.len().min(max)];
        out["body_text"] = json!(String::from_utf8_lossy(cut));
        out["body_cut"] = json!(body.len() > max);
        if info
            .message
            .as_ref()
            .is_some_and(|m| m.opcode == pm_model::WsOpcode::Binary)
        {
            out["body_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(cut));
        }
    }
    Ok(out)
}

fn export_har(ctx: &Ctx<'_>, p: ExportHar) -> ToolResult {
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

// ---- Write tools ----

/// Sends `spec` as this agent and describes the new flow.
async fn send(ctx: &Ctx<'_>, spec: RequestSpec, apply_rules: bool, wait: bool) -> ToolResult {
    let opts = SendOpts {
        apply_rules,
        wait,
        origin: SendOrigin::Agent {
            client: ctx.client.to_owned(),
        },
    };
    let id = ctx.engine.send(spec, opts).await?;
    let flow = ctx.engine.get_flow(id).map(|f| brief(&f));
    let error = ctx.engine.flow_detail(id).ok().flatten().and_then(|d| d.error);
    Ok(json!({ "id": id, "flow": flow, "error": error }))
}

async fn repeat_flow(ctx: &Ctx<'_>, p: RepeatFlow) -> ToolResult {
    let spec = ctx.engine.request_spec(LIVE_STORE, p.id, false)?;
    let opts = RepeatOpts {
        iterations: p.iterations,
        concurrency: p.concurrency,
        delay_ms: p.delay_ms,
        apply_rules: p.apply_rules,
    };
    let origin = SendOrigin::Agent {
        client: ctx.client.to_owned(),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let id = ctx.engine.repeat_as(
        spec,
        opts,
        origin,
        Box::new(move |progress| {
            if progress.finished
                && let Some(tx) = tx.take()
            {
                let _ = tx.send(progress);
            }
        }),
    )?;
    let done = tokio::select! {
        done = rx => done.map_err(|_| ToolError("the repeat stopped without a result".into()))?,
        () = ctx.ct.cancelled() => {
            ctx.engine.cancel_repeat(id);
            return Err(ToolError("cancelled".into()));
        }
    };
    if let Some(error) = &done.error
        && done.done == 0
    {
        return Err(ToolError(error.clone()));
    }
    Ok(json!({
        "repeated": p.id,
        "sent": done.done,
        "failed": done.failed,
        "statuses": done.statuses,
        "min_ms": done.min_ms,
        "p50_ms": done.p50_ms,
        "p95_ms": done.p95_ms,
        "max_ms": done.max_ms,
        "first_flow": done.first_flow,
        "last_flow": done.last_flow,
        "error": done.error,
    }))
}

async fn replay_flow(ctx: &Ctx<'_>, p: ReplayFlow) -> ToolResult {
    let Some(edits) = p.modify else {
        let opts = SendOpts {
            apply_rules: p.apply_rules,
            wait: p.wait,
            origin: SendOrigin::Agent {
                client: ctx.client.to_owned(),
            },
        };
        let id = ctx.engine.replay(LIVE_STORE, p.id, opts).await?;
        let flow = ctx.engine.get_flow(id).map(|f| brief(&f));
        let error = ctx.engine.flow_detail(id).ok().flatten().and_then(|d| d.error);
        return Ok(json!({ "id": id, "replayed": p.id, "flow": flow, "error": error }));
    };
    let mut spec = ctx.engine.request_spec(LIVE_STORE, p.id, true)?;
    if let Some(method) = edits.method {
        spec.method = method;
    }
    if let Some(url) = edits.url {
        spec.url = url;
    }
    let drop: Vec<String> = edits
        .remove_headers
        .iter()
        .chain(edits.set_headers.iter().map(|h| &h.name))
        .map(|n| n.to_ascii_lowercase())
        .collect();
    spec.headers.retain(|h| !drop.contains(&h.name.to_ascii_lowercase()));
    spec.headers.extend(edits.set_headers);
    if let Some(body) = edits.body {
        spec.body = Some(body);
        spec.body_base64 = edits.body_base64;
    }
    let mut out = send(ctx, spec, p.apply_rules, p.wait).await?;
    out["replayed"] = json!(p.id);
    Ok(out)
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

/// Runs an engine call that may block (saving a script evaluates it) off
/// the async runtime's threads.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx<'_>,
    f: impl FnOnce(&Engine) -> Result<T, pm_core::Error> + Send + 'static,
) -> Result<T, ToolError> {
    let engine = ctx.engine.clone();
    Ok(tokio::task::spawn_blocking(move || f(&engine)).await??)
}

fn list_scripts(ctx: &Ctx<'_>, p: ListScripts) -> ToolResult {
    let scripts: Vec<Script> = ctx
        .engine
        .scripts()
        .into_iter()
        .filter(|s| p.kind.is_none_or(|k| k == s.kind))
        .filter(|s| match p.origin {
            None => true,
            Some(OriginFilter::User) => matches!(s.origin, RuleOrigin::User),
            Some(OriginFilter::Preset) => matches!(s.origin, RuleOrigin::Preset { .. }),
            Some(OriginFilter::Agent) => matches!(s.origin, RuleOrigin::Agent { .. }),
        })
        .collect();
    Ok(json!({ "scripts": scripts, "total": scripts.len() }))
}

async fn create_script(ctx: &Ctx<'_>, p: CreateScript) -> ToolResult {
    let script = Script {
        id: String::new(),
        name: p.name,
        kind: p.kind,
        enabled: p.enabled,
        source: p.source,
        origin: agent_origin(ctx),
        view: match p.kind {
            ScriptKind::View => Some(p.view.unwrap_or_default()),
            _ => None,
        },
        updated_at_ms: 0.0,
    };
    Ok(json!(blocking(ctx, move |e| e.create_script(script)).await?))
}

async fn update_script(ctx: &Ctx<'_>, p: UpdateScript) -> ToolResult {
    let mut script = ctx
        .engine
        .script(&p.id)
        .ok_or_else(|| ToolError(format!("no script with id {}", p.id)))?;
    if let Some(name) = p.name {
        script.name = name;
    }
    if let Some(source) = p.source {
        script.source = source;
    }
    if let Some(enabled) = p.enabled {
        script.enabled = enabled;
    }
    if p.view.is_some() {
        script.view = p.view;
    }
    Ok(json!(blocking(ctx, move |e| e.update_script(script)).await?))
}

async fn test_script(ctx: &Ctx<'_>, p: TestScript) -> ToolResult {
    let spec = ScriptTestSpec {
        script_id: p.script_id,
        source: p.source,
        kind: p.kind,
        part: p.part,
    };
    Ok(json!(ctx.engine.test_script(LIVE_STORE, p.flow_id, spec).await?))
}

/// Saves a new rule as the agent's.
fn save(ctx: &Ctx<'_>, name: String, matcher: Matcher, actions: Vec<Action>) -> ToolResult {
    if actions.is_empty() {
        return Err(ToolError("a rule needs at least one action".into()));
    }
    let rule = ctx.engine.create_rule(Rule {
        id: String::new(),
        name,
        enabled: true,
        order: 0,
        origin: agent_origin(ctx),
        matcher,
        actions,
    })?;
    Ok(json!(rule))
}

fn create_rule(ctx: &Ctx<'_>, p: CreateRule) -> ToolResult {
    let mut rule = save(ctx, p.name, p.matcher, p.actions)?;
    if !p.enabled
        && let Some(id) = rule["id"].as_str()
    {
        rule = json!(ctx.engine.set_rule_enabled(id, false)?);
    }
    Ok(rule)
}

fn update_rule(ctx: &Ctx<'_>, p: UpdateRule) -> ToolResult {
    let mut rule = ctx
        .engine
        .rule(&p.id)
        .ok_or_else(|| ToolError(format!("no rule with id {}", p.id)))?;
    if let Some(name) = p.name {
        rule.name = name;
    }
    if let Some(enabled) = p.enabled {
        rule.enabled = enabled;
    }
    if let Some(matcher) = p.matcher {
        rule.matcher = matcher;
    }
    if let Some(actions) = p.actions {
        rule.actions = actions;
    }
    Ok(json!(ctx.engine.update_rule(rule)?))
}

/// Turns a URL pattern into a matcher: scheme, host (exact or glob) and
/// path (exact or glob; none matches every path).
fn url_matcher(url: &str) -> Result<Matcher, ToolError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(ToolError("url is empty".into()));
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = match scheme.to_ascii_lowercase().as_str() {
                "http" => Scheme::Http,
                "https" => Scheme::Https,
                "ws" => Scheme::Ws,
                "wss" => Scheme::Wss,
                "*" => return url_matcher(rest),
                other => return Err(ToolError(format!("unsupported scheme {other:?}"))),
            };
            (Some(scheme), rest)
        }
        None => (None, url),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], Some(&rest[at..])),
        None => (rest, None),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') || host.ends_with(']') => match port.parse::<u16>() {
            Ok(port) => (host, Some(port)),
            Err(_) if port == "*" => (host, None),
            Err(_) => return Err(ToolError(format!("invalid port in {url:?}"))),
        },
        _ => (authority, None),
    };
    if host.is_empty() {
        return Err(ToolError(format!("no host in {url:?}")));
    }
    let wild = |s: &str| s.contains(['*', '?']);
    let host = if host == "*" {
        None
    } else if wild(host) {
        Some(Pattern::glob(host))
    } else {
        Some(Pattern {
            case_insensitive: true,
            ..Pattern::exact(host)
        })
    };
    let path = match path {
        None | Some("/*") | Some("*") => None,
        Some(p) if wild(p) => Some(Pattern {
            case_insensitive: false,
            ..Pattern::glob(p)
        }),
        Some(p) => Some(Pattern::exact(p)),
    };
    Ok(Matcher {
        scheme,
        host,
        port,
        path,
        ..Matcher::default()
    })
}

/// The matcher (and a default name) for a sugar tool's target.
fn target_matcher(ctx: &Ctx<'_>, target: &Target, kind: &str) -> Result<(Matcher, String), ToolError> {
    let (mut matcher, described) = match (&target.url, target.flow_id) {
        (Some(url), None) => (url_matcher(url)?, url.clone()),
        (None, Some(id)) => {
            let draft = ctx.engine.draft_rule(id, PresetKind::Breakpoint)?;
            let flow = ctx
                .engine
                .get_flow(id)
                .ok_or_else(|| ToolError(format!("no flow {id}")))?;
            (draft.matcher, format!("{} {}{}", flow.method, flow.host, flow.path))
        }
        (Some(_), Some(_)) => return Err(ToolError("give either url or flow_id, not both".into())),
        (None, None) => {
            return Err(ToolError(
                "give url (a URL pattern) or flow_id (a recorded flow)".into(),
            ));
        }
    };
    if let Some(method) = &target.method {
        matcher.methods = vec![method.to_ascii_uppercase()];
    }
    let method = matcher.methods.first().map(|m| format!("{m} ")).unwrap_or_default();
    let name = target.name.clone().unwrap_or_else(|| match &target.url {
        Some(_) => format!("{kind} · {method}{described}"),
        None => format!("{kind} · {described}"),
    });
    Ok((matcher, name))
}

fn create_map_local(ctx: &Ctx<'_>, p: CreateMapLocal) -> ToolResult {
    let (matcher, name) = target_matcher(ctx, &p.target, "Map Local")?;
    let source = match (p.file, p.body) {
        (Some(_), Some(_)) => return Err(ToolError("give either body or file, not both".into())),
        (Some(path), None) => MapLocalSource::File { path },
        (None, Some(body)) => MapLocalSource::Inline { body },
        (None, None) => match p.target.flow_id {
            // Start from what the flow answered.
            Some(id) => match ctx
                .engine
                .draft_rule(id, PresetKind::MapLocal)?
                .actions
                .into_iter()
                .next()
            {
                Some(Action::MapLocal { source, .. }) => source,
                _ => MapLocalSource::Inline { body: String::new() },
            },
            None => MapLocalSource::Inline { body: String::new() },
        },
    };
    let content_type = p.content_type.or_else(|| match &source {
        MapLocalSource::Inline { body } => {
            let t = body.trim_start();
            if t.starts_with('{') || t.starts_with('[') {
                Some("application/json".into())
            } else if t.starts_with('<') {
                Some("text/html; charset=utf-8".into())
            } else {
                Some("text/plain; charset=utf-8".into())
            }
        }
        _ => None,
    });
    let action = Action::MapLocal {
        source,
        status: p.status.unwrap_or(200),
        headers: p.headers,
        content_type,
    };
    save(ctx, name, matcher, vec![action])
}

fn create_serve_from(ctx: &Ctx<'_>, p: CreateServeFrom) -> ToolResult {
    let source = match (p.path, p.store, p.source) {
        (Some(path), None, None) => ctx
            .engine
            .create_mock_source_from_file(std::path::Path::new(&path), p.name)?,
        (None, Some(store), None) => ctx.engine.create_mock_source(store, p.name)?,
        (None, None, Some(id)) => ctx
            .engine
            .mock_sources()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| ToolError(format!("there is no mock source {id}; see list_mock_sources")))?,
        _ => return Err(ToolError("give exactly one of path, store or source".into())),
    };
    let mut draft = ctx.engine.draft_serve_from_rule(&source.id)?;
    draft.actions = vec![Action::ServeFrom {
        source: source.id.clone(),
        key: p.key.unwrap_or_default(),
        on_miss: p.on_miss.unwrap_or_default(),
        pick: p.pick.unwrap_or_default(),
    }];
    let rule = save(ctx, draft.name, draft.matcher, draft.actions)?;
    Ok(json!({ "rule": rule, "source": source }))
}

fn create_breakpoint(ctx: &Ctx<'_>, p: CreateBreakpoint) -> ToolResult {
    if !p.request && !p.response {
        return Err(ToolError("pause the request, the response or both".into()));
    }
    let (matcher, name) = target_matcher(ctx, &p.target, "Breakpoint")?;
    save(
        ctx,
        name,
        matcher,
        vec![Action::Breakpoint {
            request: p.request,
            response: p.response,
        }],
    )
}

fn create_rewrite(ctx: &Ctx<'_>, p: CreateRewrite) -> ToolResult {
    let (matcher, name) = target_matcher(ctx, &p.target, "Rewrite")?;
    let mut actions = Vec::new();
    for (phase, headers) in [
        (Phase::Request, p.set_request_headers),
        (Phase::Response, p.set_response_headers),
    ] {
        actions.extend(headers.into_iter().map(|h| Action::SetHeader {
            phase,
            name: h.name,
            value: h.value,
        }));
    }
    for (phase, names) in [
        (Phase::Request, p.remove_request_headers),
        (Phase::Response, p.remove_response_headers),
    ] {
        actions.extend(names.into_iter().map(|name| Action::RemoveHeader { phase, name }));
    }
    for (phase, headers) in [
        (Phase::Request, p.add_request_headers),
        (Phase::Response, p.add_response_headers),
    ] {
        actions.extend(headers.into_iter().map(|h| Action::AddHeader {
            phase,
            name: h.name,
            value: h.value,
        }));
    }
    if let Some(status) = p.status {
        actions.push(Action::SetStatus { status });
    }
    actions.extend(p.set_query.into_iter().map(|q| Action::SetQuery {
        name: q.name,
        value: q.value,
    }));
    actions.extend(p.remove_query.into_iter().map(|name| Action::RemoveQuery { name }));
    actions.extend(p.replace_body.into_iter().map(|r| Action::ReplaceBody {
        phase: r.phase,
        pattern: if r.regex {
            Pattern::regex(r.find)
        } else {
            Pattern::exact(r.find)
        },
        replacement: r.replace,
    }));
    actions.extend(p.json_remove.into_iter().map(|j| Action::JsonRemove {
        phase: j.phase,
        pointer: j.pointer,
    }));
    actions.extend(p.json_set.into_iter().map(|j| Action::JsonSet {
        phase: j.phase,
        pointer: j.pointer,
        value: j.value,
        raw: j.raw,
    }));
    actions.extend(p.form_set.into_iter().map(|f| Action::FormSet {
        phase: f.phase,
        name: f.name,
        value: f.value,
    }));
    actions.extend(p.form_remove.into_iter().map(|f| Action::FormRemove {
        phase: f.phase,
        name: f.name,
    }));
    actions.extend(p.actions);
    save(ctx, name, matcher, actions)
}

async fn test_rule(ctx: &Ctx<'_>, p: TestRule) -> ToolResult {
    let rule = match (p.rule_id, p.rule) {
        (Some(id), None) => ctx
            .engine
            .rule(&id)
            .ok_or_else(|| ToolError(format!("no rule with id {id}")))?,
        (None, Some(draft)) => Rule {
            id: String::new(),
            name: draft.name.unwrap_or_else(|| "Dry run".into()),
            enabled: true,
            order: 0,
            origin: agent_origin(ctx),
            matcher: draft.matcher,
            actions: draft.actions,
        },
        _ => {
            return Err(ToolError(
                "give either rule_id (a saved rule) or rule (an unsaved one)".into(),
            ));
        }
    };
    let store = p.store.unwrap_or(LIVE_STORE);
    Ok(json!(ctx.engine.test_rule_in(store, &rule, p.flow_id, None).await?))
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
