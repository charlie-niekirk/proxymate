//! The `Engine` facade. Tauri commands, MCP tools and the CLI call only this,
//! so the UI and agents always see the same behaviour.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use pm_model::{
    BodyPart, CaInfo, FilterSpec, FlowDetail, FlowId, FlowSummary, ProxyStatus, RowsPage, SortSpec, SslSettings,
    ViewDelta, ViewId, ViewInfo,
};
use rustls::pki_types::CertificateDer;

use crate::Error;
use crate::ingest::{self, FlowEvent, IngestTx};
use crate::io::decode;
use crate::proxy::{self, ProxyConfig, ProxyHandle};
use crate::recorder::FlowRecorder;
use crate::store::{DetailStore, SegmentStore, Store};
use crate::tls::{CertAuthority, SslPolicy, TlsContext};

const SSL_SETTINGS_FILE: &str = "ssl.json";

/// Deltas are published at 20 Hz.
pub const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// Receives a view's deltas. Returning `false` unsubscribes (e.g. the
/// webview's channel was closed).
pub type DeltaSink = Box<dyn FnMut(ViewDelta) -> bool + Send>;

#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    /// Root for live segment files (`<data_dir>/live/<uuid>/`), the CA
    /// (`<data_dir>/ca/`) and `ssl.json`. Defaults to a `proxymate`
    /// directory under the system temp dir.
    pub data_dir: Option<PathBuf>,
    /// DER certificates trusted for upstream TLS on top of the OS trust
    /// store, e.g. a test CA or a corporate CA the OS doesn't know.
    pub extra_upstream_roots: Vec<Vec<u8>>,
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
    store: RwLock<Store>,
    sinks: Mutex<HashMap<ViewId, DeltaSink>>,
    ingest: IngestTx,
    details: DetailStore,
    recording: AtomicBool,
    proxy: Mutex<ProxyState>,
    interned: Mutex<HashSet<Arc<str>>>,
    root: PathBuf,
    extra_roots: Vec<CertificateDer<'static>>,
    tls: RwLock<Arc<TlsContext>>,
    /// Write-locked across a read-modify-write of the settings.
    ssl: RwLock<Arc<SslPolicy>>,
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
            root,
            extra_roots,
            tls: RwLock::new(Arc::new(tls)),
            ssl: RwLock::new(Arc::new(policy)),
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
    pub fn begin_flow(self: &Arc<Self>, mut summary: FlowSummary, detail: FlowDetail) -> Option<FlowRecorder> {
        if !self.is_recording() {
            return None;
        }
        summary.method = self.intern(&summary.method);
        summary.host = self.intern(&summary.host);
        let id = self.ingest.start(summary);
        Some(FlowRecorder::new(id, self, detail))
    }

    pub fn get_flow(&self, id: FlowId) -> Option<FlowSummary> {
        self.store.read().get(id).cloned()
    }

    /// Heads and body metadata of a flow, live or finished.
    pub fn flow_detail(&self, id: FlowId) -> Result<Option<FlowDetail>, Error> {
        Ok(self.details.get(id)?.map(|stored| stored.detail.clone()))
    }

    /// The captured bytes of one body. With `decode`, a content-encoded
    /// body (gzip, br, deflate, zstd) is decoded first.
    pub fn body(&self, id: FlowId, part: BodyPart, decode: bool) -> Result<BodyBytes, Error> {
        self.body_head(id, part, decode, None)
    }

    /// Like [`Engine::body`], but at most the first `max` bytes (decoded,
    /// with `decode`), so a huge body can be previewed cheaply.
    pub fn body_head(&self, id: FlowId, part: BodyPart, decode: bool, max: Option<usize>) -> Result<BodyBytes, Error> {
        let mut bytes = self.details.body(id, part)?.ok_or(Error::UnknownFlow(id))?;
        let encoding = if decode {
            self.details.get(id)?.and_then(|stored| match part {
                BodyPart::Request => stored.detail.request_body.encoding.clone(),
                BodyPart::Response => stored.detail.response_body.encoding.clone(),
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

    /// Writes one body (decoded, with `decode`) to `path`.
    pub fn save_body(&self, id: FlowId, part: BodyPart, decode: bool, path: &std::path::Path) -> Result<u64, Error> {
        let body = self.body(id, part, decode)?;
        std::fs::write(path, &body.bytes)?;
        Ok(body.bytes.len() as u64)
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
    /// [`Engine::proxy_status`].
    pub async fn start_proxy(self: &Arc<Self>, config: ProxyConfig) -> Result<SocketAddr, Error> {
        let started = match proxy::bind(&config).await {
            Ok(listener) => proxy::spawn(Arc::downgrade(self), listener),
            Err(err) => Err(err),
        };
        let mut state = self.proxy.lock();
        match started {
            Ok(handle) => {
                let addr = handle.addr;
                *state = ProxyState::Listening(handle);
                Ok(addr)
            }
            Err(err) => {
                let ports = format!("{}–{}", config.ports.start(), config.ports.end());
                let message = format!("can't listen on {} ports {ports}: {err}", config.ip);
                *state = ProxyState::Failed(message.clone());
                Err(Error::Proxy(message))
            }
        }
    }

    pub fn proxy_status(&self) -> ProxyStatus {
        let (listening, error) = match &*self.proxy.lock() {
            ProxyState::Stopped => (None, None),
            ProxyState::Listening(handle) => (Some(handle.addr.to_string()), None),
            ProxyState::Failed(err) => (None, Some(err.clone())),
        };
        ProxyStatus {
            listening,
            error,
            recording: self.is_recording(),
        }
    }

    /// Stops the proxy and deletes this session's segment files. Call on
    /// quit: the engine itself may outlive the event loop.
    pub fn shutdown(&self) {
        *self.proxy.lock() = ProxyState::Stopped;
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

    fn update_ssl(&self, change: impl FnOnce(&mut SslSettings)) -> Result<SslSettings, Error> {
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

    pub fn create_view(&self, filter: FilterSpec, sort: SortSpec) -> ViewInfo {
        self.store.write().create_view(filter, sort)
    }

    /// Refilters synchronously and returns the new length, so callers can
    /// fetch rows straight away instead of waiting for the next delta.
    pub fn set_filter(&self, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, Error> {
        self.store.write().set_filter(view, filter)
    }

    pub fn set_sort(&self, view: ViewId, sort: SortSpec) -> Result<ViewInfo, Error> {
        self.store.write().set_sort(view, sort)
    }

    pub fn drop_view(&self, view: ViewId) {
        self.sinks.lock().remove(&view);
        self.store.write().drop_view(view);
    }

    pub fn get_rows(&self, view: ViewId, start: u32, count: u32) -> Result<RowsPage, Error> {
        self.store.read().rows(view, start, count)
    }

    /// Routes a view's deltas to `sink`, replacing any previous subscriber.
    /// The first delta is structural and goes out on the next tick.
    pub fn subscribe(&self, view: ViewId, sink: DeltaSink) -> Result<(), Error> {
        self.store
            .write()
            .view_mut(view)
            .ok_or(Error::UnknownView(view))?
            .mark_structural();
        self.sinks.lock().insert(view, sink);
        Ok(())
    }

    /// Publishes one tick: drains each subscribed view's pending change and
    /// hands it to its sink. Called by the publisher thread; public for tests
    /// and headless tools.
    pub fn publish_tick(&self) {
        let mut sinks = self.sinks.lock();
        if sinks.is_empty() {
            return;
        }
        let deltas: Vec<ViewDelta> = {
            let mut store = self.store.write();
            sinks.keys().filter_map(|&view| store.take_delta(view)).collect()
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

    /// Applies one ingest batch under a single write lock.
    pub(crate) fn apply_events(&self, batch: Vec<FlowEvent>) -> Vec<std::sync::mpsc::SyncSender<()>> {
        self.store.write().apply(batch)
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
