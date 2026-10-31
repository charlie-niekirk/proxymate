//! The `Engine` facade. Tauri commands, MCP tools and the CLI call only this,
//! so the UI and agents always see the same behaviour.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use pm_model::{
    BodyPart, CaInfo, FilterSpec, FlowDetail, FlowId, FlowState, FlowSummary, LIVE_STORE, ProxyStatus, RowsPage,
    SortSpec, SsePage, SslSettings, StoreId, ViewDelta, ViewId, ViewInfo, WsPage, WsQuery,
};
use rustls::pki_types::CertificateDer;

use crate::Error;
use crate::devices::{self, DeviceRegistry};
use crate::documents::Documents;
use crate::ingest::{self, FlowEvent, IngestTx};
use crate::integration::Platform;
use crate::io::{decode, sse};
use crate::mcp::McpState;
use crate::protos::Protos;
use crate::proxy::{self, ProxyConfig, ProxyHandle};
use crate::recorder::{FlowRecorder, is_event_stream};
use crate::rules::Rules;
use crate::scripts::Scripts;
use crate::settings::Settings;
use crate::store::{BlobBackend, DeepJob, DetailStore, SegmentStore, Store, VIEW_STORE_SHIFT, page_events, sse_event};
use crate::tls::{CertAuthority, SslPolicy, TlsContext};

const SSL_SETTINGS_FILE: &str = "ssl.json";
/// Secrets when the keychain isn't used (`EngineConfig::use_keychain`).
const SECRETS_FILE: &str = "secrets.json";

/// Deltas are published at 20 Hz.
pub const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// A deep search hands the rayon pool this many flows at a time, then
/// reports what it found, so matches stream into the view.
const DEEP_BATCH: usize = 512;
/// An idle deep search (every candidate checked) looks for new ones this often.
const DEEP_IDLE: Duration = Duration::from_millis(40);

/// Receives a view's deltas. Returning `false` unsubscribes (e.g. the
/// webview's channel was closed).
pub type DeltaSink = Box<dyn FnMut(ViewDelta) -> bool + Send>;

#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    /// Root for live segment files (`<data_dir>/live/<uuid>/`), the CA
    /// (`<data_dir>/ca/`), `ssl.json` and `app.sqlite` (rules, the device
    /// allow-list and the MCP settings, token and audit log; in memory
    /// without a data dir). Defaults to a `proxymate`
    /// directory under the system temp dir.
    pub data_dir: Option<PathBuf>,
    /// DER certificates trusted for upstream TLS on top of the OS trust
    /// store, e.g. a test CA or a corporate CA the OS doesn't know.
    pub extra_upstream_roots: Vec<Vec<u8>>,
    /// The executable that runs `--sysproxy-guardian` (the app itself).
    /// Without it the system proxy is only restored by this process.
    pub guardian_exe: Option<PathBuf>,
    /// Keep upstream proxy and PKCS#12 passwords in the OS keychain (the
    /// app). Otherwise they go in a 0600 `secrets.json` in the data dir,
    /// or in memory without one, so tests never touch the keychain.
    pub use_keychain: bool,
}

/// A body as returned to the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyBytes {
    pub bytes: Vec<u8>,
    /// The `Content-Encoding` that was undone, if decoding was asked for
    /// and the body was encoded.
    pub decoded_from: Option<String>,
    /// True if only the first `max` bytes were asked for and there are more.
    pub cut: bool,
}

pub struct Engine {
    pub(crate) store: RwLock<Store>,
    sinks: Mutex<HashMap<ViewId, DeltaSink>>,
    ingest: IngestTx,
    details: DetailStore,
    recording: AtomicBool,
    proxy: Mutex<ProxyState>,
    interned: Mutex<HashSet<Arc<str>>>,
    pub(crate) root: PathBuf,
    extra_roots: Vec<CertificateDer<'static>>,
    tls: RwLock<Arc<TlsContext>>,
    /// Write-locked across a read-modify-write of the settings.
    ssl: RwLock<Arc<SslPolicy>>,
    pub(crate) platform: Platform,
    pub(crate) rules: Rules,
    pub(crate) scripts: Scripts,
    pub(crate) mcp: McpState,
    /// Storage limits and proxy settings.
    pub(crate) settings: Settings,
    /// Opened sessions and HAR imports.
    pub(crate) docs: Documents,
    /// Protobuf schemas, "Decode as…" mappings and decoded bodies.
    pub(crate) protos: Protos,
    /// `Engine::send`'s client, made on first use.
    pub(crate) sender: OnceLock<proxy::Sender>,
    /// Header columns and running repeats (M17).
    pub(crate) workflow: crate::workflow::Workflow,
    /// Upstream proxies, DNS overrides, client certificates and the
    /// reverse proxy listeners (M18).
    pub(crate) network: crate::network::Network,
    /// The plain-HTTP upstream pool every listener and `Engine::send`
    /// share; rebuilt when the proxy or network settings change.
    pool: arc_swap::ArcSwapOption<proxy::UpstreamClient>,
    /// Held while the listener is moved (port change, LAN access), so two
    /// moves never both bind.
    restart: tokio::sync::Mutex<()>,
}

