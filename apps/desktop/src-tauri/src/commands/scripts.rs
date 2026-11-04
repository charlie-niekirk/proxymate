//! Scripts (M14).

use pm_model::{
    BodyPart, ConsoleEntry, FlowId, IpcError, IpcErrorKind, Script, ScriptId, ScriptSettings, ScriptTestResult,
    ScriptTestSpec, StoreId, ViewResult,
};
use tauri::ipc::Channel;

use super::dialogs::save_path;
use super::{EngineState, blocking, ipc};

/// Every script, grouped by kind (view, rewrite, mock), then by name.
#[tauri::command]
#[specta::specta]
pub fn list_scripts(engine: EngineState<'_>) -> Vec<Script> {
    engine.scripts()
}

/// Saves a new script (a fresh id is assigned). The error says what's wrong
/// with it (a syntax error, a missing entry point).
#[tauri::command]
#[specta::specta]
pub async fn create_script(engine: EngineState<'_>, script: Script) -> Result<Script, IpcError> {
    blocking(&engine, move |e| e.create_script(script)).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_script(engine: EngineState<'_>, script: Script) -> Result<Script, IpcError> {
    blocking(&engine, move |e| e.update_script(script)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_script_enabled(engine: EngineState<'_>, id: ScriptId, enabled: bool) -> Result<Script, IpcError> {
    blocking(&engine, move |e| e.set_script_enabled(&id, enabled)).await
}

/// Refused while a rule runs the script; the error names those rules.
#[tauri::command]
#[specta::specta]
pub async fn delete_script(engine: EngineState<'_>, id: ScriptId) -> Result<(), IpcError> {
    blocking(&engine, move |e| e.delete_script(&id)).await
}

/// Deletes every agent-created script no rule runs; returns how many.
#[tauri::command]
#[specta::specta]
pub async fn delete_agent_scripts(engine: EngineState<'_>) -> Result<u32, IpcError> {
    blocking(&engine, |e| e.delete_agent_scripts()).await
}

/// Dry run of a script (saved, or unsaved source) against a recorded flow.
#[tauri::command]
#[specta::specta]
pub async fn test_script(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
    spec: ScriptTestSpec,
) -> Result<ScriptTestResult, IpcError> {
    engine.test_script(store, flow, spec).await.map_err(ipc)
}

/// The view scripts' views of one body; empty when none applies.
#[tauri::command]
#[specta::specta]
pub async fn get_flow_views(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
    part: BodyPart,
) -> Result<Vec<ViewResult>, IpcError> {
    engine.run_view_scripts(store, flow, part).await.map_err(ipc)
}

/// Console entries newer than `since`, oldest first.
#[tauri::command]
#[specta::specta]
pub fn get_script_console(engine: EngineState<'_>, since: Option<u64>, limit: u32) -> Vec<ConsoleEntry> {
    engine.script_console(since, limit)
}

#[tauri::command]
#[specta::specta]
pub fn clear_script_console(engine: EngineState<'_>) {
    engine.clear_script_console();
}

/// Pushes the newest console sequence number whenever entries are added.
#[tauri::command]
#[specta::specta]
pub fn subscribe_script_console(engine: EngineState<'_>, on_entries: Channel<u64>) {
    engine.subscribe_script_console(Box::new(move |seq| on_entries.send(seq).is_ok()));
}

/// Streams the script list whenever anyone (the UI or an agent) changes it.
#[tauri::command]
#[specta::specta]
pub fn subscribe_scripts(engine: EngineState<'_>, on_scripts: Channel<Vec<Script>>) {
    engine.subscribe_scripts(Box::new(move |scripts| on_scripts.send(scripts).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn get_script_settings(engine: EngineState<'_>) -> ScriptSettings {
    engine.script_settings()
}

#[tauri::command]
#[specta::specta]
pub fn set_script_settings(engine: EngineState<'_>, settings: ScriptSettings) -> Result<ScriptSettings, IpcError> {
    engine.set_script_settings(settings).map_err(ipc)
}

/// `proxymate.d.ts`, for "Copy type definitions".
#[tauri::command]
#[specta::specta]
pub fn get_script_type_definitions(engine: EngineState<'_>) -> String {
    engine.script_type_definitions().to_owned()
}

/// Saves a script as a `.js` file with a metadata header; without `path`,
/// asks where. `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_script(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    id: ScriptId,
    path: Option<String>,
) -> Result<Option<String>, IpcError> {
    let text = engine.export_script(&id).map_err(ipc)?;
    let name = engine.script(&id).map(|s| s.name).unwrap_or_default();
    let file_name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let file_name = format!("{}.js", file_name.trim());
    let Some(path) = save_path(&app, path, &file_name, "JavaScript", "js").await? else {
        return Ok(None);
    };
    std::fs::write(&path, text).map_err(ipc)?;
    Ok(Some(path.display().to_string()))
}

/// Imports a `.js` file as a new script; without `path`, asks for one.
/// `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn import_script(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    path: Option<String>,
) -> Result<Option<Script>, IpcError> {
    use tauri_plugin_dialog::DialogExt;
    let path = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.dialog()
                .file()
                .add_filter("JavaScript", &["js", "mjs"])
                .pick_file(move |path| {
                    let _ = tx.send(path);
                });
            let Some(path) = rx.await.ok().flatten() else {
                return Ok(None);
            };
            path.into_path().map_err(IpcError::internal)?
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| IpcError::new(IpcErrorKind::Storage, format!("{}: {e}", path.display())))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    blocking(&engine, move |e| {
        e.import_script(&text, file_name.as_deref(), pm_model::RuleOrigin::User)
    })
    .await
    .map(Some)
}
