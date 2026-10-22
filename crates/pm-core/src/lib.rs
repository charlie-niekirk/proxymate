//! Headless Proxymate engine.
//!
//! Adapters (Tauri commands, MCP tools, the CLI) talk only to [`Engine`].

mod client;
mod devices;
mod documents;
mod engine;
mod ingest;
mod integration;
pub mod io;
pub mod mcp;
mod notify;
pub mod proxy;
mod recorder;
mod replay;
pub mod rules;
mod search;
mod settings;
pub mod store;
pub mod synth;
pub mod tls;
pub mod trust;
mod updates;

pub use client::{ClientApp, ClientKind};
pub use devices::{DevicesSink, HOLD as DEVICE_HOLD, ONBOARDING_HOST};
pub use documents::ProgressSink;
pub use engine::{BodyBytes, DeltaSink, Engine, EngineConfig, PUBLISH_INTERVAL};
pub use integration::SidebarSink;
pub use mcp::McpSink;
pub use proxy::ProxyConfig;
pub use recorder::{BodyEnd, FlowRecorder, WsRecorder};
pub use rules::{BreakpointSink, RuleError, RulesSink};
pub use search::{FlowPage, SearchHit, SearchMatch, SearchPage};
pub use settings::StorageSink;

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
    /// A filter DSL query or search pattern that doesn't parse.
    #[error("filter: {0}")]
    Query(String),
    /// Device setup: a bad address, a missing tool, a non-emulator serial.
    #[error("{0}")]
    Device(String),
    /// `Engine::send`: the request can't be sent as given.
    #[error("{0}")]
    Send(String),
    /// Saving, opening or importing a session or HAR file.
    #[error("{0}")]
    Document(String),
    #[error("unknown store {0}")]
    UnknownStore(pm_model::StoreId),
    /// A setting out of range, with what is allowed.
    #[error("{0}")]
    Invalid(String),
}
