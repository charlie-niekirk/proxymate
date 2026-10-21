//! Which app each loopback connection comes from.
//!
//! Every accepted connection gets a [`PeerClient`] and a background task
//! that resolves its peer port to a pid, and the pid to an app, off the
//! forwarding path. Flows that start before the answer is in are patched
//! with a `ClientResolved` event once it is.
//!
//! Port → pid is a full socket scan (a few ms). Scans are coalesced: one
//! runs at a time, and every connection accepted before it started shares
//! its result. Pid → app is cached.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pm_model::{FlowDetail, FlowId, FlowSummary};
use pm_platform::procinfo::{AppInfo, ProcessInfo};

use crate::Engine;
use crate::ingest::FlowEvent;
use crate::recorder::FlowRecorder;

/// Pids are reused eventually; forget their apps after this long.
const APP_TTL: Duration = Duration::from_secs(600);
const APP_CACHE_MAX: usize = 4_096;
/// Icon size in points; AppKit renders it at the screen's scale (64 px on
/// Retina), plenty for the 14-16 px the UI draws.
const ICON_PT: u32 = 32;

/// The app on the other end of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientApp {
    /// Interned: every flow of the app shares it.
    pub name: Arc<str>,
    pub bundle_id: Option<String>,
    pub bundle_path: Option<PathBuf>,
}

/// A pid's app (or `None`: not one we can name) and when it was looked up.
type PidEntry = (Option<Arc<ClientApp>>, Instant);
/// A rendered PNG, or `None` if the app has no icon.
type Icon = Option<Arc<Vec<u8>>>;

pub(crate) struct ClientResolver {
    platform: Arc<dyn ProcessInfo>,
    scan: tokio::sync::Mutex<Scan>,
    apps: Mutex<HashMap<u32, PidEntry>>,
    /// Every app seen, by name, for the sidebar and icons.
    known: Mutex<HashMap<Arc<str>, Arc<ClientApp>>>,
    icons: Mutex<HashMap<Arc<str>, Icon>>,
}

#[derive(Default)]
struct Scan {
    started: Option<Instant>,
    proxy_port: u16,
    peers: HashMap<u16, u32>,
}

impl ClientResolver {
    pub fn new(platform: Box<dyn ProcessInfo>) -> ClientResolver {
        ClientResolver {
            platform: Arc::from(platform),
            scan: tokio::sync::Mutex::new(Scan::default()),
            apps: Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
            icons: Mutex::new(HashMap::new()),
        }
    }

    /// The app behind a connection from `peer` to our `proxy_port`,
    /// accepted at `accepted`. Only loopback peers can be resolved.
    pub async fn resolve(&self, peer: SocketAddr, proxy_port: u16, accepted: Instant) -> Option<Arc<ClientApp>> {
        if !peer.ip().is_loopback() && !peer.ip().to_canonical().is_loopback() {
            return None;
        }
        let pid = self.pid_for(peer.port(), proxy_port, accepted).await?;
        self.app_for(pid).await
    }

    async fn pid_for(&self, port: u16, proxy_port: u16, accepted: Instant) -> Option<u32> {
        let mut scan = self.scan.lock().await;
        let fresh = scan.proxy_port == proxy_port && scan.started.is_some_and(|t| t >= accepted);
        if !fresh {
            let started = Instant::now();
            let platform = self.platform.clone();
            let peers = tokio::task::spawn_blocking(move || platform.loopback_peers(proxy_port))
                .await
                .ok()?
                .ok()?;
            *scan = Scan {
                started: Some(started),
                proxy_port,
                peers,
            };
        }
        scan.peers.get(&port).copied()
    }

    async fn app_for(&self, pid: u32) -> Option<Arc<ClientApp>> {
        if let Some((app, at)) = self.apps.lock().get(&pid)
            && at.elapsed() < APP_TTL
        {
            return app.clone();
        }
        let platform = self.platform.clone();
        let info = tokio::task::spawn_blocking(move || platform.app_for_pid(pid))
            .await
            .ok()?;
        let app = info.map(|info| self.intern(info));
        let mut apps = self.apps.lock();
        if apps.len() >= APP_CACHE_MAX {
            apps.clear();
        }
        apps.insert(pid, (app.clone(), Instant::now()));
        app
    }

