//! Storage limits and proxy settings (M10): rows of `app.sqlite`'s
//! settings table, in memory without a data dir. Also where the limits are
//! enforced: after every ingest batch the live store evicts its oldest
//! flows past the flow cap or the disk cap, and the segment files they
//! alone used are deleted.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::{Mutex, RwLock};
use pm_model::{FlowId, ProxySettings, StorageSettings, StorageStatus};

use crate::notify::Notifier;
use crate::proxy::ProxyConfig;
use crate::rules::db::AppDb;
use crate::store::Eviction;
use crate::{Engine, Error};

const STORAGE_KEY: &str = "storage";
const PROXY_KEY: &str = "proxy";

/// Receives storage status updates. Returning `false` unsubscribes.
pub type StorageSink = Box<dyn FnMut(StorageStatus) -> bool + Send>;

pub(crate) struct Settings {
    db: Mutex<AppDb>,
    storage: RwLock<StorageSettings>,
    proxy: RwLock<ProxySettings>,
    status: Notifier<StorageStatus>,
    last_status: Mutex<Option<StorageStatus>>,
    /// The disk cap last asked eviction to go through this id; not asked
    /// again until the segments say a newer one is needed.
    disk_through: AtomicU32,
}

impl Settings {
    pub fn load(dir: Option<&Path>) -> Settings {
        let db = dir
            .map(AppDb::open)
            .unwrap_or_else(AppDb::in_memory)
            .or_else(|err| {
                eprintln!("proxymate: can't open the app database ({err}); settings won't be saved");
                AppDb::in_memory()
            })
            .expect("in-memory SQLite");
        let storage = db.setting(STORAGE_KEY).unwrap_or_default();
        let proxy = db.setting(PROXY_KEY).unwrap_or_default();
        Settings {
            db: Mutex::new(db),
            storage: RwLock::new(storage),
            proxy: RwLock::new(proxy),
            status: Notifier::new(),
            last_status: Mutex::new(None),
            disk_through: AtomicU32::new(0),
        }
    }
}

fn check_storage(s: &StorageSettings) -> Result<(), Error> {
    if s.max_flows != 0 && s.max_flows < 100 {
        return Err(Error::Invalid("keep at least 100 flows (or 0 for no limit)".into()));
    }
    if s.max_disk_mb != 0 && s.max_disk_mb < 128 {
        return Err(Error::Invalid(
            "allow at least 128 MB of disk (or 0 for no limit)".into(),
        ));
    }
    if s.max_ws_messages != 0 && s.max_ws_messages < 100 {
        return Err(Error::Invalid(
            "keep at least 100 WebSocket messages per connection (or 0 for no limit)".into(),
        ));
    }
    Ok(())
}

fn check_proxy(s: &ProxySettings) -> Result<(), Error> {
    if s.port < 1024 {
        return Err(Error::Invalid(
            "pick a port from 1024 up: lower ones need administrator rights".into(),
        ));
    }
    if s.connect_timeout_secs == 0 || s.connect_timeout_secs > 300 {
        return Err(Error::Invalid("the connect timeout must be 1–300 seconds".into()));
    }
    if s.pool_max_idle_per_host > 1024 {
        return Err(Error::Invalid("keep at most 1024 idle connections per host".into()));
    }
    Ok(())
}

impl Engine {
    // ---- Storage ----

    pub fn storage_settings(&self) -> StorageSettings {
        *self.settings.storage.read()
    }

    /// Saves new limits and applies them straight away (evicting if the
    /// capture is over them now).
    pub fn set_storage_settings(&self, settings: StorageSettings) -> Result<StorageSettings, Error> {
        check_storage(&settings)?;
        self.settings
            .db
            .lock()
            .save_setting(STORAGE_KEY, &settings)
            .map_err(|e| Error::Invalid(format!("can't save the settings: {e}")))?;
        *self.settings.storage.write() = settings;
        self.settings.disk_through.store(0, Ordering::Relaxed);
        // Through the ingest thread, which owns eviction.
        self.ingest().flush();
        Ok(settings)
    }

