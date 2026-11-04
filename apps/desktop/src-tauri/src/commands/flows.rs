//! Views, rows, flow details, bodies and WebSocket/SSE messages.

use std::sync::Arc;

use pm_model::{
    BodyPart, FilterSpec, FlowDetail, FlowId, FlowSummary, RowsPage, SortSpec, SsePage, StoreId, ViewDelta, ViewId,
    ViewInfo, WsPage, WsQuery, WsSend,
};
use tauri::ipc::Channel;
use tauri::ipc::InvokeResponseBody;
use tauri::ipc::IpcResponse;

use super::{EngineState, engine_of, run_blocking, run_blocking_or_default};

/// `get_rows` returns at most this many rows per call (R2): the table's
/// row cache asks for up to four 128-row pages at once
/// (`MAX_ROWS_PER_CALL` in `lib/rowCache.ts`).
const MAX_ROWS: u32 = 512;

#[tauri::command]
#[specta::specta]
pub async fn create_view(
    engine: EngineState<'_>,
    store: StoreId,
    filter: FilterSpec,
    sort: SortSpec,
) -> Result<ViewInfo, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.create_view_in(store, filter, sort).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_filter(engine: EngineState<'_>, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.set_filter(view, filter).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_sort(engine: EngineState<'_>, view: ViewId, sort: SortSpec) -> Result<ViewInfo, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.set_sort(view, sort).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn drop_view(app: tauri::AppHandle, view: ViewId) {
    let engine = engine_of(&app);
    run_blocking_or_default(move || {
        engine.drop_view(view);
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_rows(engine: EngineState<'_>, view: ViewId, start: u32, count: u32) -> Result<RowsPage, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .get_rows(view, start, count.min(MAX_ROWS))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_flow(app: tauri::AppHandle, store: StoreId, id: FlowId) -> Option<FlowSummary> {
    let engine = engine_of(&app);
    run_blocking_or_default(move || engine.get_flow_in(store, id)).await
}

/// Streams the view's 20 Hz deltas to the webview until the channel closes.
#[tauri::command]
#[specta::specta]
pub async fn subscribe_view(engine: EngineState<'_>, view: ViewId, on_delta: Channel<ViewDelta>) -> Result<(), String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .subscribe(view, Box::new(move |delta| on_delta.send(delta).is_ok()))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn get_flow_detail(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
) -> Result<Option<FlowDetail>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.flow_detail_in(store, id).map_err(|e| e.to_string())).await
}

/// A body's bytes as a raw `ArrayBuffer`, not JSON. With `decoded`, a
/// content-encoded body is decoded first; decoding errors are returned.
/// With `max`, only the first `max` bytes are sent.
#[tauri::command]
#[specta::specta]
pub async fn get_body(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    part: BodyPart,
    decoded: bool,
    max: Option<u64>,
) -> Result<RawBytes, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        let max = max.map(|m| usize::try_from(m).unwrap_or(usize::MAX));
        engine
            .body_head_in(store, id, part, decoded, max)
            .map(|body| RawBytes(tauri::ipc::Response::new(body.bytes)))
            .map_err(|e| e.to_string())
    })
    .await
}

/// Asks where to save a body (decoded, with `decoded`) and writes it there.
/// Returns the path, or `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn save_body(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    part: BodyPart,
    decoded: bool,
    file_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(&file_name).save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    engine
        .save_body_in(store, id, part, decoded, &path)
        .map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn get_websocket_messages(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    query: WsQuery,
    start: u32,
    count: u32,
) -> Result<WsPage, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .websocket_messages_in(store, id, &query, start, count)
            .map_err(|e| e.to_string())
    })
    .await
}

/// One WebSocket message's payload as a raw `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub async fn get_websocket_payload(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    index: u32,
) -> Result<RawBytes, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .websocket_payload_in(store, id, index)
            .map(|bytes| RawBytes(tauri::ipc::Response::new(bytes)))
            .map_err(|e| e.to_string())
    })
    .await
}

/// What arrived for an edited WebSocket message (M19), as a raw
/// `ArrayBuffer`.
#[tauri::command]
#[specta::specta]
pub async fn get_websocket_original(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    index: u32,
) -> Result<RawBytes, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || match engine.websocket_original_in(store, id, index) {
        Ok(Some(bytes)) => Ok(RawBytes(tauri::ipc::Response::new(bytes))),
        Ok(None) => Err(format!("message {} wasn't edited", index + 1)),
        Err(e) => Err(e.to_string()),
    })
    .await
}

/// Sends a message into an open WebSocket of the live capture.
#[tauri::command]
#[specta::specta]
pub fn ws_send(engine: EngineState<'_>, id: FlowId, message: WsSend) -> Result<(), String> {
    engine.ws_send(id, &message).map_err(|e| e.to_string())
}

/// Closes an open WebSocket of the live capture.
#[tauri::command]
#[specta::specta]
pub fn ws_close(engine: EngineState<'_>, id: FlowId) -> Result<(), String> {
    engine.ws_close(id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_sse_events(
    engine: EngineState<'_>,
    store: StoreId,
    id: FlowId,
    text: String,
    start: u32,
    count: u32,
) -> Result<SsePage, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || {
        engine
            .sse_events_in(store, id, &text, start, count)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_flows(app: tauri::AppHandle) {
    let engine = engine_of(&app);
    run_blocking_or_default(move || {
        engine.clear();
    })
    .await
}

/// Bytes sent to the webview as an `ArrayBuffer`. specta has no type for
/// `tauri::ipc::Response`, so the binding says `unknown` and `lib/backend.ts`
/// narrows it.
pub struct RawBytes(pub(super) tauri::ipc::Response);

impl IpcResponse for RawBytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        self.0.body()
    }
}

impl specta::Type for RawBytes {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <specta_typescript::Unknown as specta::Type>::definition(types)
    }
}
