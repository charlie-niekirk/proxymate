//! Documents: saved sessions and imported HAR and Charles files, each opened as its
//! own store beside the live capture (a [`Store`] for summaries and views,
//! a [`SqliteBackend`] for everything else), and the background writers
//! that save any store as a session or export it as HAR.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::RwLock;
use pm_model::{DocumentInfo, DocumentKind, FlowId, FlowSelection, FlowSummary, LIVE_STORE, SaveProgress, StoreId};
use rusqlite::Connection;

use crate::io::{charles, har, session};
use crate::store::{Opened, SavedFlow, SessionWriter, SqliteBackend, Store, VIEW_STORE_SHIFT};
use crate::{Engine, Error};

/// Receives a save's progress; the last update has `finished` set.
pub type ProgressSink = Box<dyn FnMut(SaveProgress) + Send>;

/// Document stores get ids `1..=MAX_DOCUMENTS`, never reused in a run, so
/// their view ids (`store << VIEW_STORE_SHIFT`) fit a `u32`.
const MAX_DOCUMENTS: StoreId = (1 << (32 - VIEW_STORE_SHIFT)) - 1;

pub(crate) struct Document {
    pub info: DocumentInfo,
    pub store: RwLock<Store>,
    pub backend: SqliteBackend,
}

#[derive(Default)]
pub(crate) struct Documents {
    last: AtomicU32,
    open: RwLock<BTreeMap<StoreId, Arc<Document>>>,
}

impl Documents {
    pub fn get(&self, store: StoreId) -> Result<Arc<Document>, Error> {
        self.open.read().get(&store).cloned().ok_or(Error::UnknownStore(store))
    }

    fn add(
        &self,
        kind: DocumentKind,
        path: &Path,
        opened: Opened,
        operations: Arc<crate::openapi::OpIndex>,
    ) -> Result<DocumentInfo, Error> {
        let store = self.last.fetch_add(1, Ordering::Relaxed) + 1;
        if store > MAX_DOCUMENTS {
            return Err(Error::Document(format!(
                "can't open more than {MAX_DOCUMENTS} documents in one run; restart Proxymate"
            )));
        }
        let info = DocumentInfo {
            store,
            kind,
            name: path
                .file_name()
                .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            path: path.display().to_string(),
            flows: opened.summaries.len() as u32,
        };
        let mut flows = Store::document(store, opened.summaries);
        flows.set_operations(Some(operations));
        let doc = Document {
            info: info.clone(),
            store: RwLock::new(flows),
            backend: opened.backend,
        };
        self.open.write().insert(store, Arc::new(doc));
        Ok(info)
    }
}

/// True if `path` starts like an SQLite database (a saved session).
pub(crate) fn is_sqlite(path: &Path) -> bool {
    let mut magic = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| io::Read::read_exact(&mut f, &mut magic))
        .is_ok_and(|()| &magic == b"SQLite format 3\0")
}

/// The flows of a HAR or Charles file, told apart by their contents: a
/// Charles JSON session is a top-level array, XML starts with `<`, HAR is
/// an object. Charles's binary `.chls` (Java serialisation) is refused
/// with how to export a readable one.
pub(crate) fn import_file(path: &Path) -> Result<(DocumentKind, Vec<SavedFlow>), Error> {
    let bytes = std::fs::read(path)?;
    let named = |e: &dyn std::fmt::Display| Error::Document(format!("{}: {e}", path.display()));
    match charles::sniff(&bytes) {
        charles::Sniffed::Json => Ok((
            DocumentKind::Charles,
            charles::import_json(&bytes).map_err(|e| named(&e))?,
        )),
        charles::Sniffed::Xml => Ok((
            DocumentKind::Charles,
            charles::import_xml(&bytes).map_err(|e| named(&e))?,
        )),
        charles::Sniffed::Binary => Err(Error::Document(charles::BINARY_UNSUPPORTED.into())),
        charles::Sniffed::Other => Ok((DocumentKind::Har, har::import(&bytes).map_err(|e| named(&e))?)),
    }
}

impl Engine {
    /// Opens a saved session (`.proxymate`) or imports a HAR or Charles
    /// file as a new document, by its contents rather than its extension.
    pub fn open_document(self: &Arc<Self>, path: &Path) -> Result<DocumentInfo, Error> {
        if is_sqlite(path) {
            let opened = session::open(path).map_err(|e| Error::Document(e.to_string()))?;
            let info = self.docs.add(DocumentKind::Session, path, opened, self.operations())?;
            self.columns_for_document(info.store);
            Ok(info)
        } else {
            let (kind, flows) = import_file(path)?;
            self.add_imported(kind, path, flows)
        }
    }

    /// Imports a HAR 1.2 file (Chrome's `_webSocketMessages` included) as a
    /// new document. The flows are converted into session tables in memory.
    pub fn import_har(self: &Arc<Self>, path: &Path) -> Result<DocumentInfo, Error> {
        let bytes = std::fs::read(path)?;
        let flows = har::import(&bytes).map_err(|e| Error::Document(format!("{}: {e}", path.display())))?;
        self.add_imported(DocumentKind::Har, path, flows)
    }

