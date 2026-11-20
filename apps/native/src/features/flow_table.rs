//! The flow table (G1): a [`FlowGrid`] over one engine view, ported from
//! `hooks/useFlowView.ts`. It creates the view over the store
//! (`create_view_in`), follows its 20 Hz deltas, applies filter and sort
//! changes (`set_filter`, `set_sort`: refiltered synchronously in the
//! engine, then the rows are fetched again), deletes the selection,
//! persists the column layout and pushes the header columns to the engine.
//!
//! The engine is touched only off the foreground thread; its delta sink
//! only enqueues, and one foreground task drains the queue into the grid.
//! A table can also show a fixed snapshot of rows (`FlowTable::snapshot`),
//! which is how tests render it without an engine.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, Global, SharedString, Subscription, WeakEntity, Window, canvas, div};
use pm_core::Engine;
use pm_model::{DeepProgress, FilterSpec, FlowRow, LIVE_STORE, SortSpec, StoreId, ViewDelta, ViewId};
use pm_widgets::flow_grid::{self, FlowGrid, FlowGridEvent, Overlay, RowSource};

use crate::bridge::Bridge;

pub fn register(cx: &mut App) {
    flow_grid::bind_keys(cx);
}

/// Where the rows come from.
#[derive(Clone)]
pub enum Backend {
    Engine(Arc<Engine>),
    /// A fixed list (tests, offscreen renders); `total` flows in the store.
    Snapshot {
        rows: Arc<Vec<FlowRow>>,
        total: usize,
    },
}

/// How long the last filter change took: in the engine, and until the
/// first frame with its rows was painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterTiming {
    pub engine_ms: f64,
    pub end_to_end_ms: f64,
    pub rows: usize,
}

struct PendingFilter {
    started: Instant,
    engine_ms: f64,
    rows: usize,
    /// The new rows (or an empty view) are in: time the next paint.
    ready: bool,
}

pub struct FlowTable {
    backend: Backend,
    store: StoreId,
    grid: Entity<FlowGrid>,
    view: Option<ViewId>,
    filter: FilterSpec,
    total: usize,
    deep: Option<DeepProgress>,
    error: Option<SharedString>,
    proxy_addr: SharedString,
    /// The header columns the engine was last given.
    header_columns: Vec<pm_model::HeaderColumn>,
    filter_seq: u64,
    sort_seq: u64,
    pending_filter: Option<PendingFilter>,
    filter_timing: Option<FilterTiming>,
    _subscriptions: Vec<Subscription>,
}

/// The tables open now, for the dev remote and the perf probe.
#[derive(Default)]
pub struct Tables(Vec<WeakEntity<FlowTable>>);

impl Global for Tables {}

/// The first live table.
pub fn first(cx: &App) -> Option<Entity<FlowTable>> {
    cx.try_global::<Tables>()?.0.iter().find_map(WeakEntity::upgrade)
}

impl FlowTable {
    /// The live capture's table, over the bridge's engine.
    pub fn live(proxy_addr: SharedString, cx: &mut Context<Self>) -> Self {
        let engine = Bridge::engine(cx);
        Self::new(Backend::Engine(engine), proxy_addr, cx)
    }

    /// A table over fixed rows: no engine, no deltas.
    pub fn snapshot(rows: Vec<FlowRow>, total: usize, cx: &mut Context<Self>) -> Self {
        Self::new(
            Backend::Snapshot {
                rows: Arc::new(rows),
                total,
            },
            "127.0.0.1:9090".into(),
            cx,
        )
    }

