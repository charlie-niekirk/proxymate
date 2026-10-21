//! The OS's HTTP and HTTPS proxy: point it at Proxymate, and always put it
//! back.
//!
//! Before anything changes, the current settings of every service we touch
//! go into a backup file with our pid. Restoring reads that file and puts
//! back only what still points at us, so a setting the user changed in the
//! meantime is left alone. The app restores on quit, on SIGINT/SIGTERM and
//! from its panic hook; a guardian process restores after a crash or
//! SIGKILL; and a stale backup (its pid is gone) is restored at startup.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::PlatformError;

/// Hosts that never go through the proxy.
pub const BYPASS: [&str; 4] = ["localhost", "127.0.0.1", "*.local", "169.254/16"];

/// One proxy entry (web or secure web) of a network service.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProxySetting {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}

impl ProxySetting {
    fn points_at(&self, host: &str, port: u16) -> bool {
        self.enabled && self.server == host && self.port == port
    }
}

/// The proxy settings of one network service (e.g. `Wi-Fi`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServiceSettings {
    pub service: String,
    pub web: ProxySetting,
    pub secure: ProxySetting,
    pub bypass: Vec<String>,
}

/// What was there before we changed anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// The process that changed the settings. Once it is gone the backup
    /// is stale and gets restored.
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub services: Vec<ServiceSettings>,
}

/// Reads and writes the OS settings. `networksetup` on macOS.
pub trait ProxyBackend: Send + Sync {
    /// Enabled services that currently have an address.
    fn active_services(&self) -> Result<Vec<String>, PlatformError>;
    fn read(&self, service: &str) -> Result<ServiceSettings, PlatformError>;
    /// Points web and secure web at `host:port` and sets the bypass list.
    fn point_at(&self, service: &str, host: &str, port: u16, bypass: &[String]) -> Result<(), PlatformError>;
    /// Writes back the given parts of `saved`. `now` is what is there, so
    /// unchanged fields need not be rewritten.
    fn write(&self, saved: &ServiceSettings, now: &ServiceSettings, parts: Parts) -> Result<(), PlatformError>;
}

/// Which parts of a service's settings to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Parts {
    pub web: bool,
    pub secure: bool,
    pub bypass: bool,
}

impl Parts {
    pub fn any(self) -> bool {
        self.web || self.secure || self.bypass
    }
}

/// What the system proxy points at now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// Services whose web or secure web proxy points at us.
    pub services: Vec<String>,
    /// True if a backup from this process exists (we turned it on).
    pub owned: bool,
}

pub struct SystemProxy {
    backend: Box<dyn ProxyBackend>,
    backup_path: PathBuf,
}

impl SystemProxy {
    pub fn new(backend: Box<dyn ProxyBackend>, backup_path: PathBuf) -> SystemProxy {
        SystemProxy { backend, backup_path }
    }

    /// The OS backend, with its backup file at `backup_path`.
    pub fn system(backup_path: PathBuf) -> SystemProxy {
        #[cfg(target_os = "macos")]
        let backend: Box<dyn ProxyBackend> = Box::new(crate::macos::networksetup::NetworkSetup);
        #[cfg(not(target_os = "macos"))]
        let backend: Box<dyn ProxyBackend> = Box::new(crate::stub::NoProxyBackend);
        SystemProxy::new(backend, backup_path)
    }

    pub fn backup_path(&self) -> &Path {
        &self.backup_path
    }

    /// Points every active service at `host:port`. Returns the services
    /// changed. The backup is written first; if a change fails, what was
    /// already changed is put back.
    pub fn enable(&self, host: &str, port: u16) -> Result<Vec<String>, PlatformError> {
        if let Some(backup) = read_backup(&self.backup_path)? {
            if backup.pid != std::process::id() || backup.host != host || backup.port != port {
                // Someone else's backup, or ours for another address: put it
                // back first so the new backup records the user's settings.
                self.restore()?;
            } else {
                // Already ours: re-point services in case one was reset.
                return self.point_all(&backup);
            }
        }
        let services = self.backend.active_services()?;
        if services.is_empty() {
            return Err(PlatformError::Os("no active network service to configure".into()));
        }
        let services = parallel(&services, |s| self.backend.read(s))?;
        let backup = Backup {
            pid: std::process::id(),
            host: host.to_owned(),
            port,
            services,
        };
        write_backup(&self.backup_path, &backup)?;
        match self.point_all(&backup) {
            Ok(changed) => Ok(changed),
            Err(err) => {
                let _ = self.restore();
                Err(err)
            }
        }
    }

    fn point_all(&self, backup: &Backup) -> Result<Vec<String>, PlatformError> {
        let bypass: Vec<String> = BYPASS.iter().map(|s| (*s).to_owned()).collect();
        let names: Vec<String> = backup.services.iter().map(|s| s.service.clone()).collect();
        parallel(&names, |s| self.backend.point_at(s, &backup.host, backup.port, &bypass))?;
        Ok(names)
    }

