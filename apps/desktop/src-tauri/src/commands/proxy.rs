//! The proxy itself: recording, the CA, SSL lists, trust, the system proxy.

use std::sync::Arc;

use pm_model::{CaInfo, ProxyStatus, SidebarSnapshot, SslSettings, StoreId, SystemProxyStatus, TrustStatus};
use tauri::ipc::Channel;

use super::{EngineState, engine_of, run_blocking_or_default};

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
pub async fn regenerate_ca(engine: EngineState<'_>, remove_old_trust: bool) -> Result<CaInfo, String> {
    engine
        .regenerate_ca_and_untrust(remove_old_trust)
        .await
        .map_err(|e| e.to_string())
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

/// Makes the sidebar's Apps and Domains count the tab the UI shows.
#[tauri::command]
#[specta::specta]
pub async fn set_sidebar_store(app: tauri::AppHandle, store: StoreId) {
    let engine = engine_of(&app);
    run_blocking_or_default(move || {
        engine.set_sidebar_store(store);
    })
    .await
}
