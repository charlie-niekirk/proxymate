//! FlowGrid: the flow table, a `uniform_list` of 22 px rows over a
//! [`RowCache`] (ported from `FlowTable.tsx`, `FlowRow.tsx` and `Header.tsx`).
//!
//! The grid knows no engine. Its owner hands it a [`RowSource`] (a blocking
//! `get_rows` for one view, run on gpui's background executor) and feeds it
//! the view's deltas ([`FlowGrid::apply_delta`]); the grid fetches only the
//! visible window plus a page of read-ahead, coalescing the asks of a frame
//! into one fetch per run of adjacent pages. Rows are formatted once, when
//! their page lands ([`GridRow`]), and stale rows stay on screen until their
//! refetch replaces them.
//!
//! Selection is by flow id, or the whole view with exceptions
//! ([`pm_view::selection`]): click, ⌘-click toggles, ⇧-click selects the
//! range from the anchor. Keys are [`pm_view::keys`]: arrows, PgUp/PgDn,
//! Home/End (⇧ extends), Enter (inspect), Escape (back to the focused
//! flow), ⌘A ([`SelectAll`]) and ⌫ ([`DeleteSelection`]).
//!
//! The header sorts (click a title), resizes (drag a column's right edge,
//! double-click resets, ←/→/Home/End/↩ when the edge is focused), reorders
//! (drag a header onto another) and opens the column menu (right-click:
//! show and hide columns, remove a header column, Reset columns). Layout
//! changes come out as [`FlowGridEvent::PrefsChanged`]; the owner persists
//! them.

mod header;
mod icons;
pub mod row;
#[cfg(test)]
mod tests;

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, KeyBinding, KeyDownEvent, Role, ScrollHandle, ScrollStrategy,
    SharedString, Task, UniformListScrollHandle, Window, div, px, uniform_list,
};
use pm_model::{FlowId, FlowRow, FlowSelection, SortSpec, ViewDelta, ViewId};
use pm_theme::kit::TokensExt as _;
use pm_view::columns::{ColumnId, HEADER_HEIGHT, PlacedColumn, ROW_HEIGHT, TablePrefs, min_table_width};
use pm_view::keys::{Key, TableAction, TableKey, TableState, table_key};
use pm_view::row_cache::{Fetch, MAX_ROWS_PER_CALL, RowCache};
use pm_view::selection::{Selected, TableSelection};

pub use icons::IconSource;
pub use row::GridRow;

/// A blocking `get_rows(start, count)` over the grid's view (at most
/// [`MAX_ROWS_PER_CALL`] rows per call), run off the foreground thread.
pub type RowSource = Arc<dyn Fn(usize, usize) -> Result<Vec<FlowRow>, String> + Send + Sync>;

/// The key context of the grid, for bindings.
pub const KEY_CONTEXT: &str = "FlowGrid";

/// ⌘A: every flow of the view (without listing them).
#[derive(Clone, PartialEq, Debug, Default, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct SelectAll;

/// ⌫: delete the selected flows (the owner does it).
#[derive(Clone, PartialEq, Debug, Default, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct DeleteSelection;

/// The column menu's show/hide item.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct ToggleColumn {
    pub id: ColumnId,
}

/// The column menu's "Remove “x” column".
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct RemoveHeaderColumn {
    pub id: ColumnId,
}

#[derive(Clone, PartialEq, Debug, Default, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct ResetColumns;

/// "Add header column…": the dialog arrives in G11.
#[derive(Clone, PartialEq, Debug, Default, gpui_kit::Action)]
#[action(namespace = flow_grid, no_json)]
pub struct AddHeaderColumn;

/// Binds the grid's keys (⌘A, ⌫) in its context.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("backspace", DeleteSelection, Some(KEY_CONTEXT)),
        KeyBinding::new("delete", DeleteSelection, Some(KEY_CONTEXT)),
    ]);
}