    /// Puts back what the backup recorded, for each setting that still
    /// points at us, then deletes the backup. Returns the services restored
    /// (none if there was no backup).
    pub fn restore(&self) -> Result<Vec<String>, PlatformError> {
        let Some(backup) = read_backup(&self.backup_path)? else {
            return Ok(Vec::new());
        };
        let ours: Vec<String> = BYPASS.iter().map(|s| (*s).to_owned()).collect();
        let restored = parallel(&backup.services, |saved| {
            let now = self.backend.read(&saved.service)?;
            let parts = Parts {
                web: now.web.points_at(&backup.host, backup.port),
                secure: now.secure.points_at(&backup.host, backup.port),
                bypass: now.bypass == ours && saved.bypass != ours,
            };
            if parts.any() {
                self.backend.write(saved, &now, parts)?;
            }
            Ok((parts.web || parts.secure).then(|| saved.service.clone()))
        })?;
        remove_backup(&self.backup_path)?;
        Ok(restored.into_iter().flatten().collect())
    }

    /// Restores a backup left by a process that is gone (a crash, SIGKILL
    /// with no guardian). Returns the services restored.
    pub fn restore_if_stale(&self) -> Result<Vec<String>, PlatformError> {
        match read_backup(&self.backup_path)? {
            Some(backup) if !crate::process::is_alive(backup.pid) || backup.pid == std::process::id() => self.restore(),
            _ => Ok(Vec::new()),
        }
    }

    /// Waits for `pid` to exit, then restores its backup if it left one.
    /// This is the guardian process's whole job.
    pub fn guard(&self, pid: u32) -> Result<Vec<String>, PlatformError> {
        crate::process::wait_for_exit(pid)?;
        match read_backup(&self.backup_path)? {
            Some(backup) if backup.pid == pid => self.restore(),
            _ => Ok(Vec::new()),
        }
    }

    /// Which active services point at `host:port` now.
    pub fn status(&self, host: &str, port: u16) -> Result<Status, PlatformError> {
        let owned = read_backup(&self.backup_path)?.is_some_and(|b| b.pid == std::process::id());
        let services = self.backend.active_services()?;
        let settings = parallel(&services, |s| self.backend.read(s))?;
        Ok(Status {
            services: settings
                .into_iter()
                .filter(|s| s.web.points_at(host, port) || s.secure.points_at(host, port))
                .map(|s| s.service)
                .collect(),
            owned,
        })
    }
}

pub fn read_backup(path: &Path) -> Result<Option<Backup>, PlatformError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn write_backup(path: &Path, backup: &Backup) -> Result<(), PlatformError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::fs::write_private(path, &serde_json::to_vec_pretty(backup)?)?;
    Ok(())
}

fn remove_backup(path: &Path) -> Result<(), PlatformError> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

