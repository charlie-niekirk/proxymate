//! Headless Proxymate engine.
//!
//! Adapters (Tauri commands, MCP tools, the CLI) talk only to [`Engine`].
//! Planned modules: `tls/` (M2), `rules/` (M5); `io/` grows HAR, cURL and
//! sessions in M8.

mod engine;
mod ingest;
pub mod io;
pub mod proxy;
mod recorder;
pub mod store;
pub mod synth;

pub use engine::{BodyBytes, DeltaSink, Engine, EngineConfig, PUBLISH_INTERVAL};
pub use proxy::ProxyConfig;
pub use recorder::{BodyEnd, FlowRecorder};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown view {0}")]
    UnknownView(pm_model::ViewId),
    #[error("unknown flow {0}")]
    UnknownFlow(pm_model::FlowId),
    #[error("proxy: {0}")]
    Proxy(String),
    #[error(transparent)]
    Decode(#[from] io::decode::DecodeError),
    #[error("flow storage: {0}")]
    Storage(#[from] std::io::Error),
}
