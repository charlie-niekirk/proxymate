//! Capture targets: intercepted launches, recording lists, auto-save (M21).

use pm_model::{AutoSaveSettings, InterceptApp, InterceptSession, InterceptTarget, IpcError, RecordingSettings};

use super::{EngineState, blocking, ipc};

/// The Intercept menu: terminals and browsers, and which are installed.
#[tauri::command]
#[specta::specta]
pub fn list_intercept_apps(engine: EngineState<'_>) -> Vec<InterceptApp> {
    engine.intercept_apps()
}

/// Intercepted launches still running.
#[tauri::command]
#[specta::specta]
pub fn list_intercepts(engine: EngineState<'_>) -> Vec<InterceptSession> {
    engine.intercepts()
}

/// Starts a terminal or browser set up for the proxy.
#[tauri::command]
#[specta::specta]
pub async fn intercept(engine: EngineState<'_>, target: InterceptTarget) -> Result<InterceptSession, IpcError> {
    blocking(&engine, move |engine| engine.intercept(target)).await
}

#[tauri::command]
#[specta::specta]
pub fn get_recording_settings(engine: EngineState<'_>) -> RecordingSettings {
    engine.recording_settings()
}

/// Saves the recording lists; they apply to flows that begin from now on.
#[tauri::command]
#[specta::specta]
pub fn set_recording_settings(
    engine: EngineState<'_>,
    settings: RecordingSettings,
) -> Result<RecordingSettings, IpcError> {
    engine.set_recording_settings(settings).map_err(ipc)
}

/// The flow table's "Never record this host".
#[tauri::command]
#[specta::specta]
pub fn never_record_host(engine: EngineState<'_>, host: String) -> Result<RecordingSettings, IpcError> {
    engine.never_record(&host).map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn get_autosave_settings(engine: EngineState<'_>) -> AutoSaveSettings {
    engine.autosave_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_autosave_settings(
    engine: EngineState<'_>,
    settings: AutoSaveSettings,
) -> Result<AutoSaveSettings, IpcError> {
    engine.set_autosave_settings(settings).map_err(ipc)
}

/// Auto-saves now, whatever the interval; progress shows in the storage status.
#[tauri::command]
#[specta::specta]
pub async fn autosave_now(engine: EngineState<'_>) -> Result<u32, IpcError> {
    blocking(&engine, move |engine| engine.autosave_now()).await
}
