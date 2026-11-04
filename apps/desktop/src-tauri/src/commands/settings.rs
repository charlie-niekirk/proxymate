//! Storage, proxy, network and update settings (M10, M11, M18).

use pm_model::{NetworkSettings, ProxySettings, StorageSettings, StorageStatus, UpdateSettings};
use tauri::ipc::Channel;

use super::EngineState;

#[tauri::command]
#[specta::specta]
pub fn get_storage_settings(engine: EngineState<'_>) -> StorageSettings {
    engine.storage_settings()
}

/// Saves new limits; flows past them are evicted straight away.
#[tauri::command]
#[specta::specta]
pub fn set_storage_settings(engine: EngineState<'_>, settings: StorageSettings) -> Result<StorageSettings, String> {
    engine.set_storage_settings(settings).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_storage_status(engine: EngineState<'_>) -> StorageStatus {
    engine.storage_status()
}

/// Streams the storage status (flows, disk use, write errors) as it changes.
#[tauri::command]
#[specta::specta]
pub fn subscribe_storage(engine: EngineState<'_>, on_status: Channel<StorageStatus>) {
    engine.subscribe_storage(Box::new(move |status| on_status.send(status).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn get_proxy_settings(engine: EngineState<'_>) -> ProxySettings {
    engine.proxy_settings()
}

/// Saves the port and upstream pool settings, restarting the listener if
/// they changed. A port that can't be bound is an error; the old one stays.
#[tauri::command]
#[specta::specta]
pub async fn set_proxy_settings(engine: EngineState<'_>, settings: ProxySettings) -> Result<ProxySettings, String> {
    engine.set_proxy_settings(settings).await.map_err(|e| e.to_string())
}

/// Settings › Network (M18). Passwords never come back: `has_password`
/// says one is saved.
#[tauri::command]
#[specta::specta]
pub fn get_network_settings(engine: EngineState<'_>) -> NetworkSettings {
    engine.network_settings()
}

/// Saves and applies Settings › Network: passwords go to the keychain,
/// client certificates are loaded and reverse proxies restarted. A bad
/// setting is an error that says why; nothing is changed then.
#[tauri::command]
#[specta::specta]
pub async fn set_network_settings(
    engine: EngineState<'_>,
    settings: NetworkSettings,
) -> Result<NetworkSettings, String> {
    engine.set_network_settings(settings).await.map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_update_settings(engine: EngineState<'_>) -> UpdateSettings {
    engine.update_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_update_settings(engine: EngineState<'_>, settings: UpdateSettings) -> Result<UpdateSettings, String> {
    engine.set_update_settings(settings).map_err(|e| e.to_string())
}

/// Settings › Updates' "Check Now": the same check as the menu item, which
/// answers with a native dialog.
#[tauri::command]
#[specta::specta]
pub fn check_for_updates(app: tauri::AppHandle) {
    crate::updater::check_now(&app);
}