    fn intern(&self, info: AppInfo) -> Arc<ClientApp> {
        let mut known = self.known.lock();
        if let Some(app) = known.get(info.name.as_str())
            && app.bundle_path == info.bundle_path
        {
            return app.clone();
        }
        let app = Arc::new(ClientApp {
            name: info.name.as_str().into(),
            bundle_id: info.bundle_id,
            bundle_path: info.bundle_path,
        });
        known.insert(app.name.clone(), app.clone());
        app
    }

    pub fn known(&self, name: &str) -> Option<Arc<ClientApp>> {
        self.known.lock().get(name).cloned()
    }

    /// The app's icon as a PNG, rendered once and cached. Blocking.
    pub fn icon(&self, name: &str) -> Icon {
        if let Some(icon) = self.icons.lock().get(name) {
            return icon.clone();
        }
        let app = self.known(name)?;
        let icon = app
            .bundle_path
            .as_deref()
            .and_then(|path| self.platform.icon_png(path, ICON_PT))
            .map(Arc::new);
        self.icons.lock().insert(app.name.clone(), icon.clone());
        icon
    }
}

/// The client of one accepted connection, resolved in the background.
pub(crate) struct PeerClient {
    state: Mutex<PeerState>,
    resolved: tokio::sync::Notify,
}

enum PeerState {
    /// Flows started before the app was known, to patch once it is.
    Pending(Vec<FlowId>),
    Resolved(Option<Arc<ClientApp>>),
}

impl PeerClient {
    /// A client that is never resolved (non-loopback peers until M9).
    pub fn unknown() -> Arc<PeerClient> {
        Arc::new(PeerClient {
            state: Mutex::new(PeerState::Resolved(None)),
            resolved: tokio::sync::Notify::new(),
        })
    }

    /// Starts resolving `peer` on the current runtime.
    pub fn spawn(engine: &Arc<Engine>, peer: SocketAddr, proxy_port: u16) -> Arc<PeerClient> {
        let client = Arc::new(PeerClient {
            state: Mutex::new(PeerState::Pending(Vec::new())),
            resolved: tokio::sync::Notify::new(),
        });
        let accepted = Instant::now();
        let weak = Arc::downgrade(engine);
        let task_client = client.clone();
        tokio::spawn(async move {
            let Some(engine) = weak.upgrade() else { return };
            let app = engine.clients().resolve(peer, proxy_port, accepted).await;
            task_client.finish(&engine, app);
        });
        client
    }

    fn finish(&self, engine: &Engine, app: Option<Arc<ClientApp>>) {
        let pending = {
            let mut state = self.state.lock();
            match std::mem::replace(&mut *state, PeerState::Resolved(app.clone())) {
                PeerState::Pending(ids) => ids,
                PeerState::Resolved(_) => Vec::new(),
            }
        };
        if let Some(app) = app {
            for id in pending {
                engine.ingest().send(FlowEvent::ClientResolved {
                    id,
                    client: app.name.clone(),
                });
            }
        }
        self.resolved.notify_waiters();
    }

    /// Starts recording a flow from this client: with its app if known,
    /// else patched in when it is.
    pub fn begin_flow(
        &self,
        engine: &Arc<Engine>,
        mut summary: FlowSummary,
        detail: FlowDetail,
    ) -> Option<FlowRecorder> {
        // Held across begin_flow so `finish` can't miss the new id.
        let mut state = self.state.lock();
        match &mut *state {
            PeerState::Resolved(app) => {
                summary.client = app.as_ref().map(|a| a.name.clone());
                engine.begin_flow(summary, detail)
            }
            PeerState::Pending(ids) => {
                let recorder = engine.begin_flow(summary, detail)?;
                ids.push(recorder.id());
                Some(recorder)
            }
        }
    }

    /// The app, waiting up to `timeout` for it to be resolved.
    pub async fn app(&self, timeout: Duration) -> Option<Arc<ClientApp>> {
        let notified = self.resolved.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let PeerState::Resolved(app) = &*self.state.lock() {
            return app.clone();
        }
        let _ = tokio::time::timeout(timeout, notified).await;
        match &*self.state.lock() {
            PeerState::Resolved(app) => app.clone(),
            PeerState::Pending(_) => None,
        }
    }
}