    fn add_imported(
        self: &Arc<Self>,
        kind: DocumentKind,
        path: &Path,
        flows: Vec<SavedFlow>,
    ) -> Result<DocumentInfo, Error> {
        let conn = Connection::open_in_memory().map_err(|e| Error::Document(e.to_string()))?;
        let meta = [("source", path.display().to_string())];
        let mut writer = SessionWriter::create(conn, &meta).map_err(|e| Error::Document(e.to_string()))?;
        for flow in flows {
            writer.add(flow)?;
        }
        let conn = writer.finish().map_err(|e| Error::Document(e.to_string()))?;
        let opened = SqliteBackend::open(conn)?;
        let info = self.docs.add(kind, path, opened, self.operations())?;
        self.columns_for_document(info.store);
        Ok(info)
    }

    /// Closes a document; its views stop.
    pub fn close_document(&self, store: StoreId) -> Result<(), Error> {
        let doc = self
            .docs
            .open
            .write()
            .remove(&store)
            .ok_or(Error::UnknownStore(store))?;
        self.sinks_retain(|view| view >> VIEW_STORE_SHIFT != doc.info.store);
        Ok(())
    }

    /// Points every store's later filters at a new set of OpenAPI specs.
    pub(crate) fn set_store_operations(&self, operations: Arc<crate::openapi::OpIndex>) {
        self.store.write().set_operations(Some(operations.clone()));
        for doc in self.docs.open.read().values() {
            doc.store.write().set_operations(Some(operations.clone()));
        }
    }

    /// The opened documents, in the order they were opened.
    pub fn documents(&self) -> Vec<DocumentInfo> {
        self.docs.open.read().values().map(|d| d.info.clone()).collect()
    }

    /// The flows of `store` that `selection` names, in its order.
    pub fn selected_flows(&self, store: StoreId, selection: &FlowSelection) -> Result<Vec<FlowSummary>, Error> {
        let pick = |s: &Store| -> Result<Vec<FlowSummary>, Error> {
            match selection {
                FlowSelection::All => Ok(s.flows().to_vec()),
                FlowSelection::View { view, except } => {
                    if view >> VIEW_STORE_SHIFT != store {
                        return Err(Error::UnknownView(*view));
                    }
                    let mut flows = s.view_flows(*view)?;
                    if !except.is_empty() {
                        let except: std::collections::HashSet<FlowId> = except.iter().copied().collect();
                        flows.retain(|f| !except.contains(&f.id));
                    }
                    Ok(flows)
                }
                FlowSelection::Ids { ids } => Ok(ids.iter().filter_map(|&id| s.get(id).cloned()).collect()),
            }
        };
        match store {
            LIVE_STORE => pick(&self.store.read()),
            _ => pick(&self.docs.get(store)?.store.read()),
        }
    }

    /// Saves flows of `store` as a `.proxymate` session at `path`, on a
    /// background thread. Returns how many flows will be written; `progress`
    /// hears how it goes and when it is done.
    pub fn save_session(
        self: &Arc<Self>,
        store: StoreId,
        selection: &FlowSelection,
        path: PathBuf,
        progress: ProgressSink,
    ) -> Result<u32, Error> {
        let flows = self.selected_flows(store, selection)?;
        let total = flows.len() as u32;
        let engine = self.clone();
        self.spawn_save(path, total, progress, move |path, report| {
            let source = flows.into_iter().map(|summary| {
                engine
                    .with_backend(store, |backend| Ok(session::collect(backend, summary)?))
                    .map_err(io::Error::other)
            });
            session::write(path, source, report)
        })
    }

    /// Exports flows of `store` as a HAR 1.2 file at `path`, on a background
    /// thread; like [`Engine::save_session`].
    pub fn export_har(
        self: &Arc<Self>,
        store: StoreId,
        selection: &FlowSelection,
        path: PathBuf,
        progress: ProgressSink,
    ) -> Result<u32, Error> {
        let flows = self.selected_flows(store, selection)?;
        let total = flows.len() as u32;
        let engine = self.clone();
        self.spawn_save(path, total, progress, move |path, report| {
            let source = flows.into_iter().map(|summary| {
                engine
                    .with_backend(store, |backend| Ok(session::collect(backend, summary)?))
                    .map_err(io::Error::other)
            });
            har::write(path, source, report)
        })
    }

    /// Flows of `store` as a HAR 1.2 document (JSON), built in memory: for
    /// agents and small selections. Files go through [`Engine::export_har`].
    pub fn har_json(&self, store: StoreId, selection: &FlowSelection) -> Result<String, Error> {
        let flows = self
            .selected_flows(store, selection)?
            .into_iter()
            .map(|summary| self.with_backend(store, |backend| Ok(session::collect(backend, summary)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        serde_json::to_string_pretty(&har::export(flows)).map_err(|e| Error::Document(e.to_string()))
    }

    pub(crate) fn spawn_save(
        &self,
        path: PathBuf,
        total: u32,
        mut progress: ProgressSink,
        write: impl FnOnce(&Path, &mut dyn FnMut(u32)) -> io::Result<u32> + Send + 'static,
    ) -> Result<u32, Error> {
        let shown = path.display().to_string();
        std::thread::Builder::new().name("pm-save".into()).spawn(move || {
            let update = |done: u32, finished: bool, error: Option<String>| SaveProgress {
                done,
                total,
                finished,
                error,
                path: shown.clone(),
            };
            progress(update(0, false, None));
            let result = write(&path, &mut |done| progress(update(done, false, None)));
            let last = match result {
                Ok(done) => update(done, true, None),
                Err(err) => update(0, true, Some(err.to_string())),
            };
            progress(last);
        })?;
        Ok(total)
    }
}
