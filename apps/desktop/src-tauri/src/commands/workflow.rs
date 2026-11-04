//! Selections, labels, comments, header columns, Copy as code, Repeat (M17).

use std::sync::Arc;

use pm_model::{
    CodeLanguage, FlowId, FlowLabel, FlowSelection, HeaderColumn, RepeatId, RepeatOpts, RepeatProgress, StoreId,
};
use tauri::ipc::Channel;

use super::{EngineState, run_blocking};

/// Pins or unpins flows of any store; pinned live flows are never evicted.
#[tauri::command]
#[specta::specta]
pub async fn pin_flows(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    pinned: bool,
) -> Result<u32, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .pin_flows_in(store, &selection, pinned)
            .map_err(|e| e.to_string())
    })
    .await
}

/// The ids a selection names, in its order.
#[tauri::command]
#[specta::specta]
pub async fn selected_ids(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
) -> Result<Vec<FlowId>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.selected_ids(store, &selection).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_flow_urls(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
) -> Result<Vec<String>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.flow_urls(store, &selection).map_err(|e| e.to_string())).await
}

/// Deletes flows; returns how many went.
#[tauri::command]
#[specta::specta]
pub async fn remove_flows(engine: EngineState<'_>, store: StoreId, selection: FlowSelection) -> Result<u32, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.remove_flows_in(store, &selection).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn label_flows(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    label: Option<FlowLabel>,
) -> Result<u32, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .label_flows_in(store, &selection, label)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn comment_flow(engine: EngineState<'_>, store: StoreId, id: FlowId, comment: String) -> Result<(), String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.comment_flow_in(store, id, &comment).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_header_columns(
    engine: EngineState<'_>,
    columns: Vec<HeaderColumn>,
) -> Result<Vec<HeaderColumn>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.set_header_columns(columns).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_code(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    language: CodeLanguage,
) -> Result<String, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.code_snippet(store, id, language).map_err(|e| e.to_string())).await
}

/// Starts repeating a recorded request; progress streams to `on_progress`.
#[tauri::command]
#[specta::specta]
pub async fn start_repeat(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    opts: RepeatOpts,
    on_progress: Channel<RepeatProgress>,
) -> Result<RepeatId, String> {
    let spec = engine.request_spec(store, id, false).map_err(|e| e.to_string())?;
    engine
        .repeat(
            spec,
            opts,
            Box::new(move |progress| {
                let _ = on_progress.send(progress);
            }),
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn cancel_repeat(engine: EngineState<'_>, id: RepeatId) -> bool {
    engine.cancel_repeat(id)
}
