//! The shell's shared state: the part of `store/ui.ts` the chrome needs
//! (the active tab, the opened documents, the filter, the background save)
//! plus the proxy's status and the SSL lists the sidebar's menus tick, with
//! the actions of `lib/actions.ts` that change them.
//!
//! One `Workspace` entity per app, reachable with [`Workspace::global`].
//! Engine calls run on the background executor; without an engine (tests,
//! offscreen renders) the state still changes, so views render from
//! injected snapshots.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString};
use pm_core::Engine;
use pm_model::{
    ContentGroup, DocumentInfo, FilterSpec, FlowSelection, LIVE_STORE, ProtocolFilter, ProxyStatus, SaveProgress,
    SslSettings, StoreId,
};

use crate::features::clipboard::report_error;

/// How long "Saved N flows to …" stays in the footer.
pub const SAVED_NOTE_FOR: Duration = Duration::from_secs(4);
/// How often the proxy's status is read again (a port change, a failure).
const PROXY_POLL: Duration = Duration::from_secs(2);

/// A background save or export, mirrored in the footer.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveJob {
    pub label: SharedString,
    pub progress: SaveProgress,
    seq: u64,
}

impl SaveJob {
    pub fn new(label: impl Into<SharedString>, progress: SaveProgress) -> SaveJob {
        SaveJob {
            label: label.into(),
            progress,
            seq: 0,
        }
    }
}

/// Save Session or Export HAR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Session,
    Har,
}

