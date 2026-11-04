//! The MCP server's settings, token and audit log (M7).

use std::sync::Arc;

use pm_model::{McpAuditPage, McpSettings, McpStatus};
use tauri::ipc::Channel;

use super::{EngineState, run_blocking};

#[tauri::command]
#[specta::specta]
pub fn get_mcp_status(engine: EngineState<'_>) -> McpStatus {
    engine.mcp_status()
}

/// Streams the MCP status (connected clients, settings, errors).
#[tauri::command]
#[specta::specta]
pub fn subscribe_mcp(engine: EngineState<'_>, on_status: Channel<McpStatus>) {
    engine.subscribe_mcp(Box::new(move |status| on_status.send(status).is_ok()));
}

#[tauri::command]
#[specta::specta]
pub fn set_mcp_settings(engine: EngineState<'_>, settings: McpSettings) -> Result<McpSettings, String> {
    engine.set_mcp_settings(settings).map_err(|e| e.to_string())
}

/// The bearer token, for the copy buttons.
#[tauri::command]
#[specta::specta]
pub fn get_mcp_token(engine: EngineState<'_>) -> String {
    engine.mcp_token()
}

#[tauri::command]
#[specta::specta]
pub fn regenerate_mcp_token(engine: EngineState<'_>) -> Result<String, String> {
    engine.regenerate_mcp_token().map_err(|e| e.to_string())
}

/// Audit entries older than `before` (all, without), newest first.
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_audit(engine: EngineState<'_>, before: Option<i64>, limit: u32) -> Result<McpAuditPage, String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.mcp_audit(before, limit).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_mcp_audit(engine: EngineState<'_>) -> Result<(), String> {
    let engine = Arc::clone(&engine);
    run_blocking(move || engine.clear_mcp_audit().map_err(|e| e.to_string())).await
}
