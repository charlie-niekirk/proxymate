//! Secrets Proxymate keeps for the user (M18): upstream proxy passwords and
//! PKCS#12 passwords. The keychain on macOS; elsewhere, and for engines
//! that must never touch the keychain (tests), a JSON file readable by the
//! owner only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::PlatformError;

/// Where the keychain items live: the generic-password service name.
pub const SERVICE: &str = "Proxymate";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError>;
    fn set(&self, key: &str, secret: &str) -> Result<(), PlatformError>;
    /// Deleting a secret that isn't there succeeds.
    fn delete(&self, key: &str) -> Result<(), PlatformError>;
}

/// The OS keychain (macOS); the 0600 file at `fallback` elsewhere.
pub fn system(fallback: &Path) -> Box<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        let _ = fallback;
        Box::new(crate::macos::secrets::KeychainSecrets)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(FileSecrets::new(fallback.to_owned()))
    }
}

/// Secrets in a JSON object on disk, written with [`crate::fs::write_private`].
pub struct FileSecrets {
    path: PathBuf,
    /// Serialises read-modify-write cycles.
    lock: std::sync::Mutex<()>,
}

impl FileSecrets {
    pub fn new(path: PathBuf) -> FileSecrets {
        FileSecrets {
            path,
            lock: std::sync::Mutex::new(()),
        }
    }

    fn read(&self) -> Result<BTreeMap<String, String>, PlatformError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(err) => Err(err.into()),
        }
    }

    fn write(&self, map: &BTreeMap<String, String>) -> Result<(), PlatformError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::fs::write_private(&self.path, &serde_json::to_vec_pretty(map)?)?;
        Ok(())
    }
}

impl SecretStore for FileSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.read()?.remove(key))
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), PlatformError> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read()?;
        map.insert(key.to_owned(), secret.to_owned());
        self.write(&map)
    }

    fn delete(&self, key: &str) -> Result<(), PlatformError> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read()?;
        if map.remove(key).is_some() {
            self.write(&map)?;
        }
        Ok(())
    }
}

/// Secrets in memory, for engines without a data directory.
#[derive(Default)]
pub struct MemorySecrets {
    map: std::sync::Mutex<BTreeMap<String, String>>,
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>, PlatformError> {
        Ok(self.map.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), PlatformError> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), PlatformError> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_secrets_round_trip_and_stay_private() {
        let dir = std::env::temp_dir().join(format!("pm-secrets-{}", std::process::id()));
        let path = dir.join("secrets.json");
        let store = FileSecrets::new(path.clone());
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "hunter2").unwrap();
        store.set("b", "x").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("hunter2"));
        store.delete("a").unwrap();
        store.delete("missing").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(FileSecrets::new(path.clone()).get("b").unwrap().as_deref(), Some("x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
