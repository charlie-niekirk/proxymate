//! Headless Proxymate engine.
//!
//! Adapters (Tauri commands, MCP tools, the CLI) talk only to [`Engine`].
//! Planned modules from M1 on: `proxy/`, `tls/`, `rules/`, `io/` (HAR, cURL,
//! sessions, content detection).

mod engine;
pub mod store;
pub mod synth;

pub use engine::{DeltaSink, Engine, PUBLISH_INTERVAL};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown view {0}")]
    UnknownView(pm_model::ViewId),
}