/// What the grid shows in place of rows.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    /// No flows yet: a command that sends one through the proxy.
    Waiting { command: SharedString },
    /// Flows, but none match the filter.
    NoMatch,
    /// A deep search is still looking.
    Searching,
    /// The view couldn't be opened: why, with Try again.
    Error(SharedString),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FlowGridEvent {
    /// The selection or its focused flow changed (read [`FlowGrid::selected`]).
    SelectionChanged,
    /// A header asked for this sort; the owner sorts the view and resets
    /// the grid with the new length.
    SortChanged(SortSpec),
    /// Columns were resized, moved, shown, hidden or reset, or header
    /// columns changed (read [`FlowGrid::prefs`]).
    PrefsChanged,
    /// Enter: into the inspector.
    Inspect,
    /// ⌫ with flows selected.
    Delete(FlowSelection),
    /// "Try again" under a view error.
    Retry,
    /// Rows of a fetch landed (the perf probe times repaints with it).
    RowsLanded,
    /// Right-click on the empty state's command, at this window position:
    /// the owner shows its "Copy command" menu.
    EmptyStateMenu(gpui_kit::Point<gpui_kit::Pixels>),
}

pub struct FlowGrid {
    focus: FocusHandle,
    view: Option<ViewId>,
    source: Option<RowSource>,
    cache: RowCache<GridRow>,
    prefs: TablePrefs,
    placed: Rc<Vec<PlacedColumn>>,
    selected: Selected,
    /// Where a ⇧-click or ⇧-arrow range starts.
    anchor: Option<usize>,
    /// The focused flow's row, while it still holds that flow.
    focus_index: Option<usize>,
    /// A keyboard move to a row that isn't loaded yet: selected once it is
    /// (with ⇧, the range to it).
    pending_index: Option<(usize, bool)>,
    scroll: UniformListScrollHandle,
    hscroll: ScrollHandle,
    /// The rows the list rendered last frame.
    rendered: Range<usize>,
    auto_select: bool,
    overlay: Overlay,
    icons: icons::IconCache,
    header: header::HeaderState,
    /// A ⇧-range still fetching ids beyond the loaded rows.
    range_task: Option<Task<()>>,
    /// Fetches landed so far.
    landed: u64,
}

impl EventEmitter<FlowGridEvent> for FlowGrid {}

impl Focusable for FlowGrid {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FlowGrid {
    pub fn new(prefs: TablePrefs, cx: &mut Context<Self>) -> Self {
        let placed = Rc::new(prefs.place());
        FlowGrid {
            focus: cx.focus_handle(),
            view: None,
            source: None,
            cache: RowCache::new(),
            prefs,
            placed,
            selected: Selected::default(),
            anchor: None,
            focus_index: None,
            pending_index: None,
            scroll: UniformListScrollHandle::new(),
            hscroll: ScrollHandle::new(),
            rendered: 0..0,
            auto_select: false,
            overlay: Overlay::None,
            icons: icons::IconCache::default(),
            header: header::HeaderState::default(),
            range_task: None,
            landed: 0,
        }
    }

    // ---- Data ----

    /// Shows a view: its rows come from `source`, and it has `len` rows now.
    pub fn set_source(&mut self, view: ViewId, source: RowSource, len: usize, cx: &mut Context<Self>) {
        self.view = Some(view);
        self.source = Some(source);
        self.anchor = None;
        self.focus_index = None;
        self.pending_index = None;
        self.reset(len, cx);
    }

    /// The view is gone (a store closed, an error): no rows until the next.
    pub fn clear_source(&mut self, cx: &mut Context<Self>) {
        self.view = None;
        self.source = None;
        self.cache = RowCache::new();
        cx.notify();
    }

    pub fn view(&self) -> Option<ViewId> {
        self.view
    }

    /// Rows were inserted, removed or reordered (a new filter or sort):
    /// every row is fetched again; the old ones stay until replaced.
    pub fn reset(&mut self, len: usize, cx: &mut Context<Self>) {
        if self.cache.reset(len) {
            self.schedule_flush(cx);
        }
        // Indexes no longer mean what they did.
        self.focus_index = None;
        self.anchor = None;
        self.pending_index = None;
        self.follow(cx);
        cx.notify();
    }

