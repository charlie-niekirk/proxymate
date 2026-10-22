//! MCP server (rmcp) over `Arc<pm_core::Engine>`: Streamable HTTP on
//! `127.0.0.1:9095/mcp` plus a stdio bridge. Like the Tauri commands, every
//! tool calls only the `Engine` facade, so agents and the UI see the same
//! behaviour.
//!
//! - [`http`]: the axum endpoint (bearer token, Host/Origin checks) and the
//!   managed server that follows the MCP settings.
//! - [`handler`]: sessions, read-only mode, the audit log, notifications.
//! - [`tools`]: the tool surface. [`resources`] and [`prompts`] beside it.
//! - [`bridge`]: `mcp-stdio`, relaying a stdio client to the endpoint.

pub mod bridge;
mod handler;
pub mod http;
mod prompts;
mod resources;
mod tools;

pub use handler::ProxymateMcp;
pub use http::{McpServer, serve, spawn_managed};
pub use tools::tool_names;

/// Makes `ring` the process's default rustls provider, unless one is set.
/// rmcp's reqwest client needs one as soon as anything in the build turns
/// on reqwest's `rustls-no-provider` (the desktop app's updater does, and
/// Cargo unifies features across the workspace), even for plain-HTTP
/// loopback. Call it before building an MCP client.
pub fn install_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Losing a race to another installer is fine: one is set either way.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
