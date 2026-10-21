//! OS integration behind traits, with `cfg`-gated implementations in
//! `macos/` and stubs elsewhere (Linux and Windows land in M12).
//!
//! - `sysproxy`: system proxy set/restore with a backup file; the guardian
//!   waits on the app with `process::wait_for_exit` (M4)
//! - `trust`: CA install/uninstall in the user trust store (M4)
//! - `procinfo`: peer port → pid → app bundle, name and icon (M4)
//! - `process`: pid liveness and exit waits (M4)
//! - `netif`: LAN interface addresses for device setup (M9)
//! - `simctl`: iOS Simulator root-cert install (M9)
//! - `fs`: private files (the CA key) (M2)

pub mod fs;
pub mod process;
pub mod procinfo;
pub mod sysproxy;
pub mod trust;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod stub;

/// Why an OS integration call failed.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("not supported on this platform yet")]
    Unsupported,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// A system tool or API refused, with its own message.
    #[error("{0}")]
    Os(String),
    /// The user dismissed the system's password or Touch ID prompt.
    #[error("cancelled")]
    Cancelled,
}
