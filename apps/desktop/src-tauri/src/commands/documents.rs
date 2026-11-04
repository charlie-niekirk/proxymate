//! Replay, compose and cURL; sessions and HAR files (M8).

use std::sync::Arc;

use pm_model::{DocumentInfo, FlowId, FlowSelection, RequestSpec, SaveProgress, SendOpts, SendOrigin, StoreId};
use tauri::ipc::Channel;

use super::dialogs::save_path;
use super::{EngineState, run_blocking};

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
pub async fn get_request_spec(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Result<RequestSpec, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.request_spec(store, id, true).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_curl(engine: EngineState<'_>, store: StoreId, id: FlowId) -> Result<String, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.curl_command(store, id).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub fn list_documents(engine: EngineState<'_>) -> Vec<DocumentInfo> {
    engine.documents()
}

/// Opens a session, HAR or Charles file as a document; without `path`,
/// asks for one (only HAR files with `har`). `None` if the user cancelled.
/// A binary Charles session (`.chls`) can be picked, so its error says how
/// to export a readable one.
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
                dialog
                    .set_title("Open a session, a HAR file or a Charles export (JSON or XML)")
                    .add_filter(
                        "Proxymate session, HAR or Charles (.chlsj, .chlsx)",
                        &["proxymate", "har", "chlsj", "chlsx", "chls"],
                    )
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
pub async fn close_document(engine: EngineState<'_>, store: StoreId) -> Result<(), String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.close_document(store).map_err(|e| e.to_string())).await
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

pub(super) fn progress_sink(channel: Channel<SaveProgress>) -> pm_core::ProgressSink {
    Box::new(move |progress| {
        let _ = channel.send(progress);
    })
}
