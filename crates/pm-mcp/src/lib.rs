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
