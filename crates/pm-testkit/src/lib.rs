//! Shared test fixtures: test upstreams (plain h1, and h1/h2 over TLS with
//! a test CA, optionally requiring a client certificate, both with a
//! WebSocket echo and a slow SSE route), proxies to chain to (HTTP with
//! Basic auth, SOCKS5), the body-viewer "zoo", proxy clients, and synthetic
//! flows.
//!
//! The synthetic flow generator lives in `pm-core` so the desktop app and
//! CLI can seed data without depending on test code, and is re-exported
//! here for tests and benches.

pub mod client;
pub mod grpc;
pub mod proxies;
pub mod stream;
pub mod tls;
pub mod upstream;
pub mod zoo;

pub use grpc::GrpcUpstream;
pub use pm_core::synth;
pub use proxies::{HttpProxy, ProxyRequest, Socks5Server};
pub use tls::{Alpn, ClientIdentity, TestCa, TlsUpstream};
pub use upstream::{RawUpstream, Upstream};
