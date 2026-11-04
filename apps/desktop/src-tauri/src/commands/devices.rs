//! Device setup: LAN access, the Simulator, Android (M9).

use std::sync::Arc;

use pm_model::{AndroidStatus, DeviceAccess, DevicesSnapshot, SimInstallResult, SimulatorStatus};
use tauri::ipc::Channel;

use super::EngineState;

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
