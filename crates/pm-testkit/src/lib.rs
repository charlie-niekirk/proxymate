//! Shared test fixtures: test upstreams (plain h1, and h1/h2 over TLS with
//! a test CA; WS echo and SSE in later milestones), the body-viewer "zoo",
//! proxy clients, and synthetic flows.
//!
//! The synthetic flow generator lives in `pm-core` so the desktop app and
//! CLI can seed data without depending on test code, and is re-exported
//! here for tests and benches.

pub mod client;
pub mod tls;
pub mod upstream;
pub mod zoo;

pub use pm_core::synth;
pub use tls::{Alpn, TestCa, TlsUpstream};
pub use upstream::{RawUpstream, Upstream};
