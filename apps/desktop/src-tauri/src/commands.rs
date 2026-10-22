//! Tauri commands: argument plumbing only, everything delegates to `Engine`.

use std::sync::Arc;

use pm_core::Engine;
use pm_model::{
    AndroidStatus, BodyPart, BreakpointId, BreakpointInfo, BreakpointResolution, CaInfo, DeviceAccess, DevicesSnapshot,
    DocumentInfo, FilterSpec, FlowDetail, FlowId, FlowSelection, FlowSummary, McpAuditPage, McpSettings, McpStatus,
    PresetKind, ProxyStatus, RequestSpec, RowsPage, Rule, RuleId, RuleSettings, RuleTestResult, SaveProgress, SendOpts,
    SendOrigin, SidebarSnapshot, SimInstallResult, SimulatorStatus, SortSpec, SsePage, SslSettings, StoreId,
    SystemProxyStatus, TrustStatus, ViewDelta, ViewId, ViewInfo, WsPage, WsQuery,
};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

type EngineState<'a> = State<'a, Arc<Engine>>;

#[tauri::command]
#[specta::specta]
pub fn create_view(
    engine: EngineState<'_>,
    store: StoreId,
    filter: FilterSpec,
    sort: SortSpec,
) -> Result<ViewInfo, String> {
    engine.create_view_in(store, filter, sort).map_err(|e| e.to_string())
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
pub fn get_flow(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Option<FlowSummary> {
    engine.get_flow_in(store, id)
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
pub fn get_flow_detail(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Result<Option<FlowDetail>, String> {
    engine.flow_detail_in(store, id).map_err(|e| e.to_string())
}

/// A body's bytes as a raw `ArrayBuffer`, not JSON. With `decoded`, a
/// content-encoded body is decoded first; decoding errors are returned.
/// With `max`, only the first `max` bytes are sent.
#[tauri::command]
#[specta::specta]
pub fn get_body(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    part: BodyPart,
    decoded: bool,
    max: Option<u64>,
) -> Result<RawBytes, String> {
    let max = max.map(|m| usize::try_from(m).unwrap_or(usize::MAX));
    engine
        .body_head_in(store, id, part, decoded, max)
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
    store: StoreId,
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
    engine
        .save_body_in(store, id, part, decoded, &path)
        .map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
#[specta::specta]
pub fn get_websocket_messages(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    query: WsQuery,
    start: u32,
    count: u32,
) -> Result<WsPage, String> {
    engine
        .websocket_messages_in(store, id, &query, start, count)
        .map_err(|e| e.to_string())
}

/// One WebSocket message's payload as a raw `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub fn get_websocket_payload(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    index: u32,
) -> Result<RawBytes, String> {
    engine
        .websocket_payload_in(store, id, index)
        .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_sse_events(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    text: String,
    start: u32,
    count: u32,
) -> Result<SsePage, String> {
    engine
        .sse_events_in(store, id, &text, start, count)
        .map_err(|e| e.to_string())
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

// ---- Device setup ----

/// LAN interfaces, the chosen one, the setup URL and its QR code.
#[tauri::command]
#[specta::specta]
pub async fn get_device_access(engine: EngineState<'_>) -> Result<DeviceAccess, String> {
    let engine = Arc::clone(&engine);
    tauri::async_runtime::spawn_blocking(move || engine.device_access())
        .await
        .map_err(|e| e.to_string())
}

/// Listens on every interface (or loopback only). Failures come back in
/// `error`.
#[tauri::command]
#[specta::specta]
pub async fn set_lan_access(engine: EngineState<'_>, on: bool) -> Result<DeviceAccess, String> {
    Ok(engine.set_lan_access(on).await)
}

#[tauri::command]
#[specta::specta]
pub fn set_lan_interface(engine: EngineState<'_>, name: String) -> Result<DeviceAccess, String> {
    engine.set_lan_interface(&name).map_err(|e| e.to_string())
}

/// Streams LAN clients (pending prompts first) and device activity.
#[tauri::command]
#[specta::specta]
pub fn subscribe_devices(engine: EngineState<'_>, on_snapshot: Channel<DevicesSnapshot>) {
    engine.subscribe_devices(Box::new(move |snapshot| on_snapshot.send(snapshot).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn decide_device(
    engine: EngineState<'_>,
    ip: String,
    allow: bool,
    nickname: Option<String>,
) -> Result<DevicesSnapshot, String> {
    engine.decide_device(&ip, allow, nickname).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn forget_device(engine: EngineState<'_>, ip: String) -> Result<DevicesSnapshot, String> {
    engine.forget_device(&ip).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_simulator_status(engine: EngineState<'_>) -> Result<SimulatorStatus, String> {
    Ok(engine.simulator_status().await)
}

/// Adds the CA to each booted simulator in `udids` (all booted if empty).
#[tauri::command]
#[specta::specta]
pub async fn install_simulator_cert(
    engine: EngineState<'_>,
    udids: Vec<String>,
) -> Result<Vec<SimInstallResult>, String> {
    engine.install_simulator_cert(udids).await.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_android_status(engine: EngineState<'_>) -> Result<AndroidStatus, String> {
    Ok(engine.android_status().await)
}

/// Points an emulator's proxy at us, or clears it.
#[tauri::command]
#[specta::specta]
pub async fn set_android_proxy(engine: EngineState<'_>, serial: String, on: bool) -> Result<AndroidStatus, String> {
    engine.set_android_proxy(&serial, on).await.map_err(|e| e.to_string())
}

/// Copies the CA to the device's Downloads; returns the path on the device.
#[tauri::command]
#[specta::specta]
pub async fn push_android_cert(engine: EngineState<'_>, serial: String) -> Result<String, String> {
    engine.push_android_cert(&serial).await.map_err(|e| e.to_string())
}

// ---- Replay, compose, cURL (M8) ----

/// Sends a composed (or edited) request from the app; resolves to the new
/// flow's id once it is recorded.
#[tauri::command]
#[specta::specta]
pub async fn send_request(engine: EngineState<'_>, spec: RequestSpec, apply_rules: bool) -> Result<FlowId, String> {
    let opts = SendOpts {
        apply_rules,
        wait: false,
        origin: SendOrigin::User,
    };
    engine.send(spec, opts).await.map_err(|e| e.to_string())
}

/// Sends a recorded request again, as it was sent.
#[tauri::command]
#[specta::specta]
pub async fn replay_flow(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    apply_rules: bool,
) -> Result<FlowId, String> {
    let opts = SendOpts {
        apply_rules,
        wait: false,
        origin: SendOrigin::User,
    };
    engine.replay(store, id, opts).await.map_err(|e| e.to_string())
}

/// A recorded request, editable (body decoded), for Edit & Replay.
#[tauri::command]
#[specta::specta]
pub fn get_request_spec(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Result<RequestSpec, String> {
    engine.request_spec(store, id, true).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_curl(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Result<String, String> {
    engine.curl_command(store, id).map_err(|e| e.to_string())
}

// ---- Documents: sessions and HAR (M8) ----

#[tauri::command]
#[specta::specta]
pub fn list_documents(engine: EngineState<'_>) -> Vec<DocumentInfo> {
    engine.documents()
}

/// Opens a session or HAR file as a document; without `path`, asks for
/// one (only HAR files with `har`). `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn open_document(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    path: Option<String>,
    har: bool,
) -> Result<Option<DocumentInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let dialog = app.dialog().file();
            let dialog = if har {
                dialog.add_filter("HAR", &["har"])
            } else {
                dialog.add_filter("Proxymate session or HAR", &["proxymate", "har"])
            };
            dialog.pick_file(move |path| {
                let _ = tx.send(path);
            });
            let Some(path) = rx.await.ok().flatten() else {
                return Ok(None);
            };
            path.into_path().map_err(|e| e.to_string())?
        }
    };
    let engine = engine.inner().clone();
    // Loading a big session or HAR takes a moment; keep it off the IPC thread.
    tokio::task::spawn_blocking(move || engine.open_document(&path))
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn close_document(engine: EngineState<'_>, store: StoreId) -> Result<(), String> {
    engine.close_document(store).map_err(|e| e.to_string())
}

/// Saves flows of a store as a `.proxymate` session in the background;
/// without `path`, asks where. Resolves to the path (`None` if cancelled)
/// once the save has started; `on_progress` hears the rest.
#[tauri::command]
#[specta::specta]
pub async fn save_session(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    path: Option<String>,
    file_name: String,
    on_progress: Channel<SaveProgress>,
) -> Result<Option<String>, String> {
    let Some(path) = save_path(&app, path, &file_name, "Proxymate session", "proxymate").await? else {
        return Ok(None);
    };
    engine
        .save_session(store, &selection, path.clone(), progress_sink(on_progress))
        .map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// Exports flows of a store as a HAR file in the background; like
/// [`save_session`].
#[tauri::command]
#[specta::specta]
pub async fn export_har(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    path: Option<String>,
    file_name: String,
    on_progress: Channel<SaveProgress>,
) -> Result<Option<String>, String> {
    let Some(path) = save_path(&app, path, &file_name, "HAR", "har").await? else {
        return Ok(None);
    };
    engine
        .export_har(store, &selection, path.clone(), progress_sink(on_progress))
        .map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

async fn save_path(
    app: &tauri::AppHandle,
    path: Option<String>,
    file_name: &str,
    kind: &str,
    extension: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    if let Some(path) = path {
        return Ok(Some(path.into()));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(kind, &[extension])
        .set_file_name(file_name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    match rx.await.ok().flatten() {
        Some(path) => path.into_path().map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

fn progress_sink(channel: Channel<SaveProgress>) -> pm_core::ProgressSink {
    Box::new(move |progress| {
        let _ = channel.send(progress);
    })
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

/// Streams the rule list whenever anyone (the UI or an agent) changes it.
#[tauri::command]
#[specta::specta]
pub fn subscribe_rules(engine: EngineState<'_>, on_rules: Channel<Vec<Rule>>) {
    engine.subscribe_rules(Box::new(move |rules| on_rules.send(rules).is_ok()));
}

// ---- MCP (M7) ----

#[tauri::command]
#[specta::specta]
pub fn get_mcp_status(engine: EngineState<'_>) -> McpStatus {
    engine.mcp_status()
}

/// Streams the MCP status (connected clients, settings, errors).
#[tauri::command]
#[specta::specta]
pub fn subscribe_mcp(engine: EngineState<'_>, on_status: Channel<McpStatus>) {
    engine.subscribe_mcp(Box::new(move |status| on_status.send(status).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn set_mcp_settings(engine: EngineState<'_>, settings: McpSettings) -> Result<McpSettings, String> {
    engine.set_mcp_settings(settings).map_err(|e| e.to_string())
}

/// The bearer token, for the copy buttons.
#[tauri::command]
#[specta::specta]
pub fn get_mcp_token(engine: EngineState<'_>) -> String {
    engine.mcp_token()
}

#[tauri::command]
#[specta::specta]
pub fn regenerate_mcp_token(engine: EngineState<'_>) -> Result<String, String> {
    engine.regenerate_mcp_token().map_err(|e| e.to_string())
}

/// Audit entries older than `before` (all, without), newest first.
#[tauri::command]
#[specta::specta]
pub fn get_mcp_audit(engine: EngineState<'_>, before: Option<i64>, limit: u32) -> Result<McpAuditPage, String> {
    engine.mcp_audit(before, limit).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn clear_mcp_audit(engine: EngineState<'_>) -> Result<(), String> {
    engine.clear_mcp_audit().map_err(|e| e.to_string())
}