    pub fn storage_status(&self) -> StorageStatus {
        let store = self.store.read();
        let segments = &self.details().segments;
        StorageStatus {
            flows: store.total(),
            pinned: store.pinned(),
            disk_bytes: segments.disk_bytes(),
            evicted: store.evicted(),
            error: segments.error(),
            dir: segments.dir_path().display().to_string(),
            sent_bytes: store.traffic().0,
            received_bytes: store.traffic().1,
        }
    }

    /// Sends the storage status now and whenever it changes (at most once
    /// per publisher tick).
    pub fn subscribe_storage(&self, sink: StorageSink) {
        self.settings.status.subscribe(sink);
    }

    /// Part of the publisher tick.
    pub(crate) fn publish_storage(&self) {
        let status = self.storage_status();
        {
            let mut last = self.settings.last_status.lock();
            if last.as_ref() != Some(&status) {
                *last = Some(status.clone());
                self.settings.status.changed();
            }
        }
        self.settings.status.publish(|| status);
    }

    /// Pins (or unpins) live flows: pinned flows are never evicted.
    pub fn pin_flows(&self, ids: Vec<FlowId>, pinned: bool) {
        self.ingest().send(crate::ingest::FlowEvent::Pin { ids, pinned });
        self.ingest().flush();
    }

    /// Evicts past the limits; runs on the ingest thread with the store
    /// write-locked, after each batch.
    pub(crate) fn evict_locked(&self, store: &mut crate::store::Store) -> Option<Eviction> {
        let limits = *self.settings.storage.read();
        let segments = &self.details().segments;
        let mut through = None;
        if limits.max_disk_mb > 0 {
            let cap = u64::from(limits.max_disk_mb) * 1024 * 1024;
            if segments.disk_bytes() > cap {
                // Down to 90%, so it doesn't run again for a while.
                let wanted = segments.owner_to_free(cap / 10 * 9);
                if let Some(id) = wanted
                    && id >= self.settings.disk_through.load(Ordering::Relaxed)
                {
                    self.settings
                        .disk_through
                        .store(id.saturating_add(1), Ordering::Relaxed);
                    through = Some(id);
                }
            }
        }
        store.evict(limits.max_flows, through)
    }

    /// Forgets what an eviction dropped: details, message logs, segment
    /// files nothing kept uses, and strings no flow shares any more.
    pub(crate) fn after_eviction(&self, eviction: Eviction) {
        self.details()
            .evict(&eviction.evicted, &eviction.kept, eviction.through);
        self.interned().lock().retain(|s| Arc::strong_count(s) > 1);
    }

    // ---- Proxy ----

    pub fn proxy_settings(&self) -> ProxySettings {
        *self.settings.proxy.read()
    }

    /// The listener config the settings ask for: loopback, the chosen port
    /// and the nine after it.
    pub fn configured_proxy(&self) -> ProxyConfig {
        let port = self.settings.proxy.read().port;
        ProxyConfig {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ports: port..=port.saturating_add(9),
        }
    }

    /// Saves new proxy settings. A port or upstream pool change restarts
    /// the listener (on the same interfaces); if the new port can't be
    /// bound, the error says why and the old listener stays.
    pub async fn set_proxy_settings(self: &Arc<Self>, settings: ProxySettings) -> Result<ProxySettings, Error> {
        check_proxy(&settings)?;
        let old = self.proxy_settings();
        if old == settings {
            return Ok(settings);
        }
        *self.settings.proxy.write() = settings;
        if let Err(err) = self.restart_proxy(old.port != settings.port).await {
            *self.settings.proxy.write() = old;
            let _ = self.restart_proxy(true).await;
            return Err(err);
        }
        self.settings
            .db
            .lock()
            .save_setting(PROXY_KEY, &settings)
            .map_err(|e| Error::Invalid(format!("can't save the settings: {e}")))?;
        Ok(settings)
    }
}