    /// A view delta (20 Hz): structural resets the rows, changed ids
    /// invalidate their pages, appended rows extend the view.
    pub fn apply_delta(&mut self, delta: &ViewDelta, cx: &mut Context<Self>) {
        let len = delta.len as usize;
        if delta.structural || len < self.cache.len() {
            self.reset(len, cx);
            return;
        }
        let mut flush = self.cache.invalidate_ids(&delta.changed);
        if len != self.cache.len() {
            flush |= self.cache.append(len);
        }
        if flush {
            self.schedule_flush(cx);
        }
        self.follow(cx);
        cx.notify();
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Row `index`, if it is loaded (fresh or stale).
    pub fn row(&self, index: usize) -> Option<&GridRow> {
        self.cache.get(index)
    }

    /// The rows the list rendered last frame.
    pub fn rendered(&self) -> Range<usize> {
        self.rendered.clone()
    }

    /// Fetches landed so far.
    pub fn landed(&self) -> u64 {
        self.landed
    }

    /// True when every rendered row is loaded.
    pub fn rendered_loaded(&self) -> bool {
        self.rendered.clone().all(|i| self.cache.get(i).is_some())
    }

    pub fn scroll_handle(&self) -> &UniformListScrollHandle {
        &self.scroll
    }

    pub fn scroll_to(&mut self, index: usize, cx: &mut Context<Self>) {
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub fn set_icon_source(&mut self, source: IconSource) {
        self.icons.set_source(source);
    }

    pub fn set_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        if self.overlay != overlay {
            self.overlay = overlay;
            cx.notify();
        }
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    /// Auto Select: follow and select the newest flow.
    pub fn set_auto_select(&mut self, on: bool, cx: &mut Context<Self>) {
        self.auto_select = on;
        self.follow(cx);
        cx.notify();
    }

    pub fn auto_select(&self) -> bool {
        self.auto_select
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let len = self.cache.len();
        if !self.auto_select || len == 0 {
            return;
        }
        self.scroll.scroll_to_item(len - 1, ScrollStrategy::Bottom);
        if let Some(newest) = self.cache.get(len - 1).map(GridRow::id)
            && self.selected.focused != Some(newest)
        {
            self.focus_index = Some(len - 1);
            self.anchor = Some(len - 1);
            self.selected.select(Some(newest));
            cx.emit(FlowGridEvent::SelectionChanged);
        }
    }

    /// Runs the frame's fetches once this frame is done, so every ask of
    /// the frame (scrolls, deltas) goes out as one batch.
    fn schedule_flush(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.flush(cx));
        })
        .detach();
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        let fetches = self.cache.take_fetches();
        let Some(source) = self.source.clone() else {
            return;
        };
        for fetch in fetches {
            self.load(fetch, source.clone(), cx);
        }
    }

    fn load(&mut self, fetch: Fetch, source: RowSource, cx: &mut Context<Self>) {
        let rows = cx.background_spawn(async move {
            source(fetch.start(), fetch.count()).map(|rows| rows.into_iter().map(GridRow::new).collect::<Vec<_>>())
        });
        cx.spawn(async move |this, cx| {
            let rows = rows.await;
            let _ = this.update(cx, |this, cx| this.landed_rows(fetch, rows, cx));
        })
        .detach();
    }

    fn landed_rows(&mut self, fetch: Fetch, rows: Result<Vec<GridRow>, String>, cx: &mut Context<Self>) {
        match rows {
            Ok(rows) => match self.cache.land(fetch, rows) {
                None => return,
                Some(again) => {
                    if again {
                        self.schedule_flush(cx);
                    }
                }
            },
            Err(err) => {
                eprintln!("flow table: get_rows failed: {err}");
                self.cache.fail(fetch);
                return;
            }
        }
        self.landed += 1;
        // A keyboard move landed on a row that has loaded since.
        if let Some((pending, extend)) = self.pending_index
            && let Some(id) = self.cache.get(pending).map(GridRow::id)
        {
            self.pending_index = None;
            if extend {
                self.select_range(pending, id, cx);
            } else {
                self.anchor = Some(pending);
                self.selected.select(Some(id));
                cx.emit(FlowGridEvent::SelectionChanged);
            }
        }
        self.follow(cx);
        cx.emit(FlowGridEvent::RowsLanded);
        cx.notify();
    }

