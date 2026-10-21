//! MCP server (rmcp) over `Arc<pm_core::Engine>`: Streamable HTTP on
//! `127.0.0.1:9095/mcp` plus a stdio bridge. Like the Tauri commands, every
//! tool calls only the `Engine` facade.
//!
//! Empty until M7.