    fn new(backend: Backend, proxy_addr: SharedString, cx: &mut Context<Self>) -> Self {
        let prefs = crate::prefs::get(cx).table.clone();
        let header_columns = prefs.header_columns.clone();
        let grid = cx.new(|cx| FlowGrid::new(prefs, cx));
        if let Backend::Engine(engine) = &backend {
            let engine = engine.clone();
            grid.update(cx, |g, _| {
                g.set_icon_source(Arc::new(move |name: &str| engine.app_icon(name)));
            });
        }
        let subscriptions = vec![
            cx.subscribe(&grid, Self::on_grid_event),
            cx.on_release(|this: &mut FlowTable, _| {
                if let (Backend::Engine(engine), Some(view)) = (&this.backend, this.view) {
                    engine.drop_view(view);
                }
            }),
        ];
        let weak = cx.weak_entity();
        let tables = &mut cx.default_global::<Tables>().0;
        tables.retain(|t| t.upgrade().is_some());
        tables.push(weak);
        let mut table = FlowTable {
            backend,
            store: LIVE_STORE,
            grid,
            view: None,
            filter: FilterSpec::default(),
            total: 0,
            deep: None,
            error: None,
            proxy_addr,
            header_columns: Vec::new(),
            filter_seq: 0,
            sort_seq: 0,
            pending_filter: None,
            filter_timing: None,
            _subscriptions: subscriptions,
        };
        table.push_header_columns(header_columns, cx);
        table.open_view(cx);
        if crate::dev_env::flag("PROXYMATE_DEV_AUTO_SELECT") {
            table.grid.update(cx, |g, cx| g.set_auto_select(true, cx));
        }
        table
    }

