//! Tauri commands: argument plumbing only, everything delegates to `Engine`.

use std::sync::Arc;

use pm_core::Engine;
use pm_model::{FilterSpec, FlowId, FlowSummary, RowsPage, SortSpec, ViewDelta, ViewId, ViewInfo};
use tauri::State;
use tauri::ipc::Channel;

type EngineState<'a> = State<'a, Arc<Engine>>;

#[tauri::command]
#[specta::specta]
pub fn create_view(engine: EngineState<'_>, filter: FilterSpec, sort: SortSpec) -> ViewInfo {
    engine.create_view(filter, sort)
}

#[tauri::command]
#[specta::specta]
pub fn set_filter(engine: EngineState<'_>, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, String> {
    engine.set_filter(view, filter).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_sort(engine: EngineState<'_>, view: ViewId, sort: SortSpec) -> Result<ViewInfo, String> {
    engine.set_sort(view, sort).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn drop_view(engine: EngineState<'_>, view: ViewId) {
    engine.drop_view(view);
}

#[tauri::command]
#[specta::specta]
pub fn get_rows(engine: EngineState<'_>, view: ViewId, start: u32, count: u32) -> Result<RowsPage, String> {
    engine.get_rows(view, start, count).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_flow(engine: EngineState<'_>, id: FlowId) -> Option<FlowSummary> {
    engine.get_flow(id)
}

/// Streams the view's 20 Hz deltas to the webview until the channel closes.
#[tauri::command]
#[specta::specta]
pub fn subscribe_view(engine: EngineState<'_>, view: ViewId, on_delta: Channel<ViewDelta>) -> Result<(), String> {
    engine
        .subscribe(view, Box::new(move |delta| on_delta.send(delta).is_ok()))
        .map_err(|e| e.to_string())
}