#[derive(Default)]
enum ProxyState {
    #[default]
    Stopped,
    Listening(ProxyHandle),
    Failed(String),
}

impl Engine {
    /// An engine with the default config. See [`Engine::with_config`].
    pub fn new() -> Arc<Engine> {
        Engine::with_config(EngineConfig::default())
    }

    /// Creates an engine and starts its ingest thread and 20 Hz delta
    /// publisher thread, which stop when the last `Arc<Engine>` is dropped.
    pub fn with_config(config: EngineConfig) -> Arc<Engine> {
        // Without a data directory (tests, throwaway engines) rules live
        // in memory, so engines never share them through the temp dir.
        let scripts = Scripts::load(config.data_dir.as_deref());
        let rules = Rules::load(config.data_dir.as_deref(), &scripts.map());
        let mcp = McpState::load(config.data_dir.as_deref());
        let devices = DeviceRegistry::load(config.data_dir.as_deref(), devices::HOLD);
        let app_settings = Settings::load(config.data_dir.as_deref());
        let protos = Protos::load(config.data_dir.as_deref());
        let secrets: Box<dyn pm_platform::secrets::SecretStore> = match &config.data_dir {
            Some(dir) if config.use_keychain => pm_platform::secrets::system(&dir.join(SECRETS_FILE)),
            Some(dir) => Box::new(pm_platform::secrets::FileSecrets::new(dir.join(SECRETS_FILE))),
            None => Box::new(pm_platform::secrets::MemorySecrets::default()),
        };
        let network = crate::network::Network::new(secrets, &app_settings.network_row());
        let root = config
            .data_dir
            .unwrap_or_else(|| std::env::temp_dir().join("proxymate"));
        let extra_roots: Vec<CertificateDer<'static>> = config
            .extra_upstream_roots
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let ca = CertAuthority::load_or_create(&root.join("ca")).unwrap_or_else(|err| {
            // An unreadable CA directory must not stop the proxy; HTTPS then
            // uses a throwaway CA that no client trusts yet.
            eprintln!("proxymate: can't load the CA from {}: {err}", root.join("ca").display());
            let dir = std::env::temp_dir().join(format!("proxymate-ca-{}", std::process::id()));
            CertAuthority::create(&dir).expect("create a throwaway CA")
        });
        let tls = TlsContext::new(ca, extra_roots.clone()).expect("TLS configuration");
        let settings = SslPolicy::load(&root.join(SSL_SETTINGS_FILE)).unwrap_or_else(|err| {
            eprintln!("proxymate: {err}; using the default SSL settings");
            SslSettings::default()
        });
        let policy = SslPolicy::compile(settings)
            .or_else(|err| {
                eprintln!("proxymate: {err}; using the default SSL settings");
                SslPolicy::compile(SslSettings::default())
            })
            .expect("default SSL settings compile");
        let (tx, rx) = std::sync::mpsc::channel();
        let engine = Arc::new(Engine {
            store: RwLock::new(Store::default()),
            sinks: Mutex::new(HashMap::new()),
            ingest: IngestTx::new(tx),
            details: DetailStore::new(SegmentStore::new(&root)),
            recording: AtomicBool::new(true),
            proxy: Mutex::new(ProxyState::Stopped),
            interned: Mutex::new(HashSet::new()),
            platform: Platform::new(&root, config.guardian_exe, devices),
            root,
            extra_roots,
            tls: RwLock::new(Arc::new(tls)),
            ssl: RwLock::new(Arc::new(policy)),
            rules,
            scripts,
            mcp,
            settings: app_settings,
            docs: Documents::default(),
            protos,
            sender: OnceLock::new(),
            workflow: Default::default(),
            network,
            pool: Default::default(),
            restart: tokio::sync::Mutex::new(()),
        });
        let weak = Arc::downgrade(&engine);
        std::thread::Builder::new()
            .name("pm-ingest".into())
            .spawn(move || ingest::ingest_loop(rx, weak))
            .expect("spawn ingest thread");
        let weak = Arc::downgrade(&engine);
        std::thread::Builder::new()
            .name("pm-publisher".into())
            .spawn(move || publisher_loop(weak))
            .expect("spawn publisher thread");
        engine
    }

    /// Number of flows in the live store.
    pub fn total(&self) -> u32 {
        self.store.read().total()
    }

