//! Shared test fixtures: test upstreams (h1/h2 TLS, WS echo, SSE, slow and
//! huge bodies, the body-viewer "zoo"), a test CA, and synthetic flows.
//!
//! In M0 only the synthetic flow generator exists; it lives in `pm-core` so
//! the desktop app and CLI can seed data without depending on test code, and
//! is re-exported here for tests and benches.

pub use pm_core::synth;