    // ---- Selection ----

    pub fn selected(&self) -> &Selected {
        &self.selected
    }

    /// The selection as the engine takes it.
    pub fn flow_selection(&self) -> FlowSelection {
        self.selected.selection.to_flow_selection(self.view)
    }

    pub fn selection_count(&self) -> usize {
        self.selected.selection.count(self.cache.len())
    }

    /// Selects one flow (or none), as a click does.
    pub fn select(&mut self, id: Option<FlowId>, cx: &mut Context<Self>) {
        self.selected.select(id);
        self.focus_index = id.and_then(|id| self.cache.index_of(id));
        self.anchor = self.focus_index;
        self.pending_index = None;
        cx.emit(FlowGridEvent::SelectionChanged);
        cx.notify();
    }

    /// Selects row `index`'s flow and scrolls to it (the dev remote's
    /// `select`): when the row isn't loaded yet, once it is.
    pub fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        self.move_to(index, false, cx);
    }

    /// Selects exactly these flows; the last one is focused.
    pub fn select_ids(&mut self, ids: &[FlowId], cx: &mut Context<Self>) {
        let focus = ids.last().copied();
        self.selected
            .set(TableSelection::range(ids.iter().copied()), Some(focus));
        self.focus_index = focus.and_then(|id| self.cache.index_of(id));
        cx.emit(FlowGridEvent::SelectionChanged);
        cx.notify();
    }

    /// ⌘A: the whole view, as a view selection (never 100k ids).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected.set(TableSelection::whole_view(), None);
        cx.emit(FlowGridEvent::SelectionChanged);
        cx.notify();
    }

    fn click_row(&mut self, index: usize, id: FlowId, shift: bool, toggle: bool, cx: &mut Context<Self>) {
        self.pending_index = None;
        self.focus_index = Some(index);
        if shift {
            self.select_range(index, id, cx);
            return;
        }
        self.anchor = Some(index);
        if toggle {
            self.selected.toggle(id);
        } else {
            self.selected.select(Some(id));
        }
        cx.emit(FlowGridEvent::SelectionChanged);
        cx.notify();
    }

    /// A right-click: inside a multi-selection it keeps the selection (the
    /// bulk menu's case); elsewhere it selects the row first. The menus
    /// themselves arrive with G3 (row) and G11 (bulk).
    fn press_row(&mut self, index: usize, id: FlowId, cx: &mut Context<Self>) {
        let len = self.cache.len();
        if self.selected.selection.is_multi(len) && self.selected.selection.is_selected(id) {
            return;
        }
        self.click_row(index, id, false, false, cx);
    }

    /// Rows `anchor..=to`, focusing `focus`: from the loaded rows, else
    /// fetched (in steps of at most [`MAX_ROWS_PER_CALL`]).
    fn select_range(&mut self, to: usize, focus: FlowId, cx: &mut Context<Self>) {
        let from = self.anchor.unwrap_or(to);
        let (lo, hi) = (from.min(to), from.max(to));
        let len = self.cache.len();
        if lo == 0 && hi + 1 >= len && len > 1 {
            self.selected.set(TableSelection::whole_view(), Some(Some(focus)));
            cx.emit(FlowGridEvent::SelectionChanged);
            cx.notify();
            return;
        }
        if let Some(ids) = self.cache.ids_in(lo..hi + 1) {
            self.range_task = None;
            self.selected.set(TableSelection::range(ids), Some(Some(focus)));
            cx.emit(FlowGridEvent::SelectionChanged);
            cx.notify();
            return;
        }
        let Some(source) = self.source.clone() else { return };
        let ids = cx.background_spawn(async move { ids_between(&source, lo, hi) });
        self.range_task = Some(cx.spawn(async move |this, cx| {
            let ids = ids.await;
            let _ = this.update(cx, |this, cx| {
                this.range_task = None;
                match ids {
                    Ok(ids) => {
                        this.selected.set(TableSelection::range(ids), Some(Some(focus)));
                        cx.emit(FlowGridEvent::SelectionChanged);
                        cx.notify();
                    }
                    Err(err) => eprintln!("flow table: can't select the range: {err}"),
                }
            });
        }));
    }

    /// True while a ⇧-range is fetching ids.
    pub fn range_pending(&self) -> bool {
        self.range_task.is_some()
    }

    fn move_to(&mut self, index: usize, extend: bool, cx: &mut Context<Self>) {
        let len = self.cache.len();
        if len == 0 {
            return;
        }
        let to = index.min(len - 1);
        self.focus_index = Some(to);
        let row = self.cache.get(to).map(GridRow::id);
        // The Tauri table drops a ⇧-move to an unloaded row; here it waits too.
        self.pending_index = if row.is_some() { None } else { Some((to, extend)) };
        if extend && self.anchor.is_none() {
            self.anchor = Some(to);
        }
        match row {
            Some(id) if extend => self.select_range(to, id, cx),
            Some(id) => {
                self.anchor = Some(to);
                self.selected.select(Some(id));
                cx.emit(FlowGridEvent::SelectionChanged);
            }
            None => {}
        }
        self.scroll.scroll_to_item(to, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Where the focused flow is now: the remembered index if it still
    /// holds that flow, else wherever it is among the loaded rows.
    fn focused_index(&self) -> Option<usize> {
        let focused = self.selected.focused?;
        match self.focus_index {
            Some(i) if self.cache.get(i).map(GridRow::id) == Some(focused) => Some(i),
            _ => self.cache.index_of(focused),
        }
    }

    /// Rows per page (PgUp/PgDn): the viewport, less one.
    fn page_rows(&self) -> usize {
        let height = self.scroll.0.borrow().base_handle.bounds().size.height;
        ((f32::from(height) / ROW_HEIGHT) as usize).saturating_sub(1).max(1)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys aimed at the header's controls (a focused resize handle) are theirs.
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &event.keystroke;
        let key = TableKey {
            key: Some(Key::from_name(&ks.key)),
            shift: ks.modifiers.shift,
            cmd: ks.modifiers.platform,
            ctrl: ks.modifiers.control,
            alt: ks.modifiers.alt,
        };
        let from = self.focused_index();
        let rendered = self.rendered.clone();
        let state = TableState {
            from,
            first: rendered.start,
            last: rendered.end.saturating_sub(1),
            len: self.cache.len(),
            page: self.page_rows(),
            multi: self.selected.selection.is_multi(self.cache.len()),
            selected: self.selected.focused.is_some(),
        };
        let Some(action) = table_key(key, state) else { return };
        cx.stop_propagation();
        match action {
            TableAction::Move { to, extend } => {
                if extend && self.anchor.is_none() {
                    self.anchor = from;
                }
                self.move_to(to, extend, cx);
            }
            TableAction::Inspect => cx.emit(FlowGridEvent::Inspect),
            TableAction::Collapse => {
                let focused = self.selected.focused;
                self.selected.select(focused);
                cx.emit(FlowGridEvent::SelectionChanged);
                cx.notify();
            }
            TableAction::None => {}
        }
    }

    // ---- Columns ----

    pub fn prefs(&self) -> &TablePrefs {
        &self.prefs
    }

    pub fn columns(&self) -> &[PlacedColumn] {
        &self.placed
    }

    /// Replaces the prefs (header columns from the dev remote, a reload).
    /// A changed sort is the owner's to apply.
    pub fn set_prefs(&mut self, mut prefs: TablePrefs, cx: &mut Context<Self>) {
        prefs.normalize();
        self.prefs = prefs;
        self.placed = Rc::new(self.prefs.place());
        cx.notify();
    }

    fn change_prefs(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut TablePrefs)) {
        let sort = self.prefs.sort;
        change(&mut self.prefs);
        self.placed = Rc::new(self.prefs.place());
        cx.emit(FlowGridEvent::PrefsChanged);
        if self.prefs.sort != sort {
            cx.emit(FlowGridEvent::SortChanged(self.prefs.sort));
        }
        cx.notify();
    }

    /// A header title was clicked.
    pub fn sort_by(&mut self, id: &ColumnId, cx: &mut Context<Self>) {
        let Some(next) = self
            .placed
            .iter()
            .find(|c| &c.id == id)
            .and_then(|c| c.next_sort(&self.prefs.sort))
        else {
            return;
        };
        self.change_prefs(cx, |p| p.sort = next);
    }

    pub fn set_column_width(&mut self, id: &ColumnId, width: f32, cx: &mut Context<Self>) {
        self.change_prefs(cx, |p| p.set_width(id, width));
    }

    pub fn move_column(&mut self, id: &ColumnId, before: Option<&ColumnId>, cx: &mut Context<Self>) {
        self.change_prefs(cx, |p| p.move_column(id, before));
    }

    pub fn toggle_column(&mut self, id: &ColumnId, cx: &mut Context<Self>) {
        let hidden = self.prefs.is_hidden(id);
        self.change_prefs(cx, |p| p.set_hidden(id, !hidden));
    }

    pub fn reset_columns(&mut self, cx: &mut Context<Self>) {
        self.change_prefs(cx, TablePrefs::reset_columns);
    }

    pub fn remove_header_column(&mut self, id: &ColumnId, cx: &mut Context<Self>) {
        self.change_prefs(cx, |p| p.remove_header_column(id));
    }

    // ---- Rendering ----

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::Stateful<gpui_kit::Div>> {
        // uniform_list also asks for row 0 alone to measure a row: that is
        // not the window (taking it as one would evict the pages around the
        // real window every frame).
        let measuring = range == (0..1) && self.cache.len() > 1;
        if !measuring {
            if self.cache.ensure(range.clone()) {
                self.schedule_flush(cx);
            }
            self.rendered = range.clone();
        }
        let t = *cx.tokens();
        let sans = cx.theme().font_family.clone();
        let len = self.cache.len();
        let multi = self.selected.selection.is_multi(len);
        let columns = self.placed.clone();
        let mut rows = Vec::with_capacity(range.len());
        for index in range {
            let row = self.cache.get(index);
            let id = row.map(GridRow::id);
            let selected = id.is_some_and(|id| self.selected.selection.is_selected(id));
            let look = row::RowLook {
                selected,
                focused: multi && id.is_some() && id == self.selected.focused,
                current: id.is_some() && id == self.selected.focused,
                odd: index % 2 == 1,
            };
            let icon = row
                .and_then(|r| r.client.as_ref())
                .and_then(|client| self.icons.get(client, cx));
            let cells = row::CellContext {
                t: &t,
                look,
                icon,
                sans: sans.clone(),
            };
            let element = row::row_element(index, row, &columns, &cells, _window, cx);
            let element = match id {
                Some(id) => element
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(move |this, e: &gpui_kit::MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            let toggle = e.modifiers.platform || e.modifiers.control;
                            this.click_row(index, id, e.modifiers.shift, toggle, cx);
                        }),
                    )
                    .on_mouse_down(
                        gpui_kit::MouseButton::Right,
                        cx.listener(move |this, _: &gpui_kit::MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.press_row(index, id, cx);
                        }),
                    ),
                None => element,
            };
            rows.push(element);
        }
        rows
    }

    fn render_overlay(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = *cx.tokens();
        let text = |s: SharedString| div().text_color(t.muted_foreground.hsla()).child(s);
        let body: gpui_kit::AnyElement = match &self.overlay {
            Overlay::None => return None,
            Overlay::Waiting { command } => div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .child(text("Waiting for traffic".into()))
                .child(
                    div()
                        .id("empty-curl")
                        .role(Role::Group)
                        .aria_label(command.clone())
                        .aria_description("Right-click to copy the command")
                        .on_mouse_down(
                            gpui_kit::MouseButton::Right,
                            cx.listener(|_, e: &gpui_kit::MouseDownEvent, _, cx| {
                                cx.emit(FlowGridEvent::EmptyStateMenu(e.position));
                            }),
                        )
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .bg(t.muted.hsla())
                        .text_color(t.foreground.hsla())
                        .text_size(px(12.))
                        .child(command.clone()),
                )
                .into_any_element(),
            Overlay::NoMatch => text("No flows match the current filter".into()).into_any_element(),
            Overlay::Searching => text("Searching headers and bodies…".into()).into_any_element(),
            Overlay::Error(err) => div()
                .id("view-error")
                .role(Role::Alert)
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .child(text("Couldn't open the flow list".into()))
                .child(
                    div()
                        .max_w(px(420.))
                        .text_size(px(12.))
                        .text_color(t.muted_foreground.hsla())
                        .child(err.clone()),
                )
                .child(
                    gpui_kit::component::button::Button::new("view-retry")
                        .label("Try again")
                        .small()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGridEvent::Retry))),
                )
                .into_any_element(),
        };
        Some(
            div()
                .absolute()
                .top(px(HEADER_HEIGHT))
                .left_0()
                .right_0()
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .font_family(cx.theme().font_family.clone())
                .child(body),
        )
    }
}