    /// Appends new flows (synthetic data, benches) and waits until views
    /// include them. Ids are assigned here; whatever `id` the summaries
    /// carry is ignored.
    pub fn insert_flows(&self, batch: Vec<FlowSummary>) -> Range<FlowId> {
        let ids = self.ingest.insert(batch);
        self.ingest.flush();
        ids
    }

    /// Replaces existing flows (matched by id) and waits until views see it.
    pub fn update_flows(&self, batch: Vec<FlowSummary>) {
        self.ingest.send(FlowEvent::Update(batch));
        self.ingest.flush();
    }

    /// Blocks until every flow event emitted so far is in the store.
    pub fn flush(&self) {
        self.ingest.flush();
    }

    /// Starts recording a flow. Returns `None` while recording is paused,
    /// in which case the caller forwards without recording.
    pub fn begin_flow(self: &Arc<Self>, summary: FlowSummary, detail: FlowDetail) -> Option<FlowRecorder> {
        if !self.is_recording() {
            return None;
        }
        Some(self.record_flow(summary, detail))
    }

    /// Starts recording a flow even while recording is paused (a request
    /// Proxymate sends itself).
    pub(crate) fn record_flow(self: &Arc<Self>, mut summary: FlowSummary, detail: FlowDetail) -> FlowRecorder {
        summary.method = self.intern(&summary.method);
        summary.host = self.intern(&summary.host);
        if let Some(client) = &summary.client {
            summary.client = Some(self.intern(client));
        }
        let id = self.ingest.start(summary);
        FlowRecorder::new(id, self, detail)
    }

    pub fn get_flow(&self, id: FlowId) -> Option<FlowSummary> {
        self.store.read().get(id).cloned()
    }

    /// A flow's summary in any store (`LIVE_STORE` or an opened document).
    pub fn get_flow_in(&self, store: StoreId, id: FlowId) -> Option<FlowSummary> {
        match store {
            LIVE_STORE => self.get_flow(id),
            _ => self.docs.get(store).ok()?.store.read().get(id).cloned(),
        }
    }

    /// Heads and body metadata of a flow, live or finished.
    pub fn flow_detail(&self, id: FlowId) -> Result<Option<FlowDetail>, Error> {
        self.flow_detail_in(LIVE_STORE, id)
    }

    /// [`Engine::flow_detail`] in any store.
    pub fn flow_detail_in(&self, store: StoreId, id: FlowId) -> Result<Option<FlowDetail>, Error> {
        self.with_backend(store, |backend| Ok(backend.detail(id)?))
    }

    /// The captured bytes of one body. With `decode`, a content-encoded
    /// body (gzip, br, deflate, zstd) is decoded first.
    pub fn body(&self, id: FlowId, part: BodyPart, decode: bool) -> Result<BodyBytes, Error> {
        self.body_head(id, part, decode, None)
    }

    /// Like [`Engine::body`], but at most the first `max` bytes (decoded,
    /// with `decode`), so a huge body can be previewed cheaply.
    pub fn body_head(&self, id: FlowId, part: BodyPart, decode: bool, max: Option<usize>) -> Result<BodyBytes, Error> {
        self.body_head_in(LIVE_STORE, id, part, decode, max)
    }

    /// [`Engine::body_head`] in any store.
    pub fn body_head_in(
        &self,
        store: StoreId,
        id: FlowId,
        part: BodyPart,
        decode: bool,
        max: Option<usize>,
    ) -> Result<BodyBytes, Error> {
        self.with_backend(store, |backend| body_head(backend, id, part, decode, max))
    }

    /// Runs `f` on the flow summaries of `store`, read-locked.
    pub(crate) fn read_store<R>(&self, store: StoreId, f: impl FnOnce(&Store) -> R) -> Result<R, Error> {
        match store {
            LIVE_STORE => Ok(f(&self.store.read())),
            _ => Ok(f(&self.docs.get(store)?.store.read())),
        }
    }

    /// Runs `f` with the details and bodies of `store`.
    pub(crate) fn with_backend<R>(
        &self,
        store: StoreId,
        f: impl FnOnce(&dyn BlobBackend) -> Result<R, Error>,
    ) -> Result<R, Error> {
        match store {
            LIVE_STORE => f(&self.details),
            _ => f(&self.docs.get(store)?.backend),
        }
    }

    /// Writes one body (decoded, with `decode`) to `path`.
    pub fn save_body(&self, id: FlowId, part: BodyPart, decode: bool, path: &std::path::Path) -> Result<u64, Error> {
        self.save_body_in(LIVE_STORE, id, part, decode, path)
    }

