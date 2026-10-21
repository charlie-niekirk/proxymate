//! Tauri commands: argument plumbing only, everything delegates to `Engine`.

use std::sync::Arc;

use pm_core::Engine;
use pm_model::{
    BodyPart, BreakpointId, BreakpointInfo, BreakpointResolution, CaInfo, FilterSpec, FlowDetail, FlowId, FlowSummary,
    PresetKind, ProxyStatus, RowsPage, Rule, RuleId, RuleSettings, RuleTestResult, SidebarSnapshot, SortSpec, SsePage,
    SslSettings, SystemProxyStatus, TrustStatus, ViewDelta, ViewId, ViewInfo, WsPage, WsQuery,
};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

type EngineState<'a> = State<'a, Arc<Engine>>;

#[tauri::command]
#[specta::specta]
pub fn create_view(engine: EngineState<'_>, filter: FilterSpec, sort: SortSpec) -> ViewInfo {
    engine.create_view(filter, sort)
}

#[tauri::command]
#[specta::specta]
pub fn set_filter(engine: EngineState<'_>, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, String> {
    engine.set_filter(view, filter).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_sort(engine: EngineState<'_>, view: ViewId, sort: SortSpec) -> Result<ViewInfo, String> {
    engine.set_sort(view, sort).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn drop_view(engine: EngineState<'_>, view: ViewId) {
    engine.drop_view(view);
}

#[tauri::command]
#[specta::specta]
pub fn get_rows(engine: EngineState<'_>, view: ViewId, start: u32, count: u32) -> Result<RowsPage, String> {
    engine.get_rows(view, start, count).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_flow(engine: EngineState<'_>, id: FlowId) -> Option<FlowSummary> {
    engine.get_flow(id)
}

/// Streams the view's 20 Hz deltas to the webview until the channel closes.
#[tauri::command]
#[specta::specta]
pub fn subscribe_view(engine: EngineState<'_>, view: ViewId, on_delta: Channel<ViewDelta>) -> Result<(), String> {
    engine
        .subscribe(view, Box::new(move |delta| on_delta.send(delta).is_ok()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_flow_detail(engine: EngineState<'_>, id: FlowId) -> Result<Option<FlowDetail>, String> {
    engine.flow_detail(id).map_err(|e| e.to_string())
}

/// A body's bytes as a raw `ArrayBuffer`, not JSON. With `decoded`, a
/// content-encoded body is decoded first; decoding errors are returned.
/// With `max`, only the first `max` bytes are sent.
#[tauri::command]
#[specta::specta]
pub fn get_body(
    engine: EngineState<'_>,
    id: FlowId,
    part: BodyPart,
    decoded: bool,
    max: Option<u64>,
) -> Result<RawBytes, String> {
    let max = max.map(|m| usize::try_from(m).unwrap_or(usize::MAX));
    engine
        .body_head(id, part, decoded, max)
        .map(|body| RawBytes(tauri::ipc::Response::new(body.bytes)))
        .map_err(|e| e.to_string())
}

/// Asks where to save a body (decoded, with `decoded`) and writes it there.
/// Returns the path, or `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn save_body(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    id: FlowId,
    part: BodyPart,
    decoded: bool,
    file_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(&file_name).save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    engine.save_body(id, part, decoded, &path).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
#[specta::specta]
pub fn get_websocket_messages(
    engine: EngineState<'_>,
    id: FlowId,
    query: WsQuery,
    start: u32,
    count: u32,
) -> Result<WsPage, String> {
    engine
        .websocket_messages(id, &query, start, count)
        .map_err(|e| e.to_string())
}

/// One WebSocket message's payload as a raw `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub fn get_websocket_payload(engine: EngineState<'_>, id: FlowId, index: u32) -> Result<RawBytes, String> {
    engine
        .websocket_payload(id, index)
        .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_sse_events(
    engine: EngineState<'_>,
    id: FlowId,
    text: String,
    start: u32,
    count: u32,
) -> Result<SsePage, String> {
    engine.sse_events(id, &text, start, count).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn clear_flows(engine: EngineState<'_>) {
    engine.clear();
}

#[tauri::command]
#[specta::specta]
pub fn set_recording(engine: EngineState<'_>, on: bool) -> ProxyStatus {
    engine.set_recording(on);
    engine.proxy_status()
}

#[tauri::command]
#[specta::specta]
pub fn get_proxy_status(engine: EngineState<'_>) -> ProxyStatus {
    engine.proxy_status()
}

#[tauri::command]
#[specta::specta]
pub fn get_ca_info(engine: EngineState<'_>) -> CaInfo {
    engine.ca_info()
}

#[tauri::command]
#[specta::specta]
pub fn regenerate_ca(engine: EngineState<'_>) -> Result<CaInfo, String> {
    engine.regenerate_ca().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_ssl_settings(engine: EngineState<'_>) -> SslSettings {
    engine.ssl_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_ssl_settings(engine: EngineState<'_>, settings: SslSettings) -> Result<SslSettings, String> {
    engine.set_ssl_settings(settings).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn add_ssl_bypass(engine: EngineState<'_>, host: String) -> Result<SslSettings, String> {
    engine.add_ssl_bypass(&host).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_insecure_upstream(engine: EngineState<'_>, host: String, allow: bool) -> Result<SslSettings, String> {
    engine.set_insecure_upstream(&host, allow).map_err(|e| e.to_string())
}

/// Streams the sidebar's Apps and Domains whenever they change.
#[tauri::command]
#[specta::specta]
pub fn subscribe_sidebar(engine: EngineState<'_>, on_snapshot: Channel<SidebarSnapshot>) {
    engine.subscribe_sidebar(Box::new(move |snapshot| on_snapshot.send(snapshot).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn set_app_ssl_excluded(engine: EngineState<'_>, app: String, excluded: bool) -> Result<SslSettings, String> {
    engine.set_app_ssl_excluded(&app, excluded).map_err(|e| e.to_string())
}

/// Keychain trust plus a real handshake check through the proxy.
#[tauri::command]
#[specta::specta]
pub async fn get_trust_status(engine: EngineState<'_>) -> Result<TrustStatus, String> {
    Ok(engine.trust_status().await)
}

/// Installs and trusts the CA; resolves once the macOS prompt is answered.
#[tauri::command]
#[specta::specta]
pub async fn install_ca_trust(engine: EngineState<'_>) -> Result<TrustStatus, String> {
    engine.install_ca_trust().await.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_ca_trust(engine: EngineState<'_>) -> Result<TrustStatus, String> {
    engine.uninstall_ca_trust().await.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_system_proxy_status(engine: EngineState<'_>) -> Result<SystemProxyStatus, String> {
    let engine = Arc::clone(&engine);
    tauri::async_runtime::spawn_blocking(move || engine.system_proxy_status())
        .await
        .map_err(|e| e.to_string())
}

/// Failures are reported in the returned status, not as an error.
#[tauri::command]
#[specta::specta]
pub async fn set_system_proxy(engine: EngineState<'_>, on: bool) -> Result<SystemProxyStatus, String> {
    let engine = Arc::clone(&engine);
    tauri::async_runtime::spawn_blocking(move || engine.set_system_proxy(on))
        .await
        .map_err(|e| e.to_string())
}

/// `export` lines for a proxied terminal.
#[tauri::command]
#[specta::specta]
pub fn get_terminal_env(engine: EngineState<'_>) -> String {
    engine.terminal_env()
}

// ---- Rules ----

/// Every rule, in evaluation order.
#[tauri::command]
#[specta::specta]
pub fn list_rules(engine: EngineState<'_>) -> Vec<Rule> {
    engine.rules()
}

/// Saves a new rule at the end of the list (a fresh id is assigned).
#[tauri::command]
#[specta::specta]
pub fn create_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, String> {
    engine.create_rule(rule).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn update_rule(engine: EngineState<'_>, rule: Rule) -> Result<Rule, String> {
    engine.update_rule(rule).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_rule_enabled(engine: EngineState<'_>, id: RuleId, enabled: bool) -> Result<Rule, String> {
    engine.set_rule_enabled(&id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn delete_rule(engine: EngineState<'_>, id: RuleId) -> Result<(), String> {
    engine.delete_rule(&id).map_err(|e| e.to_string())
}

/// Deletes every agent-created rule; returns how many.
#[tauri::command]
#[specta::specta]
pub fn delete_agent_rules(engine: EngineState<'_>) -> Result<u32, String> {
    engine.delete_agent_rules().map_err(|e| e.to_string())
}

/// Puts the listed rules first, in that order.
#[tauri::command]
#[specta::specta]
pub fn reorder_rules(engine: EngineState<'_>, ids: Vec<RuleId>) -> Result<Vec<Rule>, String> {
    engine.reorder_rules(&ids).map_err(|e| e.to_string())
}

/// Compiles a rule without saving it; the error says what's wrong.
#[tauri::command]
#[specta::specta]
pub fn validate_rule(engine: EngineState<'_>, rule: Rule) -> Result<(), String> {
    engine.validate_rule(&rule).map_err(|e| e.to_string())
}

/// An unsaved rule pre-filled from a flow by a preset.
#[tauri::command]
#[specta::specta]
pub fn draft_rule(engine: EngineState<'_>, flow: FlowId, preset: PresetKind) -> Result<Rule, String> {
    engine.draft_rule(flow, preset).map_err(|e| e.to_string())
}

/// Dry run of a rule against a recorded flow.
#[tauri::command]
#[specta::specta]
pub async fn test_rule(engine: EngineState<'_>, rule: Rule, flow: FlowId) -> Result<RuleTestResult, String> {
    engine.test_rule(&rule, flow).await.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_rule_settings(engine: EngineState<'_>) -> RuleSettings {
    engine.rule_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_rule_settings(engine: EngineState<'_>, settings: RuleSettings) -> Result<RuleSettings, String> {
    engine.set_rule_settings(settings).map_err(|e| e.to_string())
}

// ---- Breakpoints ----

#[tauri::command]
#[specta::specta]
pub fn list_breakpoints(engine: EngineState<'_>) -> Vec<BreakpointInfo> {
    engine.breakpoints()
}

/// Streams the breakpoint queue whenever it changes.
#[tauri::command]
#[specta::specta]
pub fn subscribe_breakpoints(engine: EngineState<'_>, on_queue: Channel<Vec<BreakpointInfo>>) {
    engine.subscribe_breakpoints(Box::new(move |queue| on_queue.send(queue).is_ok()));
}

/// A paused message's (decoded) body as a raw `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub fn get_breakpoint_body(engine: EngineState<'_>, id: BreakpointId) -> Result<RawBytes, String> {
    engine
        .breakpoint_body(id)
        .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
        .map_err(|e| e.to_string())
}

/// Continues, aborts or answers a paused flow. The first resolution wins.
#[tauri::command]
#[specta::specta]
pub fn resolve_breakpoint(
    engine: EngineState<'_>,
    id: BreakpointId,
    resolution: BreakpointResolution,
) -> Result<(), String> {
    engine.resolve_breakpoint(id, resolution).map_err(|e| e.to_string())
}

/// Bytes sent to the webview as an `ArrayBuffer`. specta has no type for
/// `tauri::ipc::Response`, so the binding says `unknown` and `lib/backend.ts`
/// narrows it.
pub struct RawBytes(tauri::ipc::Response);

impl IpcResponse for RawBytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        self.0.body()
    }
}

impl specta::Type for RawBytes {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <specta_typescript::Unknown as specta::Type>::definition(types)
    }
}
