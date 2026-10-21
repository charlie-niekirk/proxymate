//! OS integration behind traits, with `cfg`-gated implementations in
//! `macos/`, `linux/` and `windows/`.
//!
//! Planned surface (see `docs/PLAN.md`, "macOS integration"):
//! - `sysproxy`: system proxy set/restore with backup file and guardian (M4)
//! - `trust`: CA install/verify/uninstall in the user trust store (M4)
//! - `procinfo`: peer port → pid → app bundle, name and icon (M4)
//! - `netif`: LAN interface addresses for device setup (M9)
//! - `simctl`: iOS Simulator root-cert install (M9)
//!
//! Empty in M0.
