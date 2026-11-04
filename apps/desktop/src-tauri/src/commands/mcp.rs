//! The MCP server's settings, token and audit log (M7).

use pm_model::{IpcError, McpAuditPage, McpSettings, McpStatus};
use tauri::ipc::Channel;

use super::{EngineState, blocking, ipc};

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
pub fn set_mcp_settings(engine: EngineState<'_>, settings: McpSettings) -> Result<McpSettings, IpcError> {
    engine.set_mcp_settings(settings).map_err(ipc)
}

/// The bearer token, for the copy buttons.
#[tauri::command]
#[specta::specta]
pub fn get_mcp_token(engine: EngineState<'_>) -> String {
    engine.mcp_token()
}

#[tauri::command]
#[specta::specta]
pub fn regenerate_mcp_token(engine: EngineState<'_>) -> Result<String, IpcError> {
    engine.regenerate_mcp_token().map_err(ipc)
}

/// Audit entries older than `before` (all, without), newest first.
#[tauri::command]
#[specta::specta]
pub async fn get_mcp_audit(engine: EngineState<'_>, before: Option<i64>, limit: u32) -> Result<McpAuditPage, IpcError> {
    blocking(&engine, move |engine| engine.mcp_audit(before, limit)).await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_mcp_audit(engine: EngineState<'_>) -> Result<(), IpcError> {
    blocking(&engine, move |engine| engine.clear_mcp_audit()).await
}