use gpui_kit::component::Sizable as _;

/// The ids of rows `lo..=hi`, from the view, in steps the engine allows.
fn ids_between(source: &RowSource, lo: usize, hi: usize) -> Result<Vec<FlowId>, String> {
    let mut ids = Vec::with_capacity(hi - lo + 1);
    let mut start = lo;
    while start <= hi {
        let count = MAX_ROWS_PER_CALL.min(hi - start + 1);
        let rows = source(start, count)?;
        let got = rows.len();
        ids.extend(rows.into_iter().map(|r| r.0));
        if got < count {
            break;
        }
        start += count;
    }
    Ok(ids)
}

impl Render for FlowGrid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let len = self.cache.len();
        let width = min_table_width(&self.placed);
        let mono = cx.theme().mono_font_family.clone();
        let header = self.render_header(window, cx);
        let overlay = self.render_overlay(cx);
        let list = uniform_list("flow-rows", len, cx.processor(Self::render_rows))
            .track_scroll(&self.scroll)
            .flex_1()
            .w_full();
        div()
            .id("flow-table")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .role(Role::Grid)
            .aria_label("Flows")
            .aria_row_count(len + 1)
            .aria_column_count(self.placed.len())
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &DeleteSelection, _, cx| {
                if this.selection_count() > 0 {
                    cx.emit(FlowGridEvent::Delete(this.flow_selection()));
                }
            }))
            .on_action(cx.listener(|this, a: &ToggleColumn, _, cx| this.toggle_column(&a.id, cx)))
            .on_action(cx.listener(|this, a: &RemoveHeaderColumn, _, cx| this.remove_header_column(&a.id, cx)))
            .on_action(cx.listener(|this, _: &ResetColumns, _, cx| this.reset_columns(cx)))
            .on_action(|_: &AddHeaderColumn, _, _| {})
            .on_drop(cx.listener(|this, _: &header::ResizeDrag, _, cx| this.end_resize(cx)))
            .relative()
            .size_full()
            .bg(t.pane_table.hsla())
            .text_color(t.foreground.hsla())
            .font_family(mono)
            .child(
                div()
                    .id("flow-table-scroll-x")
                    .size_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.hscroll)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .h_full()
                            .w_full()
                            .min_w(px(width))
                            .child(header)
                            .child(list),
                    ),
            )
            .children(overlay)
    }
}