    pub fn grid(&self) -> &Entity<FlowGrid> {
        &self.grid
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn filter(&self) -> &FilterSpec {
        &self.filter
    }

    pub fn filter_timing(&self) -> Option<FilterTiming> {
        self.filter_timing
    }

    /// Creates the view (again, after an error) and follows its deltas.
    fn open_view(&mut self, cx: &mut Context<Self>) {
        let sort = self.grid.read(cx).prefs().sort;
        match self.backend.clone() {
            Backend::Snapshot { rows, total } => {
                self.total = total;
                let len = rows.len();
                let source: RowSource = Arc::new(move |start, count| {
                    let end = (start + count).min(rows.len());
                    Ok(rows.get(start..end).unwrap_or_default().to_vec())
                });
                self.grid.update(cx, |g, cx| g.set_source(1, source, len, cx));
                self.update_overlay(cx);
            }
            Backend::Engine(engine) => {
                let (store, filter) = (self.store, self.filter.clone());
                let created = cx.background_spawn({
                    let engine = engine.clone();
                    async move { engine.create_view_in(store, filter, sort) }
                });
                cx.spawn(async move |this, cx| {
                    let info = created.await;
                    let _ = this.update(cx, |this, cx| match info {
                        Ok(info) => this.view_opened(engine, info, cx),
                        Err(err) => {
                            this.error = Some(err.to_string().into());
                            this.update_overlay(cx);
                        }
                    });
                })
                .detach();
            }
        }
    }

    fn view_opened(&mut self, engine: Arc<Engine>, info: pm_model::ViewInfo, cx: &mut Context<Self>) {
        let view = info.view;
        self.view = Some(view);
        self.total = info.total as usize;
        self.deep = info.deep;
        let source_engine = engine.clone();
        let source: RowSource = Arc::new(move |start, count| {
            source_engine
                .get_rows(view, start as u32, count as u32)
                .map(|page| page.rows)
                .map_err(|e| e.to_string())
        });
        self.grid
            .update(cx, |g, cx| g.set_source(view, source, info.len as usize, cx));
        // The sink only enqueues; this task drains the queue into the grid.
        let (tx, rx) = async_channel::unbounded::<ViewDelta>();
        if let Err(err) = engine.subscribe(view, Box::new(move |delta| tx.try_send(delta).is_ok())) {
            self.error = Some(err.to_string().into());
        }
        engine.set_sidebar_store(self.store);
        cx.spawn(async move |this, cx| {
            while let Ok(delta) = rx.recv().await {
                let mut batch = vec![delta];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this.update(cx, |this, cx| {
                    for delta in &batch {
                        this.on_delta(delta, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        self.update_overlay(cx);
    }

    fn on_delta(&mut self, delta: &ViewDelta, cx: &mut Context<Self>) {
        if Some(delta.view) != self.view {
            return;
        }
        self.total = delta.total as usize;
        self.deep = delta.deep;
        if delta.total == 0 && self.grid.read(cx).selected().focused.is_some() {
            // Cleared.
            self.grid.update(cx, |g, cx| g.select(None, cx));
        }
        self.grid.update(cx, |g, cx| g.apply_delta(delta, cx));
        self.update_overlay(cx);
    }

    /// Refilters (synchronously in the engine, off this thread), then the
    /// grid fetches the new rows. Only the latest filter's answer applies.
    pub fn set_filter(&mut self, filter: FilterSpec, cx: &mut Context<Self>) {
        self.filter = filter.clone();
        self.filter_seq += 1;
        let seq = self.filter_seq;
        let started = Instant::now();
        let (Backend::Engine(engine), Some(view)) = (self.backend.clone(), self.view) else {
            return;
        };
        let done = cx.background_spawn(async move { engine.set_filter(view, filter) });
        cx.spawn(async move |this, cx| {
            let info = done.await;
            let _ = this.update(cx, |this, cx| {
                if seq != this.filter_seq {
                    return;
                }
                match info {
                    Ok(info) => {
                        let rows = info.len as usize;
                        this.total = info.total as usize;
                        this.deep = info.deep;
                        this.pending_filter = Some(PendingFilter {
                            started,
                            engine_ms: info.elapsed_us as f64 / 1000.0,
                            rows,
                            ready: rows == 0,
                        });
                        this.grid.update(cx, |g, cx| g.reset(rows, cx));
                        this.update_overlay(cx);
                        cx.notify();
                    }
                    Err(err) => eprintln!("flow table: can't apply the filter: {err}"),
                }
            });
        })
        .detach();
    }

    fn set_sort(&mut self, sort: SortSpec, cx: &mut Context<Self>) {
        self.sort_seq += 1;
        let seq = self.sort_seq;
        let (Backend::Engine(engine), Some(view)) = (self.backend.clone(), self.view) else {
            return;
        };
        let done = cx.background_spawn(async move { engine.set_sort(view, sort) });
        cx.spawn(async move |this, cx| {
            let info = done.await;
            let _ = this.update(cx, |this, cx| {
                if seq != this.sort_seq {
                    return;
                }
                match info {
                    Ok(info) => this.grid.update(cx, |g, cx| g.reset(info.len as usize, cx)),
                    Err(err) => eprintln!("flow table: can't sort the flows: {err}"),
                }
            });
        })
        .detach();
    }

    /// Header columns are global in the engine; every store fills them.
    fn push_header_columns(&mut self, columns: Vec<pm_model::HeaderColumn>, cx: &mut Context<Self>) {
        if columns == self.header_columns {
            return;
        }
        self.header_columns = columns.clone();
        let Backend::Engine(engine) = self.backend.clone() else {
            return;
        };
        cx.background_spawn(async move {
            if let Err(err) = engine.set_header_columns(columns) {
                eprintln!("flow table: header columns: {err}");
            }
        })
        .detach();
    }

    /// Sets the header columns (the dev remote; G11's dialog).
    pub fn set_header_columns(&mut self, columns: Vec<pm_model::HeaderColumn>, cx: &mut Context<Self>) {
        let mut prefs = self.grid.read(cx).prefs().clone();
        for id in prefs.header_ids().map(|(_, id)| id).collect::<Vec<_>>() {
            prefs.remove_header_column(&id);
        }
        for column in columns {
            prefs.add_header_column(column);
        }
        let sort = prefs.sort;
        self.grid.update(cx, |g, cx| g.set_prefs(prefs, cx));
        self.prefs_changed(cx);
        if sort != self.grid.read(cx).prefs().sort {
            self.set_sort(self.grid.read(cx).prefs().sort, cx);
        }
    }

    fn prefs_changed(&mut self, cx: &mut Context<Self>) {
        let table = self.grid.read(cx).prefs().clone();
        let columns = table.header_columns.clone();
        crate::prefs::update(cx, |p| p.table = table);
        self.push_header_columns(columns, cx);
    }

    fn on_grid_event(&mut self, _: Entity<FlowGrid>, event: &FlowGridEvent, cx: &mut Context<Self>) {
        match event {
            FlowGridEvent::SelectionChanged | FlowGridEvent::Inspect => cx.notify(),
            FlowGridEvent::SortChanged(sort) => self.set_sort(*sort, cx),
            FlowGridEvent::PrefsChanged => self.prefs_changed(cx),
            FlowGridEvent::Delete(selection) => self.delete(selection.clone(), cx),
            FlowGridEvent::Retry => {
                self.error = None;
                self.open_view(cx);
            }
            FlowGridEvent::RowsLanded => {
                if let Some(pending) = &mut self.pending_filter
                    && !pending.ready
                {
                    pending.ready = true;
                    cx.notify();
                }
            }
        }
    }

    fn delete(&mut self, selection: pm_model::FlowSelection, cx: &mut Context<Self>) {
        let Backend::Engine(engine) = self.backend.clone() else {
            return;
        };
        let store = self.store;
        let removed = cx.background_spawn(async move { engine.remove_flows_in(store, &selection) });
        cx.spawn(async move |this, cx| match removed.await {
            Ok(n) if n > 0 => {
                let _ = this.update(cx, |this, cx| this.grid.update(cx, |g, cx| g.select(None, cx)));
            }
            Ok(_) => {}
            Err(err) => eprintln!("flow table: delete failed: {err}"),
        })
        .detach();
    }

    fn update_overlay(&mut self, cx: &mut Context<Self>) {
        let len = self.grid.read(cx).len();
        let overlay = if let Some(err) = &self.error {
            Overlay::Error(err.clone())
        } else if len > 0 {
            Overlay::None
        } else if self.deep.is_some_and(|d| d.running) {
            Overlay::Searching
        } else if self.total > 0 {
            Overlay::NoMatch
        } else {
            Overlay::Waiting {
                command: format!("curl -x http://{} http://example.com/", self.proxy_addr).into(),
            }
        };
        self.grid.update(cx, |g, cx| g.set_overlay(overlay, cx));
    }

    /// The frame with the new filter's rows was painted.
    fn painted(&mut self, at: Instant, cx: &mut Context<Self>) {
        let Some(p) = self.pending_filter.take() else { return };
        let timing = FilterTiming {
            engine_ms: p.engine_ms,
            end_to_end_ms: at.duration_since(p.started).as_secs_f64() * 1000.0,
            rows: p.rows,
        };
        if crate::dev_env::enabled() {
            eprintln!(
                "filter: engine {:.2} ms, end to end {:.1} ms ({} rows)",
                timing.engine_ms, timing.end_to_end_ms, timing.rows
            );
        }
        self.filter_timing = Some(timing);
        cx.notify();
    }
}

impl Render for FlowTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // After a filter change, a zero-size canvas painted in the same frame
        // as the new rows (after them) times the change end to end.
        let probe = self.pending_filter.as_ref().filter(|p| p.ready).map(|_| {
            let weak = cx.weak_entity();
            let painted = Rc::new(Cell::new(false));
            canvas(
                |_, _, _| {},
                move |_, _, _, cx| {
                    if painted.replace(true) {
                        return;
                    }
                    let at = Instant::now();
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |t, cx| t.painted(at, cx));
                    });
                },
            )
            .absolute()
            .size_0()
        });
        div().size_full().relative().child(self.grid.clone()).children(probe)
    }
}
