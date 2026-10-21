//! Linux and Windows stand-ins until M12: everything reports Unsupported
//! (or nothing), so the engine and its tests run unchanged.

use std::collections::HashMap;
use std::path::Path;

use crate::PlatformError;
use crate::procinfo::{AppInfo, ProcessInfo};
use crate::sysproxy::{Parts, ProxyBackend, ServiceSettings};
use crate::trust::{CaCert, TrustState, TrustStore};

pub struct NoProxyBackend;

impl ProxyBackend for NoProxyBackend {
    fn active_services(&self) -> Result<Vec<String>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn read(&self, _service: &str) -> Result<ServiceSettings, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn point_at(&self, _service: &str, _host: &str, _port: u16, _bypass: &[String]) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn write(&self, _saved: &ServiceSettings, _now: &ServiceSettings, _parts: Parts) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

pub struct NoTrustStore;

impl TrustStore for NoTrustStore {
    fn state(&self, _ca: CaCert<'_>) -> Result<TrustState, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn install(&self, _ca: CaCert<'_>) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn uninstall(&self, _ca: CaCert<'_>) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

pub struct NoProcessInfo;

impl ProcessInfo for NoProcessInfo {
    fn loopback_peers(&self, _remote_port: u16) -> Result<HashMap<u16, u32>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn app_for_pid(&self, _pid: u32) -> Option<AppInfo> {
        None
    }
    fn icon_png(&self, _bundle_path: &Path, _size: u32) -> Option<Vec<u8>> {
        None
    }
}
