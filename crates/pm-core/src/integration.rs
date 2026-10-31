//! The OS-facing part of the [`Engine`] facade: CA trust, the system proxy
//! and its guardian, client apps and their icons, the sidebar aggregates,
//! per-app SSL exclusions and the proxied-terminal environment. The OS
//! calls themselves live behind `pm-platform` traits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use parking_lot::Mutex;
use pm_model::{
    AppCount, KeychainTrust, LIVE_STORE, SidebarSnapshot, SslSettings, StoreId, SystemProxyStatus, TrustStatus,
};
use pm_platform::PlatformError;
use pm_platform::sysproxy::SystemProxy;
use pm_platform::trust::{CaCert, TrustState, TrustStore};

use crate::client::{ClientKind, ClientResolver};
use crate::devices::DeviceRegistry;
use crate::{Engine, Error};

const SYSPROXY_BACKUP_FILE: &str = "sysproxy-backup.json";

/// Receives sidebar snapshots. Returning `false` unsubscribes.
pub type SidebarSink = Box<dyn FnMut(SidebarSnapshot) -> bool + Send>;

/// OS integration state owned by the engine.
pub(crate) struct Platform {
    pub clients: ClientResolver,
    trust: Box<dyn TrustStore>,
    sysproxy: Arc<SystemProxy>,
    sysproxy_error: Mutex<Option<String>>,
    /// `<exe> --sysproxy-guardian <pid> --data-dir <dir>` is spawned the
    /// first time the system proxy is turned on, if set.
    guardian_exe: Option<PathBuf>,
    guardian_spawned: AtomicBool,
    sidebar_sink: Mutex<Option<SidebarSink>>,
    /// The store the sidebar counts: the live capture or the document tab
    /// the UI shows.
    sidebar_store: AtomicU32,
    pub devices: DeviceRegistry,
}

impl Platform {
    /// For reading the system proxy off the async threads.
    pub fn sysproxy_handle(&self) -> Arc<SystemProxy> {
        self.sysproxy.clone()
    }

    pub fn new(root: &Path, guardian_exe: Option<PathBuf>, devices: DeviceRegistry) -> Platform {
        Platform {
            clients: ClientResolver::new(pm_platform::procinfo::system()),
            trust: pm_platform::trust::system(),
            sysproxy: Arc::new(SystemProxy::system(root.join(SYSPROXY_BACKUP_FILE))),
            sysproxy_error: Mutex::new(None),
            guardian_exe,
            guardian_spawned: AtomicBool::new(false),
            sidebar_sink: Mutex::new(None),
            sidebar_store: AtomicU32::new(LIVE_STORE),
            devices,
        }
    }
}

impl Engine {
    // ---- Sidebar ----

    /// Apps and Domains with their flow counts, aggregated in the store
    /// the sidebar follows ([`Engine::set_sidebar_store`]).
    pub fn sidebar(&self) -> SidebarSnapshot {
        let which = self.platform.sidebar_store.load(Ordering::Relaxed);
        let doc = match which {
            LIVE_STORE => None,
            _ => self.docs.get(which).ok(),
        };
        let guard = match &doc {
            Some(doc) => doc.store.read(),
            None => self.store.read(),
        };
        let store = &*guard;
        let counts = store.sidebar();
        SidebarSnapshot {
            store: if doc.is_some() { which } else { LIVE_STORE },
            total: store.total(),
            apps: counts
                .apps()
                .into_iter()
                .map(|(name, count)| {
                    let app = self.platform.clients.known(&name);
                    AppCount {
                        name: name.to_string(),
                        bundle_id: app.as_ref().and_then(|a| a.bundle_id.clone()),
                        has_icon: app.as_ref().is_some_and(|a| a.bundle_path.is_some()),
                        remote: app.is_some_and(|a| matches!(a.kind, ClientKind::Remote(_))),
                        count,
                    }
                })
                .collect(),
            domains: counts.domains(),
        }
    }

    /// Sends the sidebar to `sink` now and again on every tick it changes,
    /// replacing any previous subscriber.
    pub fn subscribe_sidebar(&self, sink: SidebarSink) {
        *self.platform.sidebar_sink.lock() = Some(sink);
        self.mark_sidebar_dirty();
    }

    /// Makes the sidebar count `store` (the tab the UI shows); a closed
    /// document falls back to the live capture.
    pub fn set_sidebar_store(&self, store: StoreId) {
        self.platform.sidebar_store.store(store, Ordering::Relaxed);
        self.mark_sidebar_dirty();
    }

