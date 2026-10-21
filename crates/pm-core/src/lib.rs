//! Headless Proxymate engine.
//!
//! Adapters (Tauri commands, MCP tools, the CLI) talk only to [`Engine`].
//! `io/` grows HAR, cURL and sessions in M8.

mod client;
mod engine;
mod ingest;
mod integration;
pub mod io;
pub mod proxy;
mod recorder;
pub mod rules;
pub mod store;
pub mod synth;
pub mod tls;
pub mod trust;

pub use client::ClientApp;
pub use engine::{BodyBytes, DeltaSink, Engine, EngineConfig, PUBLISH_INTERVAL};
pub use integration::SidebarSink;
pub use proxy::ProxyConfig;
pub use recorder::{BodyEnd, FlowRecorder, WsRecorder};
pub use rules::{BreakpointSink, RuleError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown view {0}")]
    UnknownView(pm_model::ViewId),
    #[error("unknown flow {0}")]
    UnknownFlow(pm_model::FlowId),
    #[error("flow {0} has no message {1}")]
    UnknownMessage(pm_model::FlowId, u32),
    #[error("proxy: {0}")]
    Proxy(String),
    #[error(transparent)]
    Decode(#[from] io::decode::DecodeError),
    #[error("flow storage: {0}")]
    Storage(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(String),
    #[error(transparent)]
    Settings(#[from] tls::PolicyError),
    #[error(transparent)]
    Platform(#[from] pm_platform::PlatformError),
    #[error(transparent)]
    Rule(#[from] rules::RuleError),
}