    /// [`Engine::save_body`] in any store.
    pub fn save_body_in(
        &self,
        store: StoreId,
        id: FlowId,
        part: BodyPart,
        decode: bool,
        path: &std::path::Path,
    ) -> Result<u64, Error> {
        let body = self.body_head_in(store, id, part, decode, None)?;
        std::fs::write(path, &body.bytes)?;
        Ok(body.bytes.len() as u64)
    }

    /// A window of a WebSocket flow's messages, filtered by `query`. A flow
    /// that never became a WebSocket (the upgrade was refused) has none.
    pub fn websocket_messages(&self, id: FlowId, query: &WsQuery, start: u32, count: u32) -> Result<WsPage, Error> {
        self.websocket_messages_in(LIVE_STORE, id, query, start, count)
    }

    /// [`Engine::websocket_messages`] in any store.
    pub fn websocket_messages_in(
        &self,
        store: StoreId,
        id: FlowId,
        query: &WsQuery,
        start: u32,
        count: u32,
    ) -> Result<WsPage, Error> {
        let log = self.with_backend(store, |backend| match backend.ws(id)? {
            Some(log) => Ok(Some(log.page(backend.segments(), query, start, count)?)),
            None => Ok(None),
        })?;
        match log {
            Some(page) => Ok(page),
            None if self.get_flow_in(store, id).is_some() => Ok(WsPage {
                total: 0,
                len: 0,
                start,
                messages: Vec::new(),
                open: false,
                close: None,
                dropped: 0,
            }),
            None => Err(Error::UnknownFlow(id)),
        }
    }

    /// The recorded payload of one WebSocket message (at most 1 MB).
    pub fn websocket_payload(&self, id: FlowId, index: u32) -> Result<Vec<u8>, Error> {
        self.websocket_payload_in(LIVE_STORE, id, index)
    }

    /// [`Engine::websocket_payload`] in any store.
    pub fn websocket_payload_in(&self, store: StoreId, id: FlowId, index: u32) -> Result<Vec<u8>, Error> {
        self.with_backend(store, |backend| {
            let log = backend.ws(id)?.ok_or(Error::UnknownFlow(id))?;
            log.payload(backend.segments(), index)?
                .ok_or(Error::UnknownMessage(id, index))
        })
    }

    /// A window of the events of a `text/event-stream` response matching
    /// `text`. Uncompressed streams are parsed live as they arrive; a
    /// compressed one is decoded and parsed once it is complete, without
    /// per-event times.
    pub fn sse_events(&self, id: FlowId, text: &str, start: u32, count: u32) -> Result<SsePage, Error> {
        self.sse_events_in(LIVE_STORE, id, text, start, count)
    }

    /// [`Engine::sse_events`] in any store.
    pub fn sse_events_in(
        &self,
        store: StoreId,
        id: FlowId,
        text: &str,
        start: u32,
        count: u32,
    ) -> Result<SsePage, Error> {
        if let Some(log) = self.with_backend(store, |backend| Ok(backend.sse(id)?))? {
            return Ok(log.page(text, start, count));
        }
        let detail = self.flow_detail_in(store, id)?.ok_or(Error::UnknownFlow(id))?;
        let open = self
            .get_flow_in(store, id)
            .is_some_and(|f| f.state == FlowState::Active);
        let is_stream = detail.response.as_ref().is_some_and(|r| is_event_stream(&r.headers));
        let events = match is_stream && !open {
            true => match self.body_head_in(store, id, BodyPart::Response, true, None) {
                Ok(body) => sse::parse_all(&body.bytes)
                    .into_iter()
                    .enumerate()
                    .map(|(i, e)| sse_event(i as u32, None, e))
                    .collect(),
                Err(_) => Vec::new(),
            },
            false => Vec::new(),
        };
        Ok(page_events(&events, text, start, count, open))
    }

