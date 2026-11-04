//! Offline mocking: the mock library and OpenAPI specs (M20).

use std::sync::Arc;

use pm_model::{FlowId, MockSource, OpenApiSpec, OperationMatch, Rule, StoreId};

use super::dialogs::pick_file;
use super::{EngineState, blocking, run_blocking};

#[tauri::command]
#[specta::specta]
pub fn list_mock_sources(engine: EngineState<'_>) -> Vec<MockSource> {
    engine.mock_sources()
}

/// "Mock from session…" on a document tab (or the live capture): copies
/// the store into the mock library.
#[tauri::command]
#[specta::specta]
pub async fn create_mock_source(
    engine: EngineState<'_>,
    store: StoreId,
    name: Option<String>,
) -> Result<MockSource, String> {
    blocking(&engine, move |e| e.create_mock_source(store, name)).await
}

/// "Mock from session…" for a file that isn't open; without `path`, asks
/// for one. `None` if the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn create_mock_source_from_file(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    path: Option<String>,
) -> Result<Option<MockSource>, String> {
    let path = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => match pick_file(
            &app,
            "Mock from a session, a HAR file or a Charles export",
            (
                "Proxymate session, HAR or Charles (.chlsj, .chlsx)",
                &["proxymate", "har", "chlsj", "chlsx", "chls"],
            ),
        )
        .await?
        {
            Some(path) => path,
            None => return Ok(None),
        },
    };
    blocking(&engine, move |e| e.create_mock_source_from_file(&path, None))
        .await
        .map(Some)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_mock_source(engine: EngineState<'_>, id: String) -> Result<(), String> {
    blocking(&engine, move |e| e.remove_mock_source(&id)).await
}

/// The "Serve from" preset for a mock source (unsaved).
#[tauri::command]
#[specta::specta]
pub fn draft_serve_from_rule(engine: EngineState<'_>, source: String) -> Result<Rule, String> {
    engine.draft_serve_from_rule(&source).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn list_openapi_specs(engine: EngineState<'_>) -> Vec<OpenApiSpec> {
    engine.openapi_specs()
}

/// Imports an OpenAPI 3.0/3.1 file; without `path`, asks for one. `None`
/// if the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn import_openapi_spec(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    path: Option<String>,
) -> Result<Option<OpenApiSpec>, String> {
    let path = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => match pick_file(
            &app,
            "Import an OpenAPI spec",
            ("OpenAPI (JSON or YAML)", &["json", "yaml", "yml"]),
        )
        .await?
        {
            Some(path) => path,
            None => return Ok(None),
        },
    };
    blocking(&engine, move |e| e.import_openapi_file(&path)).await.map(Some)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_openapi_spec(engine: EngineState<'_>, id: u32) -> Result<(), String> {
    blocking(&engine, move |e| e.remove_openapi_spec(id)).await
}

/// "Generate mocks": one disabled Map Local rule per operation, saved.
#[tauri::command]
#[specta::specta]
pub async fn generate_openapi_mocks(engine: EngineState<'_>, id: u32) -> Result<Vec<Rule>, String> {
    blocking(&engine, move |e| e.generate_openapi_mocks(id)).await
}

/// The OpenAPI operation a flow is, for the inspector.
#[tauri::command]
#[specta::specta]
pub async fn flow_operation(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
) -> Result<Option<OperationMatch>, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.flow_operation_in(store, flow).map_err(|e| e.to_string())).await
}