    fn mark_sidebar_dirty(&self) {
        match self.platform.sidebar_store.load(Ordering::Relaxed) {
            LIVE_STORE => self.store.write().sidebar_mut().mark_dirty(),
            doc => match self.docs.get(doc) {
                Ok(doc) => doc.store.write().sidebar_mut().mark_dirty(),
                Err(_) => {
                    self.platform.sidebar_store.store(LIVE_STORE, Ordering::Relaxed);
                    self.store.write().sidebar_mut().mark_dirty();
                }
            },
        }
    }

    /// Part of the publisher tick.
    pub(crate) fn publish_sidebar(&self) {
        let mut sink = self.platform.sidebar_sink.lock();
        let Some(send) = sink.as_mut() else {
            return;
        };
        let dirty = match self.platform.sidebar_store.load(Ordering::Relaxed) {
            LIVE_STORE => self.store.write().sidebar_mut().take_dirty(),
            doc => match self.docs.get(doc) {
                Ok(doc) => doc.store.write().sidebar_mut().take_dirty(),
                // Its tab closed: back to the live capture.
                Err(_) => {
                    self.platform.sidebar_store.store(LIVE_STORE, Ordering::Relaxed);
                    true
                }
            },
        };
        if !dirty {
            return;
        }
        if !send(self.sidebar()) {
            *sink = None;
        }
    }

    /// The PNG icon of a client app seen in some flow, by its name.
    /// Rendered on first use (blocking), then cached.
    pub fn app_icon(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        self.platform.clients.icon(name)
    }

    pub(crate) fn clients(&self) -> &ClientResolver {
        &self.platform.clients
    }

    /// Stops (or resumes) decrypting everything `app` sends. `app` is a
    /// bundle id, or a name for tools without a bundle.
    pub fn set_app_ssl_excluded(&self, app: &str, excluded: bool) -> Result<SslSettings, Error> {
        let app = app.trim().to_owned();
        self.update_ssl(|s| {
            s.exclude_apps.retain(|a| !a.eq_ignore_ascii_case(&app));
            if excluded && !app.is_empty() {
                s.exclude_apps.push(app);
            }
        })
    }

    // ---- CA trust ----

    /// What the trust store says about the CA, plus a real handshake check
    /// through the proxy (if it is listening).
    pub async fn trust_status(self: &Arc<Self>) -> TrustStatus {
        let keychain = self.keychain_trust().await;
        let check = match self.proxy_addr() {
            Some(addr) => crate::trust::handshake_check(addr).await,
            None => Err("the proxy isn't running".into()),
        };
        TrustStatus {
            keychain,
            verified: check.is_ok(),
            detail: check.err(),
        }
    }

    async fn keychain_trust(self: &Arc<Self>) -> KeychainTrust {
        let engine = self.clone();
        let state = tokio::task::spawn_blocking(move || engine.with_ca(|ca| engine.platform.trust.state(ca))).await;
        match state {
            Ok(Ok(TrustState::Trusted)) => KeychainTrust::Trusted,
            Ok(Ok(TrustState::Denied)) => KeychainTrust::Denied,
            Ok(Ok(TrustState::NotTrusted)) | Ok(Err(PlatformError::Os(_))) | Err(_) => KeychainTrust::NotTrusted,
            Ok(Err(_)) => KeychainTrust::Unsupported,
        }
    }

    /// Adds the CA to the login keychain and trusts it as a root. macOS
    /// shows its own password or Touch ID prompt; this waits for it.
    pub async fn install_ca_trust(self: &Arc<Self>) -> Result<TrustStatus, Error> {
        let engine = self.clone();
        tokio::task::spawn_blocking(move || engine.with_ca(|ca| engine.platform.trust.install(ca)))
            .await
            .map_err(|e| Error::Platform(PlatformError::Os(e.to_string())))??;
        Ok(self.trust_status().await)
    }

