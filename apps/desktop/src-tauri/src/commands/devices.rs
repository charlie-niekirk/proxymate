//! Device setup: LAN access, the Simulator, Android (M9).

use pm_model::{AndroidStatus, DeviceAccess, DevicesSnapshot, IpcError, SimInstallResult, SimulatorStatus};
use tauri::ipc::Channel;

use super::{EngineState, blocking, ipc};

/// LAN interfaces, the chosen one, the setup URL and its QR code.
#[tauri::command]
#[specta::specta]
pub async fn get_device_access(engine: EngineState<'_>) -> Result<DeviceAccess, IpcError> {
    blocking(&engine, move |engine| Ok::<_, IpcError>(engine.device_access())).await
}

/// Listens on every interface (or loopback only). Failures come back in
/// `error`.
#[tauri::command]
#[specta::specta]
pub async fn set_lan_access(engine: EngineState<'_>, on: bool) -> Result<DeviceAccess, IpcError> {
    Ok(engine.set_lan_access(on).await)
}

#[tauri::command]
#[specta::specta]
pub fn set_lan_interface(engine: EngineState<'_>, name: String) -> Result<DeviceAccess, IpcError> {
    engine.set_lan_interface(&name).map_err(ipc)
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
) -> Result<DevicesSnapshot, IpcError> {
    engine.decide_device(&ip, allow, nickname).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn forget_device(engine: EngineState<'_>, ip: String) -> Result<DevicesSnapshot, IpcError> {
    engine.forget_device(&ip).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub async fn get_simulator_status(engine: EngineState<'_>) -> Result<SimulatorStatus, IpcError> {
    Ok(engine.simulator_status().await)
}

/// Adds the CA to each booted simulator in `udids` (all booted if empty).
#[tauri::command]
#[specta::specta]
pub async fn install_simulator_cert(
    engine: EngineState<'_>,
    udids: Vec<String>,
) -> Result<Vec<SimInstallResult>, IpcError> {
    engine.install_simulator_cert(udids).await.map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub async fn get_android_status(engine: EngineState<'_>) -> Result<AndroidStatus, IpcError> {
    Ok(engine.android_status().await)
}

/// Points an emulator's proxy at us, or clears it.
#[tauri::command]
#[specta::specta]
pub async fn set_android_proxy(engine: EngineState<'_>, serial: String, on: bool) -> Result<AndroidStatus, IpcError> {
    engine.set_android_proxy(&serial, on).await.map_err(ipc)
}

/// Copies the CA to the device's Downloads; returns the path on the device.
#[tauri::command]
#[specta::specta]
pub async fn push_android_cert(engine: EngineState<'_>, serial: String) -> Result<String, IpcError> {
    engine.push_android_cert(&serial).await.map_err(ipc)
}
