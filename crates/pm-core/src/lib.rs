//! Headless Proxymate engine.
//!
//! Adapters (Tauri commands, MCP tools, the CLI) talk only to [`Engine`].

mod capture;
mod client;
mod devices;
mod documents;
mod engine;
mod ingest;
mod integration;
pub(crate) mod io;
pub(crate) mod mcp;
mod mocks;
mod network;
mod notify;
pub(crate) mod openapi;
mod protos;
pub(crate) mod proxy;
mod recorder;
mod replay;
pub(crate) mod rules;
pub(crate) mod scripts;
mod search;
mod settings;
pub(crate) mod store;
pub(crate) mod synth;
pub(crate) mod tls;
pub(crate) mod trust;
mod updates;
mod workflow;

/// Not part of the API. What `pm-core`'s integration tests and benches (and
/// `pm-testkit`) need beyond [`Engine`]; adapters must not use it.
#[doc(hidden)]
pub mod test_support {
    pub use crate::proxy::APP_CLIENT;
    pub use crate::rules::REWRITE_BODY_CAP;
    pub mod synth {
        pub use crate::synth::{Generator, seed, spawn_live};
    }
}

pub use client::{ClientApp, ClientKind};
pub use devices::{DevicesSink, HOLD as DEVICE_HOLD, ONBOARDING_HOST};
pub use documents::ProgressSink;
pub use engine::{BodyBytes, DeltaSink, Engine, EngineConfig, PUBLISH_INTERVAL};
pub use integration::SidebarSink;
pub use mcp::{McpCall, McpSink};
pub use protos::ProtoImport;
pub use proxy::ProxyConfig;
pub use recorder::{BodyEnd, FlowRecorder, WsRecorder};
pub use rules::{BreakpointSink, RuleError, RulesSink};
pub use scripts::{ConsoleSink, ScriptError, ScriptsSink};
pub use search::{FlowPage, SearchHit, SearchMatch, SearchPage};
pub use settings::StorageSink;
pub use workflow::{MAX_REPEAT_CONCURRENCY, MAX_REPEAT_ITERATIONS, RepeatSink};

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
    #[error(transparent)]
    Script(#[from] scripts::ScriptError),
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
    /// A protobuf schema that doesn't compile or load, or a bad mapping.
    #[error("protobuf: {0}")]
    Proto(String),
    /// Sending into or closing a WebSocket that can't take it.
    #[error("{0}")]
    WebSocket(String),
}
