//! Selections, labels, comments, header columns, Copy as code, Repeat (M17).

use pm_model::{
    CodeLanguage, FlowId, FlowLabel, FlowSelection, HeaderColumn, IpcError, RepeatId, RepeatOpts, RepeatProgress,
    StoreId,
};
use tauri::ipc::Channel;

use super::{EngineState, blocking, ipc};

/// Pins or unpins flows of any store; pinned live flows are never evicted.
#[tauri::command]
#[specta::specta]
pub async fn pin_flows(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    pinned: bool,
) -> Result<u32, IpcError> {
    blocking(&engine, move |engine| engine.pin_flows_in(store, &selection, pinned)).await
}

/// The ids a selection names, in its order.
#[tauri::command]
#[specta::specta]
pub async fn selected_ids(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
) -> Result<Vec<FlowId>, IpcError> {
    blocking(&engine, move |engine| engine.selected_ids(store, &selection)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_flow_urls(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
) -> Result<Vec<String>, IpcError> {
    blocking(&engine, move |engine| engine.flow_urls(store, &selection)).await
}

/// Deletes flows; returns how many went.
#[tauri::command]
#[specta::specta]
pub async fn remove_flows(engine: EngineState<'_>, store: StoreId, selection: FlowSelection) -> Result<u32, IpcError> {
    blocking(&engine, move |engine| engine.remove_flows_in(store, &selection)).await
}

#[tauri::command]
#[specta::specta]
pub async fn label_flows(
    engine: EngineState<'_>,
    store: StoreId,
    selection: FlowSelection,
    label: Option<FlowLabel>,
) -> Result<u32, IpcError> {
    blocking(&engine, move |engine| engine.label_flows_in(store, &selection, label)).await
}

#[tauri::command]
#[specta::specta]
pub async fn comment_flow(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    comment: String,
) -> Result<(), IpcError> {
    blocking(&engine, move |engine| engine.comment_flow_in(store, id, &comment)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_header_columns(
    engine: EngineState<'_>,
    columns: Vec<HeaderColumn>,
) -> Result<Vec<HeaderColumn>, IpcError> {
    blocking(&engine, move |engine| engine.set_header_columns(columns)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_code(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    language: CodeLanguage,
) -> Result<String, IpcError> {
    blocking(&engine, move |engine| engine.code_snippet(store, id, language)).await
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
) -> Result<RepeatId, IpcError> {
    let spec = engine.request_spec(store, id, false).map_err(ipc)?;
    engine
        .repeat(
            spec,
            opts,
            Box::new(move |progress| {
                let _ = on_progress.send(progress);
            }),
        )
        .map_err(ipc)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_repeat(engine: EngineState<'_>, id: RepeatId) -> bool {
    engine.cancel_repeat(id)
}
