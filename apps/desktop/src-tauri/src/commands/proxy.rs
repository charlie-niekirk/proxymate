//! The proxy itself: recording, the CA, SSL lists, trust, the system proxy.

use pm_model::{CaInfo, IpcError, ProxyStatus, SidebarSnapshot, SslSettings, StoreId, SystemProxyStatus, TrustStatus};
use tauri::ipc::Channel;

use super::{EngineState, blocking, blocking_or_default, engine_of, ipc};

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
pub async fn regenerate_ca(engine: EngineState<'_>, remove_old_trust: bool) -> Result<CaInfo, IpcError> {
    engine.regenerate_ca_and_untrust(remove_old_trust).await.map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn get_ssl_settings(engine: EngineState<'_>) -> SslSettings {
    engine.ssl_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_ssl_settings(engine: EngineState<'_>, settings: SslSettings) -> Result<SslSettings, IpcError> {
    engine.set_ssl_settings(settings).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn add_ssl_bypass(engine: EngineState<'_>, host: String) -> Result<SslSettings, IpcError> {
    engine.add_ssl_bypass(&host).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn set_insecure_upstream(engine: EngineState<'_>, host: String, allow: bool) -> Result<SslSettings, IpcError> {
    engine.set_insecure_upstream(&host, allow).map_err(ipc)
}

/// Streams the sidebar's Apps and Domains whenever they change.
#[tauri::command]
#[specta::specta]
pub fn subscribe_sidebar(engine: EngineState<'_>, on_snapshot: Channel<SidebarSnapshot>) {
    engine.subscribe_sidebar(Box::new(move |snapshot| on_snapshot.send(snapshot).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn set_app_ssl_excluded(engine: EngineState<'_>, app: String, excluded: bool) -> Result<SslSettings, IpcError> {
    engine.set_app_ssl_excluded(&app, excluded).map_err(ipc)
}

/// Keychain trust plus a real handshake check through the proxy.
#[tauri::command]
#[specta::specta]
pub async fn get_trust_status(engine: EngineState<'_>) -> Result<TrustStatus, IpcError> {
    Ok(engine.trust_status().await)
}

/// Installs and trusts the CA; resolves once the macOS prompt is answered.
#[tauri::command]
#[specta::specta]
pub async fn install_ca_trust(engine: EngineState<'_>) -> Result<TrustStatus, IpcError> {
    engine.install_ca_trust().await.map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_ca_trust(engine: EngineState<'_>) -> Result<TrustStatus, IpcError> {
    engine.uninstall_ca_trust().await.map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub async fn get_system_proxy_status(engine: EngineState<'_>) -> Result<SystemProxyStatus, IpcError> {
    blocking(&engine, move |engine| Ok::<_, IpcError>(engine.system_proxy_status())).await
}

/// Failures are reported in the returned status, not as an error.
#[tauri::command]
#[specta::specta]
pub async fn set_system_proxy(engine: EngineState<'_>, on: bool) -> Result<SystemProxyStatus, IpcError> {
    blocking(&engine, move |engine| Ok::<_, IpcError>(engine.set_system_proxy(on))).await
}

/// `export` lines for a proxied terminal.
#[tauri::command]
#[specta::specta]
pub fn get_terminal_env(engine: EngineState<'_>) -> String {
    engine.terminal_env()
}

/// Makes the sidebar's Apps and Domains count the tab the UI shows.
#[tauri::command]
#[specta::specta]
pub async fn set_sidebar_store(app: tauri::AppHandle, store: StoreId) {
    blocking_or_default(&engine_of(&app), move |engine| {
        engine.set_sidebar_store(store);
    })
    .await
}