    /// Drops every flow and deletes their bodies. Flows still in flight are
    /// forwarded to the end but no longer recorded. Ids are never reused.
    pub fn clear(&self) {
        let base = self.ingest.clear();
        self.details.clear(base);
        self.ingest.flush();
    }

    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::Relaxed);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    /// Binds the proxy listener (the first free port in `config.ports`) and
    /// serves it on the current tokio runtime. Failure is also kept for
    /// [`Engine::proxy_status`]. A loopback `config.ip` becomes every
    /// interface if LAN access was left on.
    pub async fn start_proxy(self: &Arc<Self>, mut config: ProxyConfig) -> Result<SocketAddr, Error> {
        if config.ip.is_loopback() && self.device_registry().lan_enabled() {
            config.ip = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        }
        let started = match proxy::bind(&config).await {
            Ok(listener) => proxy::spawn(Arc::downgrade(self), listener),
            Err(err) => Err(err),
        };
        let first = matches!(*self.proxy.lock(), ProxyState::Stopped) && self.network.reverse.lock().is_empty();
        let addr = match started {
            Ok(handle) => {
                let addr = handle.addr;
                *self.proxy.lock() = ProxyState::Listening(handle);
                addr
            }
            Err(err) => {
                let message = self.bind_error(&config, &err);
                *self.proxy.lock() = ProxyState::Failed(message.clone());
                return Err(Error::Proxy(message));
            }
        };
        // The reverse proxies start with the first listener.
        if first && self.network_settings().reverse_proxies.iter().any(|r| r.enabled) {
            self.restart_reverse_proxies().await;
        }
        Ok(addr)
    }

    /// Why the listener couldn't bind, and what to do about it.
    fn bind_error(&self, config: &ProxyConfig, err: &std::io::Error) -> String {
        let (first, last) = (*config.ports.start(), *config.ports.end());
        let ports = if first == last {
            format!("port {first}")
        } else {
            format!("ports {first}–{last}")
        };
        match err.kind() {
            std::io::ErrorKind::AddrInUse => {
                let owner = self
                    .platform
                    .clients
                    .port_owner(first)
                    .map(|app| format!(" ({app} is using {first})"))
                    .unwrap_or_default();
                format!(
                    "{ports} {} in use{owner}. Quit the app using it, or pick another port in Settings › Proxy.",
                    if first == last { "is" } else { "are all" }
                )
            }
            std::io::ErrorKind::PermissionDenied => {
                format!("not allowed to listen on {ports}: pick a port from 1024 up in Settings › Proxy.")
            }
            std::io::ErrorKind::AddrNotAvailable => {
                format!(
                    "can't listen on {}: that address isn't on this Mac any more.",
                    config.ip
                )
            }
            _ => format!("can't listen on {} {ports}: {err}", config.ip),
        }
    }

    /// The shared plain-HTTP upstream pool, built on first use.
    pub(crate) fn pool(self: &Arc<Self>) -> Arc<proxy::UpstreamClient> {
        if let Some(pool) = self.pool.load_full() {
            return pool;
        }
        let pool = Arc::new(proxy::upstream_client(Arc::downgrade(self), &self.proxy_settings()));
        self.pool.store(Some(pool.clone()));
        pool
    }

    /// Drops the pool: idle connections close, and new requests dial with
    /// the current settings.
    pub(crate) fn rebuild_pool(&self) {
        self.pool.store(None);
    }

    pub(crate) fn restart_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.restart
    }

    /// Stops the listener and starts it again with the current settings,
    /// on the same interfaces: on the configured port if `new_port`, else
    /// on the port it had.
    pub(crate) async fn restart_proxy(self: &Arc<Self>, new_port: bool) -> Result<SocketAddr, Error> {
        let old = std::mem::take(&mut *self.proxy.lock());
        let mut config = self.configured_proxy();
        if let ProxyState::Listening(handle) = old {
            config.ip = handle.addr.ip();
            if !new_port {
                config.ports = handle.addr.port()..=handle.addr.port();
            }
            handle.stop().await;
        }
        self.start_proxy(config).await
    }

    /// Moves the listener to `ip` on the same port (LAN access). If that
    /// port can't be bound there, the old listener comes back.
    pub(crate) async fn rebind_proxy(self: &Arc<Self>, ip: IpAddr) -> Result<SocketAddr, Error> {
        let _restart = self.restart.lock().await;
        let old = std::mem::take(&mut *self.proxy.lock());
        let ProxyState::Listening(handle) = old else {
            let ports = self.configured_proxy().ports;
            return self.start_proxy(ProxyConfig { ip, ports }).await;
        };
        let previous = handle.addr;
        if previous.ip() == ip {
            *self.proxy.lock() = ProxyState::Listening(handle);
            return Ok(previous);
        }
        handle.stop().await;
        let port = previous.port();
        let bound = match proxy::bind(&ProxyConfig { ip, ports: port..=port }).await {
            Ok(listener) => proxy::spawn(Arc::downgrade(self), listener).map_err(|e| (e, None)),
            Err(err) => {
                let back = proxy::bind(&ProxyConfig {
                    ip: previous.ip(),
                    ports: port..=port,
                })
                .await
                .and_then(|l| proxy::spawn(Arc::downgrade(self), l));
                Err((err, back.ok()))
            }
        };
        let mut state = self.proxy.lock();
        match bound {
            Ok(handle) => {
                let addr = handle.addr;
                *state = ProxyState::Listening(handle);
                Ok(addr)
            }
            Err((err, back)) => {
                let message = self.bind_error(&ProxyConfig { ip, ports: port..=port }, &err);
                *state = match back {
                    Some(handle) => ProxyState::Listening(handle),
                    None => ProxyState::Failed(message.clone()),
                };
                Err(Error::Proxy(message))
            }
        }
    }

    /// Where this machine reaches the listener: loopback when it listens on
    /// every interface.
    pub(crate) fn proxy_addr(&self) -> Option<SocketAddr> {
        match &*self.proxy.lock() {
            ProxyState::Listening(handle) if handle.addr.ip().is_unspecified() => {
                Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), handle.addr.port()))
            }
            ProxyState::Listening(handle) => Some(handle.addr),
            _ => None,
        }
    }

    /// `listening` is what the toolbar shows: the chosen LAN interface's
    /// address while LAN access is on.
    pub fn proxy_status(&self) -> ProxyStatus {
        let (listening, error) = match &*self.proxy.lock() {
            ProxyState::Stopped => (None, None),
            ProxyState::Listening(handle) => (Some(handle.addr), None),
            ProxyState::Failed(err) => (None, Some(err.clone())),
        };
        let listening = listening.map(|addr| self.display_addr(addr).to_string());
        ProxyStatus {
            listeners: self.extra_listeners(listening.as_deref()),
            listening,
            error,
            recording: self.is_recording(),
        }
    }

    /// Stops the proxy and deletes this session's segment files. Call on
    /// quit: the engine itself may outlive the event loop.
    pub fn shutdown(&self) {
        *self.proxy.lock() = ProxyState::Stopped;
        self.network.reverse.lock().clear();
        self.restore_android_proxies();
        self.details.segments.remove_all();
    }

    /// The root CA clients must trust for HTTPS decryption.
    pub fn ca_info(&self) -> CaInfo {
        self.tls.read().ca.info().clone()
    }

    /// Replaces the root CA (and so every leaf). Clients must trust the new
    /// one; connections already open keep their certificates.
    pub fn regenerate_ca(&self) -> Result<CaInfo, Error> {
        let ca = CertAuthority::create(&self.root.join("ca")).map_err(|e| Error::Tls(e.to_string()))?;
        let tls = TlsContext::new(ca, self.extra_roots.clone()).map_err(|e| Error::Tls(e.to_string()))?;
        let info = tls.ca.info().clone();
        *self.tls.write() = Arc::new(tls);
        Ok(info)
    }

    pub fn ssl_settings(&self) -> SslSettings {
        self.ssl.read().settings().clone()
    }

    /// Validates, saves and applies new SSL settings. New CONNECTs use them
    /// straight away.
    pub fn set_ssl_settings(&self, settings: SslSettings) -> Result<SslSettings, Error> {
        self.update_ssl(|s| *s = settings)
    }

    /// Stops decrypting `host` (e.g. an app that pins its certificates).
    pub fn add_ssl_bypass(&self, host: &str) -> Result<SslSettings, Error> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        self.update_ssl(|s| {
            if !s.bypass.contains(&host) {
                s.bypass.push(host);
            }
        })
    }

    /// Accepts (or stops accepting) `host`'s upstream certificate without
    /// verification, e.g. a dev server with a self-signed certificate.
    pub fn set_insecure_upstream(&self, host: &str, allow: bool) -> Result<SslSettings, Error> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        self.update_ssl(|s| {
            s.insecure_upstream.retain(|h| *h != host);
            if allow {
                s.insecure_upstream.push(host);
            }
        })
    }

    pub(crate) fn update_ssl(&self, change: impl FnOnce(&mut SslSettings)) -> Result<SslSettings, Error> {
        let mut current = self.ssl.write();
        let mut settings = current.settings().clone();
        change(&mut settings);
        let policy = SslPolicy::compile(settings)?;
        policy.save(&self.root.join(SSL_SETTINGS_FILE))?;
        let settings = policy.settings().clone();
        *current = Arc::new(policy);
        Ok(settings)
    }

    pub(crate) fn tls(&self) -> Arc<TlsContext> {
        self.tls.read().clone()
    }

    pub(crate) fn ssl_policy(&self) -> Arc<SslPolicy> {
        self.ssl.read().clone()
    }

    /// A view over the live store.
    pub fn create_view(self: &Arc<Self>, filter: FilterSpec, sort: SortSpec) -> ViewInfo {
        let (info, job) = {
            let mut store = self.store.write();
            let info = store.create_view(filter, sort);
            (info, store.view(info.view).and_then(|v| v.deep_job()))
        };
        self.spawn_deep_search(info.view, job);
        info
    }

    /// A view over any store. Its id says which store it is over, so the
    /// other view methods need no store.
    pub fn create_view_in(
        self: &Arc<Self>,
        store: StoreId,
        filter: FilterSpec,
        sort: SortSpec,
    ) -> Result<ViewInfo, Error> {
        match store {
            LIVE_STORE => Ok(self.create_view(filter, sort)),
            _ => {
                let doc = self.docs.get(store)?;
                let (info, job) = {
                    let mut store = doc.store.write();
                    let info = store.create_view(filter, sort);
                    (info, store.view(info.view).and_then(|v| v.deep_job()))
                };
                self.spawn_deep_search(info.view, job);
                Ok(info)
            }
        }
    }

    /// Refilters synchronously and returns the new length, so callers can
    /// fetch rows straight away instead of waiting for the next delta. A
    /// filter with header or body terms starts a deep search: its matches
    /// stream into the view, with progress in the view's deltas.
    pub fn set_filter(self: &Arc<Self>, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, Error> {
        let (info, job) = self.with_view_store(view, |store| {
            let info = store.set_filter(view, filter)?;
            Ok((info, store.view(view).and_then(|v| v.deep_job())))
        })?;
        self.spawn_deep_search(view, job);
        Ok(info)
    }

    /// Runs a view's deep search on its own thread, checking candidates on
    /// the rayon pool a batch at a time, until the filter changes or the
    /// view is dropped.
    fn spawn_deep_search(self: &Arc<Self>, view: ViewId, job: Option<DeepJob>) {
        let Some(DeepJob { query, cancel }) = job else {
            return;
        };
        let engine = Arc::downgrade(self);
        let store = view >> VIEW_STORE_SHIFT;
        let spawned = std::thread::Builder::new()
            .name("pm-deep-search".into())
            .spawn(move || {
                use rayon::prelude::*;
                loop {
                    if cancel.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(engine) = engine.upgrade() else {
                        return;
                    };
                    let batch = engine.with_view_store(view, |s| Ok(s.take_deep_batch(view, &cancel, DEEP_BATCH)));
                    let Ok(Some(batch)) = batch else {
                        return;
                    };
                    if batch.is_empty() {
                        drop(engine);
                        std::thread::sleep(DEEP_IDLE);
                        continue;
                    }
                    let hits: Vec<FlowId> = engine
                        .with_backend(store, |backend| {
                            Ok(batch
                                .par_iter()
                                .filter(|&&id| !cancel.load(Ordering::Relaxed) && query.matches(backend, id))
                                .copied()
                                .collect())
                        })
                        .unwrap_or_default();
                    if cancel.load(Ordering::Acquire) {
                        return;
                    }
                    let _ = engine.with_view_store(view, |s| {
                        s.deep_results(view, &cancel, batch.len() as u32, &hits);
                        Ok(())
                    });
                }
            });
        if let Err(err) = spawned {
            eprintln!("proxymate: can't start a deep search: {err}");
        }
    }

    pub fn set_sort(&self, view: ViewId, sort: SortSpec) -> Result<ViewInfo, Error> {
        self.with_view_store(view, |store| store.set_sort(view, sort))
    }

    pub fn drop_view(&self, view: ViewId) {
        self.sinks.lock().remove(&view);
        let _ = self.with_view_store(view, |store| Ok(store.drop_view(view)));
    }

    pub fn get_rows(&self, view: ViewId, start: u32, count: u32) -> Result<RowsPage, Error> {
        match view >> VIEW_STORE_SHIFT {
            LIVE_STORE => self.store.read().rows(view, start, count),
            store => self.docs.get(store)?.store.read().rows(view, start, count),
        }
    }

    /// Routes a view's deltas to `sink`, replacing any previous subscriber.
    /// The first delta is structural and goes out on the next tick.
    pub fn subscribe(&self, view: ViewId, sink: DeltaSink) -> Result<(), Error> {
        self.with_view_store(view, |store| {
            store.view_mut(view).ok_or(Error::UnknownView(view))?.mark_structural();
            Ok(())
        })?;
        self.sinks.lock().insert(view, sink);
        Ok(())
    }

    /// Drops the delta subscribers whose view fails `keep`.
    pub(crate) fn sinks_retain(&self, keep: impl Fn(ViewId) -> bool) {
        self.sinks.lock().retain(|&view, _| keep(view));
    }

    /// Runs `f` on the store `view` is over, write-locked.
    pub(crate) fn with_view_store<R>(
        &self,
        view: ViewId,
        f: impl FnOnce(&mut Store) -> Result<R, Error>,
    ) -> Result<R, Error> {
        match view >> VIEW_STORE_SHIFT {
            LIVE_STORE => f(&mut self.store.write()),
            store => {
                let doc = self.docs.get(store).map_err(|_| Error::UnknownView(view))?;
                let mut store = doc.store.write();
                f(&mut store)
            }
        }
    }

    /// Publishes one tick: drains each subscribed view's pending change and
    /// hands it to its sink. Called by the publisher thread; public for tests
    /// and headless tools.
    pub fn publish_tick(&self) {
        self.publish_sidebar();
        self.publish_storage();
        self.rules.breakpoints.publish();
        self.rules.publish();
        self.scripts.publish();
        self.publish_mcp();
        self.device_registry().publish();
        let mut sinks = self.sinks.lock();
        if sinks.is_empty() {
            return;
        }
        let deltas: Vec<ViewDelta> = {
            let mut store = self.store.write();
            sinks
                .keys()
                .filter_map(|&view| match view >> VIEW_STORE_SHIFT {
                    LIVE_STORE => store.take_delta(view),
                    doc => self.docs.get(doc).ok()?.store.write().take_delta(view),
                })
                .collect()
        };
        for delta in deltas {
            let view = delta.view;
            if let Some(sink) = sinks.get_mut(&view)
                && !sink(delta)
            {
                sinks.remove(&view);
            }
        }
    }

    /// Applies one ingest batch under a single write lock, then evicts
    /// past the storage limits.
    pub(crate) fn apply_events(&self, batch: Vec<FlowEvent>) -> Vec<std::sync::mpsc::SyncSender<()>> {
        let (acks, eviction) = {
            let mut store = self.store.write();
            let acks = store.apply(batch);
            let eviction = self.evict_locked(&mut store);
            (acks, eviction)
        };
        if let Some(eviction) = eviction {
            self.after_eviction(eviction);
        }
        acks
    }

    pub(crate) fn interned(&self) -> &Mutex<HashSet<Arc<str>>> {
        &self.interned
    }

    pub(crate) fn ingest(&self) -> &IngestTx {
        &self.ingest
    }

    pub(crate) fn details(&self) -> &DetailStore {
        &self.details
    }

    /// Hosts and methods repeat across flows; share one allocation each.
    fn intern(&self, s: &Arc<str>) -> Arc<str> {
        let mut interned = self.interned.lock();
        if let Some(existing) = interned.get(&**s) {
            return existing.clone();
        }
        interned.insert(s.clone());
        s.clone()
    }
}