/// Runs `f` for each item on its own thread: each `networksetup` call takes
/// tens of milliseconds, and a restore after SIGKILL should finish in well
/// under a second. Fails with the first error, after every call finished.
fn parallel<T: Sync, R: Send>(
    items: &[T],
    f: impl Fn(&T) -> Result<R, PlatformError> + Sync,
) -> Result<Vec<R>, PlatformError> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.iter().map(|item| scope.spawn(|| f(item))).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(PlatformError::Os("worker panicked".into())))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// An in-memory OS.
    #[derive(Clone, Default)]
    struct Fake {
        services: Arc<Mutex<BTreeMap<String, ServiceSettings>>>,
        active: Vec<String>,
        fail_point_at: Option<String>,
    }

    impl Fake {
        fn with(services: &[(&str, ProxySetting)], active: &[&str]) -> Fake {
            let map = services
                .iter()
                .map(|(name, web)| {
                    (
                        name.to_string(),
                        ServiceSettings {
                            service: name.to_string(),
                            web: web.clone(),
                            secure: web.clone(),
                            bypass: vec!["*.corp".into()],
                        },
                    )
                })
                .collect();
            Fake {
                services: Arc::new(Mutex::new(map)),
                active: active.iter().map(|s| s.to_string()).collect(),
                fail_point_at: None,
            }
        }

        fn get(&self, name: &str) -> ServiceSettings {
            self.services.lock().unwrap()[name].clone()
        }
    }

    impl ProxyBackend for Fake {
        fn active_services(&self) -> Result<Vec<String>, PlatformError> {
            Ok(self.active.clone())
        }
        fn read(&self, service: &str) -> Result<ServiceSettings, PlatformError> {
            Ok(self.get(service))
        }
        fn point_at(&self, service: &str, host: &str, port: u16, bypass: &[String]) -> Result<(), PlatformError> {
            if self.fail_point_at.as_deref() == Some(service) {
                return Err(PlatformError::Os("denied".into()));
            }
            let mut map = self.services.lock().unwrap();
            let s = map.get_mut(service).unwrap();
            let ours = ProxySetting {
                enabled: true,
                server: host.into(),
                port,
            };
            s.web = ours.clone();
            s.secure = ours;
            s.bypass = bypass.to_vec();
            Ok(())
        }
        fn write(&self, settings: &ServiceSettings, _now: &ServiceSettings, parts: Parts) -> Result<(), PlatformError> {
            let mut map = self.services.lock().unwrap();
            let s = map.get_mut(&settings.service).unwrap();
            if parts.web {
                s.web = settings.web.clone();
            }
            if parts.secure {
                s.secure = settings.secure.clone();
            }
            if parts.bypass {
                s.bypass = settings.bypass.clone();
            }
            Ok(())
        }
    }

    fn temp_backup() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("pm-sysproxy-{}-{n}/backup.json", std::process::id()))
    }

    fn corp() -> ProxySetting {
        ProxySetting {
            enabled: true,
            server: "proxy.corp".into(),
            port: 3128,
        }
    }

    #[test]
    fn enable_backs_up_then_restore_puts_everything_back() {
        let fake = Fake::with(
            &[("Wi-Fi", corp()), ("Ethernet", ProxySetting::default())],
            &["Wi-Fi", "Ethernet"],
        );
        let before = (fake.get("Wi-Fi"), fake.get("Ethernet"));
        let path = temp_backup();
        let proxy = SystemProxy::new(Box::new(fake.clone()), path.clone());

        let changed = proxy.enable("127.0.0.1", 9090).unwrap();
        assert_eq!(changed, vec!["Wi-Fi", "Ethernet"]);
        let backup = read_backup(&path).unwrap().unwrap();
        assert_eq!(backup.pid, std::process::id());
        assert_eq!(backup.services, vec![before.0.clone(), before.1.clone()]);
        assert!(fake.get("Wi-Fi").web.points_at("127.0.0.1", 9090));
        assert_eq!(fake.get("Wi-Fi").bypass, BYPASS.map(String::from).to_vec());
        let status = proxy.status("127.0.0.1", 9090).unwrap();
        assert!(status.owned);
        assert_eq!(status.services.len(), 2);

        let restored = proxy.restore().unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!((fake.get("Wi-Fi"), fake.get("Ethernet")), before);
        assert!(read_backup(&path).unwrap().is_none());
        assert_eq!(proxy.restore().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn settings_the_user_changed_since_are_left_alone() {
        let fake = Fake::with(&[("Wi-Fi", corp())], &["Wi-Fi"]);
        let proxy = SystemProxy::new(Box::new(fake.clone()), temp_backup());
        proxy.enable("127.0.0.1", 9090).unwrap();
        // The user points the secure proxy elsewhere while we run.
        let mine = ProxySetting {
            enabled: true,
            server: "other".into(),
            port: 1,
        };
        fake.services.lock().unwrap().get_mut("Wi-Fi").unwrap().secure = mine.clone();
        proxy.restore().unwrap();
        let now = fake.get("Wi-Fi");
        assert_eq!(now.web, corp());
        assert_eq!(now.secure, mine);
    }

    #[test]
    fn a_failed_change_rolls_back() {
        let mut fake = Fake::with(&[("Wi-Fi", corp()), ("VPN", corp())], &["Wi-Fi", "VPN"]);
        fake.fail_point_at = Some("VPN".into());
        let path = temp_backup();
        let proxy = SystemProxy::new(Box::new(fake.clone()), path.clone());
        assert!(proxy.enable("127.0.0.1", 9090).is_err());
        assert_eq!(fake.get("Wi-Fi").web, corp());
        assert!(read_backup(&path).unwrap().is_none());
    }

    #[test]
    fn stale_backups_are_restored_and_live_ones_kept() {
        let fake = Fake::with(&[("Wi-Fi", corp())], &["Wi-Fi"]);
        let path = temp_backup();
        let proxy = SystemProxy::new(Box::new(fake.clone()), path.clone());
        proxy.enable("127.0.0.1", 9090).unwrap();

        // A backup from a live process that isn't us is kept.
        let mut backup = read_backup(&path).unwrap().unwrap();
        backup.pid = 1;
        write_backup(&path, &backup).unwrap();
        assert!(proxy.restore_if_stale().unwrap().is_empty());
        assert!(fake.get("Wi-Fi").web.points_at("127.0.0.1", 9090));

        // One from a dead process is restored.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        backup.pid = child.id();
        child.wait().unwrap();
        write_backup(&path, &backup).unwrap();
        assert_eq!(proxy.restore_if_stale().unwrap(), vec!["Wi-Fi"]);
        assert_eq!(fake.get("Wi-Fi").web, corp());
    }

    #[test]
    fn the_guardian_restores_once_its_process_exits() {
        let fake = Fake::with(&[("Wi-Fi", corp())], &["Wi-Fi"]);
        let path = temp_backup();
        let proxy = SystemProxy::new(Box::new(fake.clone()), path.clone());
        proxy.enable("127.0.0.1", 9090).unwrap();
        let mut child = std::process::Command::new("sleep").arg("0.2").spawn().unwrap();
        let mut backup = read_backup(&path).unwrap().unwrap();
        backup.pid = child.id();
        write_backup(&path, &backup).unwrap();
        let reaper = std::thread::spawn(move || child.wait());
        assert_eq!(proxy.guard(backup.pid).unwrap(), vec!["Wi-Fi"]);
        reaper.join().unwrap().unwrap();
        assert_eq!(fake.get("Wi-Fi").web, corp());
    }
}
