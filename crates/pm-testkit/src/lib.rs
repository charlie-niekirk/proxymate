//! Shared test fixtures: test upstreams (h1 now; h2 TLS, WS echo, SSE and
//! the body-viewer "zoo" in later milestones), proxy clients, and synthetic
//! flows.
//!
//! The synthetic flow generator lives in `pm-core` so the desktop app and
//! CLI can seed data without depending on test code, and is re-exported
//! here for tests and benches.

pub mod client;
pub mod upstream;

pub use pm_core::synth;
pub use upstream::{RawUpstream, Upstream};