/// A body's bytes from `backend`; see [`Engine::body_head`].
pub(crate) fn body_head(
    backend: &dyn BlobBackend,
    id: FlowId,
    part: BodyPart,
    decode: bool,
    max: Option<usize>,
) -> Result<BodyBytes, Error> {
    let mut bytes = backend.body(id, part)?.ok_or(Error::UnknownFlow(id))?;
    let encoding = if decode {
        backend.detail(id)?.and_then(|detail| match part {
            BodyPart::Request => detail.request_body.encoding,
            BodyPart::Response => detail.response_body.encoding,
        })
    } else {
        None
    };
    let (bytes, decoded_from) = match (encoding, max) {
        (Some(encoding), None) if !bytes.is_empty() => (decode::decode(&encoding, &bytes)?, Some(encoding)),
        (Some(encoding), Some(max)) if !bytes.is_empty() => {
            let head = decode::decode_prefix(&encoding, &bytes, max.saturating_add(1));
            if head.is_empty() {
                // Report why it didn't decode.
                decode::decode(&encoding, &bytes)?;
            }
            (head, Some(encoding))
        }
        _ => {
            if let Some(max) = max {
                bytes.truncate(max.saturating_add(1));
            }
            (bytes, None)
        }
    };
    let mut bytes = bytes;
    let cut = max.is_some_and(|max| bytes.len() > max);
    if let Some(max) = max {
        bytes.truncate(max);
    }
    Ok(BodyBytes {
        bytes,
        decoded_from,
        cut,
    })
}

fn publisher_loop(engine: Weak<Engine>) {
    loop {
        std::thread::sleep(PUBLISH_INTERVAL);
        let Some(engine) = engine.upgrade() else {
            return;
        };
        engine.publish_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Generator;
    use std::sync::mpsc;

    #[test]
    fn subscribers_receive_structural_then_incremental_deltas() {
        let engine = Engine::new();
        let mut generator = Generator::new(1);
        engine.insert_flows((0..10).map(|_| generator.completed_flow()).collect());
        let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;

        let (tx, rx) = mpsc::channel();
        engine.subscribe(view, Box::new(move |d| tx.send(d).is_ok())).unwrap();
        engine.publish_tick();
        let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(first.structural);
        assert_eq!(first.len, 10);

        engine.insert_flows((0..3).map(|_| generator.completed_flow()).collect());
        engine.publish_tick();
        // The background publisher may have raced us to this tick.
        let delta = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!((delta.appended, delta.len), (3, 13));
    }

    #[test]
    fn closed_sinks_are_dropped() {
        let engine = Engine::new();
        let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
        engine.subscribe(view, Box::new(|_| false)).unwrap();
        engine.publish_tick();
        assert!(engine.sinks.lock().is_empty());
    }
}
