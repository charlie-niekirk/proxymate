//! Capture targets: intercepted launches, recording lists, auto-save (M21).

use std::sync::Arc;

use pm_model::{AutoSaveSettings, InterceptApp, InterceptSession, InterceptTarget, RecordingSettings};

use super::{EngineState, run_blocking};

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
pub async fn intercept(engine: EngineState<'_>, target: InterceptTarget) -> Result<InterceptSession, String> {
    let engine = engine.inner().clone();
    tokio::task::spawn_blocking(move || engine.intercept(target))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
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
) -> Result<RecordingSettings, String> {
    engine.set_recording_settings(settings).map_err(|e| e.to_string())
}

/// The flow table's "Never record this host".
#[tauri::command]
#[specta::specta]
pub fn never_record_host(engine: EngineState<'_>, host: String) -> Result<RecordingSettings, String> {
    engine.never_record(&host).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_autosave_settings(engine: EngineState<'_>) -> AutoSaveSettings {
    engine.autosave_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_autosave_settings(engine: EngineState<'_>, settings: AutoSaveSettings) -> Result<AutoSaveSettings, String> {
    engine.set_autosave_settings(settings).map_err(|e| e.to_string())
}

/// Auto-saves now, whatever the interval; progress shows in the storage status.
#[tauri::command]
#[specta::specta]
pub async fn autosave_now(engine: EngineState<'_>) -> Result<u32, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.autosave_now().map_err(|e| e.to_string())).await
}
