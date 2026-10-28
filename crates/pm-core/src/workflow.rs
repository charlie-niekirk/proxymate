//! Everyday workflow (M17), as `Engine` methods: deleting flows, pins,
//! labels and comments in any store, header columns (filled as flows
//! arrive and backfilled in the background), Copy as code and Repeat.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use pm_model::{
    BodyPart, CodeLanguage, FlowId, FlowLabel, FlowSelection, FlowState, HeaderColumn, LIVE_STORE, MAX_HEADER_COLUMNS,
    RepeatId, RepeatOpts, RepeatProgress, RequestSpec, SendOpts, SendOrigin, StatusCount, StoreId,
};

use crate::ingest::FlowEvent;
use crate::io::snippets;
use crate::store::{BlobBackend, Store};
use crate::{Engine, Error};

/// Header column values are cut to this many characters.
const VALUE_MAX: usize = 256;
/// A backfill reads this many details at a time.
const BACKFILL_BATCH: usize = 512;
/// Repeat progress is pushed at most this often (and once at the end).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
pub const MAX_REPEAT_ITERATIONS: u32 = 10_000;
pub const MAX_REPEAT_CONCURRENCY: u32 = 64;
const MAX_REPEAT_DELAY_MS: u32 = 60_000;
/// A comment longer than this is refused.
const COMMENT_MAX: usize = 10_000;

/// Receives a repeat's progress; the last update has `finished` set.
pub type RepeatSink = Box<dyn FnMut(RepeatProgress) + Send>;

/// The workflow state the engine keeps.
#[derive(Default)]
pub(crate) struct Workflow {
    columns: RwLock<ColumnSet>,
    next_repeat: AtomicU32,
    repeats: Mutex<HashMap<RepeatId, Arc<AtomicBool>>>,
}

#[derive(Default, Clone)]
struct ColumnSet {
    /// Bumped on every change; values carry it so stale ones are dropped.
    generation: u32,
    columns: Arc<Vec<HeaderColumn>>,
}

impl Engine {
    /// The ids `selection` names in `store`, in its order.
    pub fn selected_ids(&self, store: StoreId, selection: &FlowSelection) -> Result<Vec<FlowId>, Error> {
        Ok(self
            .selected_flows(store, selection)?
            .into_iter()
            .map(|f| f.id)
            .collect())
    }

    /// Deletes flows from `store`, pinned ones too; flows still in flight
    /// stay. Live flows' details go, and segment files no flow left uses
    /// are deleted. A document's file isn't touched: only its tab changes.
    /// Returns how many flows went.
    pub fn remove_flows_in(&self, store: StoreId, selection: &FlowSelection) -> Result<u32, Error> {
        let ids: HashSet<FlowId> = self.selected_ids(store, selection)?.into_iter().collect();
        if ids.is_empty() {
            return Ok(0);
        }
        if store != LIVE_STORE {
            let doc = self.docs.get(store)?;
            return Ok(doc.store.write().remove(&ids).len() as u32);
        }
        // Queued events go first, so a flow that just finished is seen as such.
        self.flush();
        let removed = {
            let mut live = self.store.write();
            let removed = live.remove(&ids);
            self.details().remove(&removed, |low, high| live.has_flow_in(low, high));
            removed
        };
        self.interned().lock().retain(|s| Arc::strong_count(s) > 1);
        Ok(removed.len() as u32)
    }

    /// Pins (or unpins) flows of any store. Pinned live flows are never
    /// evicted; a saved session keeps its pins.
    pub fn pin_flows_in(&self, store: StoreId, selection: &FlowSelection, pinned: bool) -> Result<u32, Error> {
        let ids = self.selected_ids(store, selection)?;
        let n = ids.len() as u32;
        self.annotate(store, FlowEvent::Pin { ids, pinned })?;
        Ok(n)
    }

    /// Puts a colour label on flows of any store; `None` takes it off.
    pub fn label_flows_in(
        &self,
        store: StoreId,
        selection: &FlowSelection,
        label: Option<FlowLabel>,
    ) -> Result<u32, Error> {
        let ids = self.selected_ids(store, selection)?;
        let n = ids.len() as u32;
        self.annotate(store, FlowEvent::Label { ids, label })?;
        Ok(n)
    }