impl SaveKind {
    fn label(self) -> &'static str {
        match self {
            SaveKind::Session => "Saving session",
            SaveKind::Har => "Exporting HAR",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            SaveKind::Session => "proxymate",
            SaveKind::Har => "har",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceEvent {
    /// The filter changed; `text_only` changes are debounced by the table.
    FilterChanged { text_only: bool },
    /// The active tab changed.
    StoreChanged,
}

pub struct Workspace {
    engine: Option<Arc<Engine>>,
    pub proxy: ProxyStatus,
    /// The tab shown: the live capture or a document.
    pub store: StoreId,
    pub documents: Vec<DocumentInfo>,
    pub filter: FilterSpec,
    pub save: Option<SaveJob>,
    pub ssl: SslSettings,
    save_seq: u64,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

struct GlobalWorkspace(Entity<Workspace>);

impl Global for GlobalWorkspace {}

impl Workspace {
    /// The app's workspace over the bridge's engine (or none, in tests).
    pub fn new(engine: Option<Arc<Engine>>, proxy: ProxyStatus, cx: &mut Context<Self>) -> Workspace {
        let ssl = engine.as_ref().map(|e| e.ssl_settings()).unwrap_or_default();
        let documents = engine.as_ref().map(|e| e.documents()).unwrap_or_default();
        let filter = FilterSpec {
            text: crate::dev_env::value("PROXYMATE_DEV_FILTER").unwrap_or_default(),
            ..Default::default()
        };
        if engine.is_some() {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(PROXY_POLL).await;
                    if this.update(cx, |this, cx| this.refresh_proxy(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        Workspace {
            engine,
            proxy,
            store: LIVE_STORE,
            documents,
            filter,
            save: None,
            ssl,
            save_seq: 0,
        }
    }

    /// Makes `workspace` the one [`Workspace::global`] returns.
    pub fn set_global(workspace: Entity<Workspace>, cx: &mut App) {
        cx.set_global(GlobalWorkspace(workspace));
    }

    pub fn global(cx: &App) -> Option<Entity<Workspace>> {
        cx.try_global::<GlobalWorkspace>().map(|g| g.0.clone())
    }

    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.engine.as_ref()
    }

    pub fn is_live(&self) -> bool {
        self.store == LIVE_STORE
    }

    pub fn document(&self, store: StoreId) -> Option<&DocumentInfo> {
        self.documents.iter().find(|d| d.store == store)
    }

    // ---- The filter ----

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.filter.text == text {
            return;
        }
        self.filter.text = text;
        cx.emit(WorkspaceEvent::FilterChanged { text_only: true });
        cx.notify();
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceEvent::FilterChanged { text_only: false });
        cx.notify();
    }

    /// One protocol chip at a time.
    pub fn set_protocol(&mut self, protocol: ProtocolFilter, cx: &mut Context<Self>) {
        if self.filter.protocol != protocol {
            self.filter.protocol = protocol;
            self.filter_changed(cx);
        }
    }

    /// Content chips combine.
    pub fn toggle_content(&mut self, group: ContentGroup, cx: &mut Context<Self>) {
        let content = &mut self.filter.content;
        match content.iter().position(|g| *g == group) {
            Some(i) => {
                content.remove(i);
            }
            None => content.push(group),
        }
        self.filter_changed(cx);
    }

    pub fn clear_content(&mut self, cx: &mut Context<Self>) {
        if !self.filter.content.is_empty() {
            self.filter.content.clear();
            self.filter_changed(cx);
        }
    }

    /// The sidebar's scope: one host (or `*.domain`) or one app; setting
    /// one clears the other.
    pub fn set_scope(&mut self, host: Option<String>, client: Option<String>, cx: &mut Context<Self>) {
        if self.filter.host != host || self.filter.client != client {
            self.filter.host = host;
            self.filter.client = client;
            self.filter_changed(cx);
        }
    }

    // ---- Capture ----

    /// Pauses or resumes recording; traffic is forwarded either way.
    pub fn set_recording(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(engine) = &self.engine {
            engine.set_recording(on);
            self.proxy = engine.proxy_status();
        } else {
            self.proxy.recording = on;
        }
        cx.notify();
    }

    /// Reads the proxy's status again (an O(1) read of the listener).
    pub fn refresh_proxy(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = &self.engine else { return };
        let status = engine.proxy_status();
        if status != self.proxy {
            self.proxy = status;
            cx.notify();
        }
    }

    /// Clears every live flow; the view's structural delta empties the table.
    pub fn clear_flows(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else { return };
        cx.background_spawn(async move { engine.clear() }).detach();
    }

    // ---- Tabs and documents ----

    pub fn set_store(&mut self, store: StoreId, cx: &mut Context<Self>) {
        if self.store == store {
            return;
        }
        self.store = store;
        cx.emit(WorkspaceEvent::StoreChanged);
        cx.notify();
    }

    /// Re-reads the opened documents.
    pub fn refresh_documents(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = &self.engine {
            self.documents = engine.documents();
            self.documents.sort_by_key(|d| d.store);
            cx.notify();
        }
    }

    /// Opens a session, HAR or Charles file in a new tab and shows it.
    pub fn open_document(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else { return };
        // Loading a big session or HAR takes a moment.
        let opened = cx.background_spawn(async move { engine.open_document(&path) });
        cx.spawn(async move |this, cx| {
            let opened = opened.await;
            let _ = this.update(cx, |this, cx| match opened {
                Ok(doc) => {
                    this.refresh_documents(cx);
                    this.set_store(doc.store, cx);
                }
                Err(err) => report_error("Couldn't open the file", err, cx),
            });
        })
        .detach();
    }

    /// Closes a document; closing the one shown returns to the live capture.
    pub fn close_document(&mut self, store: StoreId, cx: &mut Context<Self>) {
        if store == LIVE_STORE {
            return;
        }
        if self.store == store {
            self.set_store(LIVE_STORE, cx);
        }
        let Some(engine) = self.engine.clone() else {
            self.documents.retain(|d| d.store != store);
            cx.notify();
            return;
        };
        let closed = cx.background_spawn(async move { engine.close_document(store) });
        cx.spawn(async move |this, cx| {
            let closed = closed.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(err) = closed {
                    report_error("Couldn't close the document", err, cx);
                }
                this.refresh_documents(cx);
            });
        })
        .detach();
    }

    /// A file name for what the active tab holds: the document's name, or
    /// "Proxymate YYYY-MM-DD HH.MM".
    pub fn suggested_name(&self, kind: SaveKind) -> String {
        let extension = kind.extension();
        if let Some(doc) = self.document(self.store) {
            let lower = doc.name.to_ascii_lowercase();
            let stem = [".har", ".proxymate"]
                .iter()
                .find_map(|ext| lower.ends_with(ext).then(|| &doc.name[..doc.name.len() - ext.len()]))
                .unwrap_or(&doc.name);
            return format!("{stem}.{extension}");
        }
        let now = chrono::Local::now();
        format!("Proxymate {}.{extension}", now.format("%Y-%m-%d %H.%M"))
    }

    /// Saves (or exports) `selection` of the active tab to `path` in the
    /// background, mirroring its progress in the footer.
    pub fn save_to(&mut self, kind: SaveKind, selection: FlowSelection, path: PathBuf, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else { return };
        self.save_seq += 1;
        let seq = self.save_seq;
        let label: SharedString = kind.label().into();
        let store = self.store;
        let (tx, rx) = async_channel::unbounded::<SaveProgress>();
        let started = {
            let path = path.clone();
            let sink: pm_core::ProgressSink = Box::new(move |p| {
                let _ = tx.try_send(p);
            });
            cx.background_spawn(async move {
                match kind {
                    SaveKind::Session => engine.save_session(store, &selection, path, sink),
                    SaveKind::Har => engine.export_har(store, &selection, path, sink),
                }
            })
        };
        self.save = Some(SaveJob {
            label: label.clone(),
            progress: SaveProgress {
                path: path.display().to_string(),
                ..Default::default()
            },
            seq,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Err(err) = started.await {
                let _ = this.update(cx, |this, cx| {
                    this.set_save_progress(
                        seq,
                        SaveProgress {
                            finished: true,
                            error: Some(err.to_string()),
                            ..Default::default()
                        },
                        cx,
                    )
                });
                return;
            }
            while let Ok(mut progress) = rx.recv().await {
                while let Ok(newer) = rx.try_recv() {
                    progress = newer;
                }
                let finished = progress.finished;
                if this
                    .update(cx, |this, cx| this.set_save_progress(seq, progress, cx))
                    .is_err()
                    || finished
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn set_save_progress(&mut self, seq: u64, progress: SaveProgress, cx: &mut Context<Self>) {
        let Some(job) = self.save.as_mut().filter(|j| j.seq == seq) else {
            return;
        };
        let saved = progress.finished && progress.error.is_none();
        job.progress = progress;
        cx.notify();
        if saved {
            // The "saved" note stays a moment, unless another save replaced it.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVED_NOTE_FOR).await;
                let _ = this.update(cx, |this, cx| {
                    if this.save.as_ref().is_some_and(|j| j.seq == seq) {
                        this.dismiss_save(cx);
                    }
                });
            })
            .detach();
        }
    }

    /// Sets the footer's save status directly (tests, offscreen renders).
    pub fn set_save(&mut self, save: Option<SaveJob>, cx: &mut Context<Self>) {
        self.save = save;
        cx.notify();
    }

    pub fn dismiss_save(&mut self, cx: &mut Context<Self>) {
        self.save = None;
        cx.notify();
    }

    // ---- SSL lists (the sidebar's menus) ----

    fn change_ssl(
        &mut self,
        what: &'static str,
        change: impl FnOnce(&Engine) -> Result<SslSettings, pm_core::Error> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.engine.clone() else { return };
        let changed = cx.background_spawn(async move { change(&engine) });
        cx.spawn(async move |this, cx| {
            let changed = changed.await;
            let _ = this.update(cx, |this, cx| match changed {
                Ok(ssl) => {
                    this.ssl = ssl;
                    cx.notify();
                }
                Err(err) => report_error(what, err, cx),
            });
        })
        .detach();
    }

    /// Stops decrypting `host`; its next CONNECTs are tunnelled.
    pub fn add_ssl_bypass(&mut self, host: String, cx: &mut Context<Self>) {
        self.change_ssl("Couldn't add the SSL bypass", move |e| e.add_ssl_bypass(&host), cx);
    }

    /// Accepts (or stops accepting) `host`'s upstream certificate unverified.
    pub fn set_insecure_upstream(&mut self, host: String, allow: bool, cx: &mut Context<Self>) {
        self.change_ssl(
            "Couldn't change the certificate setting",
            move |e| e.set_insecure_upstream(&host, allow),
            cx,
        );
    }

    /// Stops (or resumes) decrypting everything `app` sends.
    pub fn set_app_ssl_excluded(&mut self, app: String, excluded: bool, cx: &mut Context<Self>) {
        self.change_ssl(
            "Couldn't change the app's SSL setting",
            move |e| e.set_app_ssl_excluded(&app, excluded),
            cx,
        );
    }

    pub fn is_insecure(&self, host: &str) -> bool {
        self.ssl.insecure_upstream.iter().any(|h| h.eq_ignore_ascii_case(host))
    }

    pub fn is_app_excluded(&self, key: &str) -> bool {
        self.ssl.exclude_apps.iter().any(|a| a.eq_ignore_ascii_case(key))
    }

    /// `export HTTPS_PROXY=…` lines for a terminal (a string the engine
    /// builds from the address and the CA path).
    pub fn terminal_env(&self) -> Option<String> {
        self.engine.as_ref().map(|e| e.terminal_env())
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use pm_model::DocumentKind;

    use super::*;

    fn doc(store: StoreId, name: &str) -> DocumentInfo {
        DocumentInfo {
            store,
            kind: DocumentKind::Har,
            name: name.into(),
            path: format!("/tmp/{name}"),
            flows: 3,
        }
    }

    #[gpui_kit::test]
    fn filter_changes_emit_and_scopes_replace_each_other(cx: &mut TestAppContext) {
        let ws = cx.new(|cx| Workspace::new(None, ProxyStatus::default(), cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = events.clone();
        let _sub = cx.update(|cx| cx.subscribe(&ws, move |_, e: &WorkspaceEvent, _| seen.borrow_mut().push(e.clone())));
        ws.update(cx, |w, cx| {
            w.set_text("host:a", cx);
            w.set_text("host:a", cx);
            w.set_protocol(ProtocolFilter::Https, cx);
            w.toggle_content(ContentGroup::Json, cx);
            w.toggle_content(ContentGroup::Xml, cx);
            w.toggle_content(ContentGroup::Json, cx);
            w.set_scope(Some("*.example.com".into()), None, cx);
            w.set_scope(None, Some("curl".into()), cx);
        });
        let f = ws.read_with(cx, |w, _| w.filter.clone());
        assert_eq!(f.content, [ContentGroup::Xml]);
        assert_eq!((f.host, f.client.as_deref()), (None, Some("curl")));
        let events = events.borrow();
        assert_eq!(events[0], WorkspaceEvent::FilterChanged { text_only: true });
        assert_eq!(events.len(), 7, "the repeated text is no change: {events:?}");
    }

    #[gpui_kit::test]
    fn closing_the_shown_document_returns_to_live_and_names_follow_the_tab(cx: &mut TestAppContext) {
        let ws = cx.new(|cx| Workspace::new(None, ProxyStatus::default(), cx));
        ws.update(cx, |w, cx| {
            w.documents = vec![doc(1, "trip.har"), doc(2, "Login.proxymate")];
            assert!(w.suggested_name(SaveKind::Har).starts_with("Proxymate "));
            w.set_store(2, cx);
            assert_eq!(w.suggested_name(SaveKind::Har), "Login.har");
            w.set_store(1, cx);
            assert_eq!(w.suggested_name(SaveKind::Session), "trip.proxymate");
            w.close_document(1, cx);
        });
        let (store, docs) = ws.read_with(cx, |w, _| (w.store, w.documents.len()));
        assert_eq!((store, docs), (LIVE_STORE, 1));
    }
}