    /// Replaces the root CA. With `remove_old_trust`, the old CA's trust
    /// setting is removed from the keychain afterwards (macOS prompts); if
    /// that fails the new CA is still in place and the error says so.
    pub async fn regenerate_ca_and_untrust(
        self: &Arc<Self>,
        remove_old_trust: bool,
    ) -> Result<pm_model::CaInfo, Error> {
        let old = remove_old_trust.then(|| {
            self.with_ca(|ca| {
                let copy = ca.pem_path.with_file_name("previous-ca.pem");
                let _ = std::fs::copy(ca.pem_path, &copy);
                (ca.der.to_vec(), ca.sha1_hex.to_owned(), copy)
            })
        });
        let info = self.regenerate_ca()?;
        let Some((der, sha1_hex, pem_path)) = old else {
            return Ok(info);
        };
        let engine = self.clone();
        let removed = tokio::task::spawn_blocking(move || {
            let result = engine.platform.trust.uninstall(CaCert {
                der: &der,
                pem_path: &pem_path,
                sha1_hex: &sha1_hex,
            });
            let _ = std::fs::remove_file(&pem_path);
            result
        })
        .await
        .map_err(|e| PlatformError::Os(e.to_string()));
        match removed {
            Ok(Ok(())) => Ok(info),
            Ok(Err(err)) | Err(err) => Err(Error::Tls(format!(
                "the new CA is in place, but the old one is still trusted: {err}"
            ))),
        }
    }

    /// Removes the CA's trust setting and deletes it from the keychain.
    pub async fn uninstall_ca_trust(self: &Arc<Self>) -> Result<TrustStatus, Error> {
        let engine = self.clone();
        tokio::task::spawn_blocking(move || engine.with_ca(|ca| engine.platform.trust.uninstall(ca)))
            .await
            .map_err(|e| Error::Platform(PlatformError::Os(e.to_string())))??;
        Ok(self.trust_status().await)
    }

    fn with_ca<T>(&self, f: impl FnOnce(CaCert<'_>) -> T) -> T {
        let tls = self.tls();
        let der = tls.ca.cert_der().as_ref();
        let sha1 = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, der);
        let sha1_hex: String = sha1.as_ref().iter().map(|b| format!("{b:02X}")).collect();
        let pem_path = PathBuf::from(&tls.ca.info().cert_path);
        f(CaCert {
            der,
            pem_path: &pem_path,
            sha1_hex: &sha1_hex,
        })
    }

    // ---- System proxy ----

    /// Whether the OS proxy points at the listener. Runs `networksetup`, so
    /// call it off the UI thread.
    pub fn system_proxy_status(&self) -> SystemProxyStatus {
        let error = self.platform.sysproxy_error.lock().clone();
        let Some(addr) = self.proxy_addr() else {
            return SystemProxyStatus {
                error: error.or_else(|| Some("the proxy isn't running".into())),
                supported: cfg!(target_os = "macos"),
                ..Default::default()
            };
        };
        match self.platform.sysproxy.status(&addr.ip().to_string(), addr.port()) {
            Ok(status) => SystemProxyStatus {
                enabled: status.owned && !status.services.is_empty(),
                services: status.services,
                supported: true,
                error,
            },
            Err(PlatformError::Unsupported) => SystemProxyStatus::default(),
            Err(err) => SystemProxyStatus {
                supported: true,
                error: Some(err.to_string()),
                ..Default::default()
            },
        }
    }

    /// Points the OS proxy at the listener, or puts it back. The previous
    /// settings are backed up first, and a guardian process restores them
    /// if this one dies.
    pub fn set_system_proxy(&self, on: bool) -> SystemProxyStatus {
        let result = match (on, self.proxy_addr()) {
            (true, Some(addr)) => self
                .platform
                .sysproxy
                .enable(&addr.ip().to_string(), addr.port())
                .map(|_| {
                    self.spawn_guardian();
                }),
            (true, None) => Err(PlatformError::Os("the proxy isn't running".into())),
            (false, _) => self.platform.sysproxy.restore().map(drop),
        };
        *self.platform.sysproxy_error.lock() = result.err().map(|e| e.to_string());
        self.system_proxy_status()
    }

    /// Puts the OS proxy back if this process changed it. For quit, signal
    /// and panic handlers; safe to call more than once.
    pub fn restore_system_proxy(&self) -> Result<Vec<String>, Error> {
        Ok(self.platform.sysproxy.restore()?)
    }

    /// Restores settings left behind by an earlier run that died without
    /// its guardian. Call once at startup.
    pub fn restore_stale_system_proxy(&self) -> Result<Vec<String>, Error> {
        Ok(self.platform.sysproxy.restore_if_stale()?)
    }

    /// The guardian process: waits for `pid` to exit, then restores the
    /// system proxy backup it left in `data_dir`, if any. No engine is
    /// created.
    pub fn run_system_proxy_guardian(data_dir: &Path, pid: u32) -> Result<Vec<String>, Error> {
        Ok(SystemProxy::system(data_dir.join(SYSPROXY_BACKUP_FILE)).guard(pid)?)
    }