    /// Sets the note on a flow of any store; an empty one removes it.
    pub fn comment_flow_in(&self, store: StoreId, id: FlowId, comment: &str) -> Result<(), Error> {
        if self.get_flow_in(store, id).is_none() {
            return Err(Error::UnknownFlow(id));
        }
        let comment = comment.trim();
        if comment.chars().count() > COMMENT_MAX {
            return Err(Error::Invalid(format!(
                "a comment can be at most {COMMENT_MAX} characters"
            )));
        }
        let comment = (!comment.is_empty()).then(|| Arc::from(comment));
        self.annotate(store, FlowEvent::Comment { id, comment })
    }

    /// Applies a user annotation: through ingest for the live store (so it
    /// orders with the flow's own events), directly for a document.
    fn annotate(&self, store: StoreId, event: FlowEvent) -> Result<(), Error> {
        match store {
            LIVE_STORE => {
                self.ingest().send(event);
                self.flush();
            }
            _ => {
                self.docs.get(store)?.store.write().apply([event]);
            }
        }
        Ok(())
    }

    // ---- Header columns ----

    /// The header columns every store fills.
    pub fn header_columns(&self) -> Vec<HeaderColumn> {
        self.workflow.columns.read().columns.to_vec()
    }

    /// Sets the header columns (up to [`MAX_HEADER_COLUMNS`]). Every store's
    /// values are dropped and backfilled in the background from the flows'
    /// details; new flows fill theirs as their heads arrive.
    pub fn set_header_columns(self: &Arc<Self>, columns: Vec<HeaderColumn>) -> Result<Vec<HeaderColumn>, Error> {
        if columns.len() > MAX_HEADER_COLUMNS {
            return Err(Error::Invalid(format!("at most {MAX_HEADER_COLUMNS} header columns")));
        }
        let mut cleaned: Vec<HeaderColumn> = Vec::with_capacity(columns.len());
        for column in columns {
            let name = column.name.trim().to_ascii_lowercase();
            if http::HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(Error::Invalid(format!("{:?} isn't a header name", column.name)));
            }
            let column = HeaderColumn {
                part: column.part,
                name,
            };
            if !cleaned.contains(&column) {
                cleaned.push(column);
            }
        }
        let generation = {
            let mut set = self.workflow.columns.write();
            if *set.columns == cleaned {
                return Ok(cleaned);
            }
            set.generation += 1;
            set.columns = Arc::new(cleaned.clone());
            set.generation
        };
        self.ingest().send(FlowEvent::Columns { generation });
        self.flush();
        self.spawn_backfill(LIVE_STORE, generation);
        for doc in self.documents() {
            self.start_columns(doc.store, generation);
        }
        Ok(cleaned)
    }

    /// Fills a newly opened document's header columns.
    pub(crate) fn columns_for_document(self: &Arc<Self>, store: StoreId) {
        let set = self.workflow.columns.read().clone();
        if !set.columns.is_empty() {
            self.start_columns(store, set.generation);
        }
    }

    fn start_columns(self: &Arc<Self>, store: StoreId, generation: u32) {
        if let Ok(doc) = self.docs.get(store) {
            doc.store.write().apply([FlowEvent::Columns { generation }]);
            self.spawn_backfill(store, generation);
        }
    }

    /// A flow's values for the columns of `part`, from its headers, or
    /// `None` when no column is about that part. For the recorder.
    pub(crate) fn header_cells(&self, id: FlowId, part: BodyPart, headers: &[pm_model::Header]) -> Option<FlowEvent> {
        let set = self.workflow.columns.read();
        if !set.columns.iter().any(|c| c.part == part) {
            return None;
        }
        Some(FlowEvent::Extra {
            generation: set.generation,
            values: vec![(id, cells(&set.columns, part, headers, true))],
        })
    }

    /// Fills the columns of every flow in `store` from its detail, a batch
    /// at a time, until done or the columns change again.
    fn spawn_backfill(self: &Arc<Self>, store: StoreId, generation: u32) {
        let engine = Arc::downgrade(self);
        let spawned = std::thread::Builder::new().name("pm-columns".into()).spawn(move || {
            let mut after: Option<FlowId> = None;
            loop {
                let Some(engine) = engine.upgrade() else {
                    return;
                };
                let set = engine.workflow.columns.read().clone();
                if set.generation != generation {
                    return;
                }
                let batch = engine.read_store(store, |s: &Store| {
                    if s.extra_generation() != generation {
                        return Vec::new();
                    }
                    let from = s.flows().partition_point(|f| after.is_some_and(|a| f.id <= a));
                    s.flows()[from..].iter().take(BACKFILL_BATCH).map(|f| f.id).collect()
                });
                let Ok(batch) = batch else {
                    return;
                };
                let Some(&last) = batch.last() else {
                    return;
                };
                after = Some(last);
                let values = engine
                    .with_backend(store, |backend| Ok(backfill_cells(backend, &set.columns, &batch)))
                    .unwrap_or_default();
                if values.is_empty() {
                    continue;
                }
                let event = FlowEvent::Extra { generation, values };
                match store {
                    LIVE_STORE => engine.ingest().send(event),
                    _ => match engine.docs.get(store) {
                        Ok(doc) => {
                            doc.store.write().apply([event]);
                        }
                        Err(_) => return,
                    },
                }
            }
        });
        if let Err(err) = spawned {
            eprintln!("proxymate: can't fill the header columns: {err}");
        }
    }

    // ---- Copy as code ----

    /// A recorded request as a snippet in `language` that sends it again.
    pub fn code_snippet(&self, store: StoreId, id: FlowId, language: CodeLanguage) -> Result<String, Error> {
        if language == CodeLanguage::Curl {
            return self.curl_command(store, id);
        }
        let spec = self.request_spec(store, id, true)?;
        Ok(snippets::render(language, &spec))
    }

    /// [`Engine::code_snippet`] for a request that hasn't been sent.
    pub fn code_snippet_for(&self, spec: &RequestSpec, language: CodeLanguage) -> String {
        snippets::render(language, spec)
    }

    // ---- Repeat ----

    /// Sends `spec` `opts.iterations` times, `opts.concurrency` at a time,
    /// through [`Engine::send`] (each one a live flow marked
    /// `SentBy::Repeat`). Returns at once; `sink` hears the counts as they
    /// go, and last with `finished` set. [`Engine::cancel_repeat`] stops it.
    pub fn repeat(self: &Arc<Self>, spec: RequestSpec, opts: RepeatOpts, sink: RepeatSink) -> Result<RepeatId, Error> {
        if !(1..=MAX_REPEAT_ITERATIONS).contains(&opts.iterations) {
            return Err(Error::Invalid(format!(
                "iterations must be 1 to {MAX_REPEAT_ITERATIONS}"
            )));
        }
        if !(1..=MAX_REPEAT_CONCURRENCY).contains(&opts.concurrency) {
            return Err(Error::Invalid(format!(
                "concurrency must be 1 to {MAX_REPEAT_CONCURRENCY}"
            )));
        }
        if opts.delay_ms > MAX_REPEAT_DELAY_MS {
            return Err(Error::Invalid(format!(
                "the delay can be at most {MAX_REPEAT_DELAY_MS} ms"
            )));
        }
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| Error::Send("repeat needs the async runtime".into()))?;
        let id = self.workflow.next_repeat.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.workflow.repeats.lock().insert(id, cancel.clone());
        let tally = Arc::new(Mutex::new(Tally::new(id, opts.iterations, sink)));
        let next = Arc::new(AtomicU32::new(0));
        let workers = opts.concurrency.min(opts.iterations);
        let engine = self.clone();
        runtime.spawn(async move {
            let mut tasks = Vec::with_capacity(workers as usize);
            for _ in 0..workers {
                let (engine, spec, tally, next, cancel) = (
                    engine.clone(),
                    spec.clone(),
                    tally.clone(),
                    next.clone(),
                    cancel.clone(),
                );
                let opts = opts.clone();
                tasks.push(tokio::spawn(async move {
                    loop {
                        if cancel.load(Ordering::Relaxed) || next.fetch_add(1, Ordering::Relaxed) >= opts.iterations {
                            return;
                        }
                        if opts.delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(opts.delay_ms.into())).await;
                        }
                        let send = SendOpts {
                            apply_rules: opts.apply_rules,
                            wait: true,
                            origin: SendOrigin::User,
                        };
                        match engine.send_marked(spec.clone(), send, pm_model::SentBy::Repeat).await {
                            Ok(flow) => {
                                let summary = engine.get_flow(flow);
                                tally.lock().record(flow, summary.as_ref());
                            }
                            Err(err) => {
                                // The spec itself is bad: every try would fail the same way.
                                cancel.store(true, Ordering::Relaxed);
                                tally.lock().error = Some(err.to_string());
                                return;
                            }
                        }
                    }
                }));
            }
            for task in tasks {
                let _ = task.await;
            }
            engine.workflow.repeats.lock().remove(&id);
            tally.lock().finish(cancel.load(Ordering::Relaxed));
        });
        Ok(id)
    }

    /// Stops a running repeat; requests in flight finish. False if it had
    /// already ended.
    pub fn cancel_repeat(&self, id: RepeatId) -> bool {
        match self.workflow.repeats.lock().get(&id) {
            Some(cancel) => {
                cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// The values for `columns` of `part` from `headers`, as `(column, value)`
/// pairs; with `all`, absent headers are `None` pairs too.
fn cells(
    columns: &[HeaderColumn],
    part: BodyPart,
    headers: &[pm_model::Header],
    all: bool,
) -> Vec<(u8, Option<Arc<str>>)> {
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.part == part)
        .filter_map(|(i, c)| {
            let mut value: Option<String> = None;
            for h in headers.iter().filter(|h| h.name.eq_ignore_ascii_case(&c.name)) {
                match &mut value {
                    Some(v) => {
                        v.push_str(", ");
                        v.push_str(&h.value);
                    }
                    None => value = Some(h.value.clone()),
                }
            }
            let value = value.map(|v| match v.char_indices().nth(VALUE_MAX) {
                Some((at, _)) => Arc::from(format!("{}…", &v[..at])),
                None => Arc::from(v),
            });
            (all || value.is_some()).then_some((i as u8, value))
        })
        .collect()
}

/// Values found for `ids` (only the headers present, so a backfill never
/// clears what a recorder set meanwhile).
fn backfill_cells(
    backend: &dyn BlobBackend,
    columns: &[HeaderColumn],
    ids: &[FlowId],
) -> Vec<(FlowId, Vec<(u8, Option<Arc<str>>)>)> {
    use rayon::prelude::*;
    ids.par_iter()
        .filter_map(|&id| {
            let detail = backend.detail(id).ok()??;
            let mut found = cells(columns, BodyPart::Request, &detail.request.headers, false);
            if let Some(response) = &detail.response {
                found.extend(cells(columns, BodyPart::Response, &response.headers, false));
            }
            (!found.is_empty()).then_some((id, found))
        })
        .collect()
}

/// A repeat's running counts.
struct Tally {
    progress: RepeatProgress,
    statuses: BTreeMap<Option<u16>, u32>,
    durations: Vec<u32>,
    error: Option<String>,
    sink: RepeatSink,
    last_push: Instant,
}

impl Tally {
    fn new(id: RepeatId, total: u32, mut sink: RepeatSink) -> Tally {
        let progress = RepeatProgress {
            id,
            total,
            ..Default::default()
        };
        sink(progress.clone());
        Tally {
            progress,
            statuses: BTreeMap::new(),
            durations: Vec::with_capacity(total as usize),
            error: None,
            sink,
            last_push: Instant::now(),
        }
    }

    fn record(&mut self, flow: FlowId, summary: Option<&pm_model::FlowSummary>) {
        let p = &mut self.progress;
        p.done += 1;
        p.first_flow = Some(p.first_flow.map_or(flow, |f| f.min(flow)));
        p.last_flow = Some(p.last_flow.map_or(flow, |f| f.max(flow)));
        let status = summary.filter(|s| s.state != FlowState::Failed).and_then(|s| s.status);
        if status.is_none() {
            p.failed += 1;
        }
        *self.statuses.entry(status).or_default() += 1;
        if let Some(ms) = summary.and_then(|s| s.duration_ms) {
            self.durations.push(ms);
        }
        if self.last_push.elapsed() >= PROGRESS_EVERY {
            self.push();
        }
    }

    fn push(&mut self) {
        self.last_push = Instant::now();
        self.progress.statuses = self
            .statuses
            .iter()
            .map(|(&status, &count)| StatusCount { status, count })
            .collect();
        let mut sorted = self.durations.clone();
        sorted.sort_unstable();
        let at = |q: f64| -> Option<u32> {
            let last = sorted.len().checked_sub(1)?;
            sorted.get(((last as f64) * q).round() as usize).copied()
        };
        self.progress.min_ms = sorted.first().copied();
        self.progress.p50_ms = at(0.5);
        self.progress.p95_ms = at(0.95);
        self.progress.max_ms = sorted.last().copied();
        self.progress.error = self.error.clone();
        (self.sink)(self.progress.clone());
    }

    fn finish(&mut self, cancelled: bool) {
        self.progress.finished = true;
        self.progress.cancelled = cancelled && self.error.is_none();
        self.push();
    }
}
