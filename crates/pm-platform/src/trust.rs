//! The OS trust store: is the Proxymate CA a trusted root, and install or
//! remove it. On macOS that is the login keychain plus a user-domain trust
//! setting, which needs no sudo: macOS shows its own password or Touch ID
//! prompt.

use std::path::Path;

use crate::PlatformError;

/// What the trust store says about a certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    /// Marked as a trusted root (user or admin domain).
    Trusted,
    /// Explicitly distrusted, e.g. "Never Trust" in Keychain Access.
    Denied,
    /// No trust setting for it.
    NotTrusted,
}

/// The CA a trust store call is about.
#[derive(Debug, Clone, Copy)]
pub struct CaCert<'a> {
    pub der: &'a [u8],
    /// The same certificate as a PEM file, for command-line fallbacks.
    pub pem_path: &'a Path,
    /// SHA-1 of the DER, upper-case hex: how `security` names certificates.
    pub sha1_hex: &'a str,
}

pub trait TrustStore: Send + Sync {
    fn state(&self, ca: CaCert<'_>) -> Result<TrustState, PlatformError>;
    /// Adds the certificate to the user's keychain and trusts it as a root
    /// for every use. Blocks until the user answers the system prompt.
    fn install(&self, ca: CaCert<'_>) -> Result<(), PlatformError>;
    /// Removes the trust setting and deletes the certificate. Also blocks
    /// on a system prompt.
    fn uninstall(&self, ca: CaCert<'_>) -> Result<(), PlatformError>;
}

/// The trust store of this OS.
pub fn system() -> Box<dyn TrustStore> {
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::macos::trust::Keychain)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(crate::stub::NoTrustStore)
    }
}