    fn spawn_guardian(&self) {
        let Some(exe) = &self.platform.guardian_exe else {
            return;
        };
        if self.platform.guardian_spawned.swap(true, Ordering::SeqCst) {
            return;
        }
        let pid = std::process::id().to_string();
        let mut command = std::process::Command::new(exe);
        command
            .args(["--sysproxy-guardian", &pid, "--data-dir"])
            .arg(&self.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        // Its own process group, so a Ctrl-C to the app's terminal doesn't
        // take the guardian down with it.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        if let Err(err) = command.spawn() {
            self.platform.guardian_spawned.store(false, Ordering::SeqCst);
            eprintln!("proxymate: can't start the system proxy guardian: {err}");
        }
    }

    // ---- Terminal ----

    /// Shell `export` lines that send a terminal's traffic through the
    /// proxy and make common runtimes trust the CA.
    pub fn terminal_env(&self) -> String {
        let proxy = match self.proxy_addr() {
            Some(addr) => format!("http://{addr}"),
            None => "http://127.0.0.1:9090".into(),
        };
        let ca = self.ca_info().cert_path;
        // These replace a tool's roots rather than add to them, so they get
        // the OS roots too; Node's variable adds to its own.
        let bundle = match self.write_ca_bundle(Path::new(&ca)) {
            Ok(path) => path.display().to_string(),
            Err(err) => {
                eprintln!("proxymate: can't write the CA bundle ({err}); using the CA alone");
                ca.clone()
            }
        };
        let mut lines = Vec::new();
        for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            lines.push(format!("export {var}={proxy}"));
        }
        lines.push(format!("export NODE_EXTRA_CA_CERTS={}", shell_quote(&ca)));
        for var in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"] {
            lines.push(format!("export {var}={}", shell_quote(&bundle)));
        }
        lines.join("\n") + "\n"
    }

    /// Writes `ca/bundle.pem` beside the CA: the OS's trusted roots, then
    /// our CA. Rewritten each time, so it follows a regenerated CA.
    fn write_ca_bundle(&self, ca_pem: &Path) -> std::io::Result<PathBuf> {
        use base64::Engine as _;
        let ours = std::fs::read_to_string(ca_pem)?;
        let roots = rustls_native_certs::load_native_certs();
        let mut out = String::with_capacity(roots.certs.len() * 2048 + ours.len());
        for cert in &roots.certs {
            out.push_str("-----BEGIN CERTIFICATE-----\n");
            let b64 = base64::engine::general_purpose::STANDARD.encode(cert.as_ref());
            for line in b64.as_bytes().chunks(64) {
                out.push_str(std::str::from_utf8(line).unwrap_or_default());
                out.push('\n');
            }
            out.push_str("-----END CERTIFICATE-----\n");
        }
        out.push_str(&ours);
        let path = ca_pem.with_file_name(CA_BUNDLE_FILE);
        let tmp = path.with_extension("pem.tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }
}

/// The OS roots plus our CA, for `SSL_CERT_FILE` and friends.
const CA_BUNDLE_FILE: &str = "bundle.pem";

/// Single-quotes `s` for POSIX shells.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_env_quotes_the_ca_path() {
        let dir = std::env::temp_dir()
            .join(format!("pm-env-{}", uuid::Uuid::new_v4()))
            .join("Application Support");
        let engine = Engine::with_config(crate::EngineConfig {
            data_dir: Some(dir.clone()),
            ..Default::default()
        });
        let env = engine.terminal_env();
        assert!(env.contains("export HTTPS_PROXY=http://127.0.0.1:9090\n"), "{env}");
        assert!(env.contains("export http_proxy=http://127.0.0.1:9090\n"));
        let pem = engine.ca_info().cert_path;
        assert!(env.contains(&format!("export NODE_EXTRA_CA_CERTS='{pem}'\n")), "{env}");
        // The bundle holds the OS roots and ends with our CA.
        let bundle = std::path::Path::new(&pem).with_file_name(CA_BUNDLE_FILE);
        assert!(
            env.contains(&format!("export SSL_CERT_FILE='{}'\n", bundle.display())),
            "{env}"
        );
        let text = std::fs::read_to_string(&bundle).unwrap();
        assert!(text.ends_with(&std::fs::read_to_string(&pem).unwrap()));
        #[cfg(target_os = "macos")]
        assert!(text.matches("BEGIN CERTIFICATE").count() > 50);
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
