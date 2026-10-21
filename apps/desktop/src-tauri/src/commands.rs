//! Tauri commands: argument plumbing only, everything delegates to `Engine`.

use std::sync::Arc;

use pm_core::Engine;
use pm_model::{
    BodyPart, CaInfo, FilterSpec, FlowDetail, FlowId, FlowSummary, ProxyStatus, RowsPage, SidebarSnapshot, SortSpec,
    SslSettings, SystemProxyStatus, TrustStatus, ViewDelta, ViewId, ViewInfo,
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
#[tauri::command]
#[specta::specta]
pub fn get_body(engine: EngineState<'_>, id: FlowId, part: BodyPart, decoded: bool) -> Result<RawBytes, String> {
    engine
        .body(id, part, decoded)
        .map(|body| RawBytes(tauri::ipc::Response::new(body.bytes)))
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
