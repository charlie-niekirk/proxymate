//! FlowGrid driven like a user would: clicks at row positions, keystrokes
//! through gpui's dispatch, over a fake view (no engine). The key and
//! selection tables of `keys.test.ts` and `selection.test.ts` are ported in
//! `pm-view`; these check the grid wires them up.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gpui_kit::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use pm_model::{
    BodyPart, ContentKind, FlowRow, FlowSelection, FlowState, HeaderColumn, HttpVersion, Scheme, SortKey, TlsStatus,
    ViewDelta,
};
use pm_view::columns::{Builtin, ColumnId, HEADER_HEIGHT, ROW_HEIGHT, TablePrefs};

use super::*;

fn flow_row(id: u32) -> FlowRow {
    FlowRow(
        id,
        FlowState::Completed,
        Scheme::Https,
        "GET".into(),
        format!("h{}.example.com", id % 7).into(),
        443,
        format!("/items/{id}"),
        String::new(),
        Some(200),
        ContentKind::Json,
        None,
        0.0,
        Some(12),
        0,
        100,
        false,
        HttpVersion::Http11,
        TlsStatus::Intercepted,
        None,
        None,
        false,
        None,
        None,
        None,
        vec![],
    )
}

/// A fake view of `ids` (shared, so a test can change it), counting fetches.
#[derive(Clone)]
struct Fake {
    ids: Arc<Mutex<Vec<u32>>>,
    calls: Arc<AtomicUsize>,
    max_count: Arc<AtomicUsize>,
}

impl Fake {
    fn new(n: u32) -> Fake {
        Fake {
            ids: Arc::new(Mutex::new((0..n).collect())),
            calls: Arc::default(),
            max_count: Arc::default(),
        }
    }

    fn source(&self) -> RowSource {
        let f = self.clone();
        Arc::new(move |start, count| {
            f.calls.fetch_add(1, Ordering::SeqCst);
            f.max_count.fetch_max(count, Ordering::SeqCst);
            let ids = f.ids.lock().unwrap();
            let end = (start + count).min(ids.len());
            Ok(ids
                .get(start..end)
                .unwrap_or_default()
                .iter()
                .map(|&id| flow_row(id))
                .collect())
        })
    }
}

struct Harness<'a> {
    grid: Entity<FlowGrid>,
    cx: &'a mut VisualTestContext,
    events: Arc<Mutex<Vec<FlowGridEvent>>>,
}

fn setup<'a>(cx: &'a mut TestAppContext, fake: &Fake) -> Harness<'a> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        pm_theme::kit::apply(pm_theme::Mode::Dark, cx);
        bind_keys(cx);
    });
    let len = fake.ids.lock().unwrap().len();
    let source = fake.source();
    let (grid, cx) = cx.add_window_view(|_, cx| {
        let mut grid = FlowGrid::new(TablePrefs::default(), cx);
        grid.set_source(1, source, len, cx);
        grid
    });
    // 26 rows of 22 px under the header.
    cx.simulate_resize(size(px(1200.), px(HEADER_HEIGHT + 26. * ROW_HEIGHT)));
    let events: Arc<Mutex<Vec<FlowGridEvent>>> = Arc::default();
    let sink = events.clone();
    cx.update(|window, cx| {
        cx.subscribe(&grid, move |_, e: &FlowGridEvent, _| {
            sink.lock().unwrap().push(e.clone())
        })
        .detach();
        let focus = grid.focus_handle(cx);
        window.focus(&focus, cx);
    });
    cx.run_until_parked();
    Harness { grid, cx, events }
}

impl Harness<'_> {
    fn click(&mut self, row: usize, modifiers: Modifiers) {
        let y = HEADER_HEIGHT + ROW_HEIGHT * row as f32 + ROW_HEIGHT / 2.;
        self.cx.simulate_click(point(px(300.), px(y)), modifiers);
        self.cx.run_until_parked();
    }

    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }

    fn focused(&mut self) -> Option<u32> {
        self.grid.read_with(self.cx, |g, _| g.selected().focused)
    }

    fn selection(&mut self) -> FlowSelection {
        self.grid.read_with(self.cx, |g, _| g.flow_selection())
    }

    fn ids(ids: &[u32]) -> FlowSelection {
        FlowSelection::Ids { ids: ids.to_vec() }
    }

    fn took(&mut self, event: &FlowGridEvent) -> bool {
        let mut events = self.events.lock().unwrap();
        let found = events.contains(event);
        events.clear();
        found
    }
}

#[gpui_kit::test]
fn loads_only_the_visible_window_in_capped_fetches(cx: &mut TestAppContext) {
    let fake = Fake::new(100_000);
    let h = setup(cx, &fake);
    let (rendered, loaded) = h.grid.read_with(h.cx, |g, _| (g.rendered(), g.rendered_loaded()));
    assert!(rendered.end > 20 && rendered.end < 40, "{rendered:?}");
    assert!(loaded);
    assert!(fake.calls.load(Ordering::SeqCst) <= 2);
    assert!(fake.max_count.load(Ordering::SeqCst) <= MAX_ROWS_PER_CALL);
    h.grid.update(h.cx, |g, cx| g.scroll_to(60_000, cx));
    h.cx.run_until_parked();
    let (rendered, loaded) = h.grid.read_with(h.cx, |g, _| (g.rendered(), g.rendered_loaded()));
    assert!(rendered.contains(&60_000) && loaded, "{rendered:?}");
}

#[gpui_kit::test]
fn click_cmd_click_and_shift_click_select_by_id(cx: &mut TestAppContext) {
    let fake = Fake::new(200);
    let mut h = setup(cx, &fake);
    h.click(3, Modifiers::none());
    assert_eq!((h.focused(), h.selection()), (Some(3), Harness::ids(&[3])));
    h.click(7, Modifiers::command());
    assert_eq!((h.focused(), h.selection()), (Some(7), Harness::ids(&[3, 7])));
    h.click(3, Modifiers::command());
    assert_eq!((h.focused(), h.selection()), (Some(7), Harness::ids(&[7])));
    // ⇧-click: the range from the anchor (the last plain or ⌘ click).
    h.click(10, Modifiers::shift());
    assert_eq!(
        (h.focused(), h.selection()),
        (Some(10), Harness::ids(&[3, 4, 5, 6, 7, 8, 9, 10]))
    );
    assert!(h.took(&FlowGridEvent::SelectionChanged));
}

#[gpui_kit::test]
fn arrows_pages_and_home_end_move_the_selection(cx: &mut TestAppContext) {
    let fake = Fake::new(1_000);
    let mut h = setup(cx, &fake);
    // Nothing focused: ↓ starts at the first rendered row.
    h.keys("down");
    assert_eq!(h.focused(), Some(0));
    h.keys("down down");
    assert_eq!(h.focused(), Some(2));
    h.keys("up");
    assert_eq!(h.focused(), Some(1));
    // A page is the viewport less one row (26 rows shown).
    h.keys("pagedown");
    assert_eq!(h.focused(), Some(26));
    h.keys("pageup");
    assert_eq!(h.focused(), Some(1));
    // End jumps to a row that isn't loaded yet: it is selected once it is.
    h.keys("end");
    assert_eq!(h.focused(), Some(999));
    assert!(h.grid.read_with(h.cx, |g, _| g.rendered().contains(&999)));
    h.keys("home");
    assert_eq!(h.focused(), Some(0));
    assert_eq!(h.selection(), Harness::ids(&[0]));
}

#[gpui_kit::test]
fn shift_arrows_extend_and_escape_collapses(cx: &mut TestAppContext) {
    let fake = Fake::new(100);
    let mut h = setup(cx, &fake);
    h.click(5, Modifiers::none());
    h.keys("shift-down shift-down");
    assert_eq!((h.focused(), h.selection()), (Some(7), Harness::ids(&[5, 6, 7])));
    h.keys("shift-up shift-up shift-up");
    assert_eq!(h.selection(), Harness::ids(&[4, 5]));
    h.keys("escape");
    assert_eq!((h.focused(), h.selection()), (Some(4), Harness::ids(&[4])));
    // Escape with one flow selected is left to the app.
    h.events.lock().unwrap().clear();
    h.keys("escape");
    assert!(!h.took(&FlowGridEvent::SelectionChanged));
}

#[gpui_kit::test]
fn a_shift_range_beyond_the_loaded_rows_is_fetched_in_steps(cx: &mut TestAppContext) {
    let fake = Fake::new(5_000);
    let mut h = setup(cx, &fake);
    h.click(2, Modifiers::none());
    // A keyboard ⇧-jump far below: rows 2..=3000 aren't all loaded.
    h.grid.update(h.cx, |g, cx| {
        g.anchor = Some(2);
        let id = 3000;
        g.select_range(3000, id, cx);
    });
    h.cx.run_until_parked();
    let FlowSelection::Ids { ids } = h.selection() else {
        panic!("a listed range")
    };
    assert_eq!((ids.len(), ids[0], *ids.last().unwrap()), (2999, 2, 3000));
    assert!(fake.max_count.load(Ordering::SeqCst) <= MAX_ROWS_PER_CALL);
    // From the first row to the last is the whole view, not 5,000 ids.
    h.keys("home");
    h.keys("shift-end");
    assert_eq!(
        h.selection(),
        FlowSelection::View {
            view: 1,
            except: vec![]
        }
    );
}

#[gpui_kit::test]
fn cmd_a_enter_and_backspace(cx: &mut TestAppContext) {
    let fake = Fake::new(50);
    let mut h = setup(cx, &fake);
    h.keys("enter");
    assert!(!h.took(&FlowGridEvent::Inspect), "nothing focused");
    h.click(1, Modifiers::none());
    h.keys("enter");
    assert!(h.took(&FlowGridEvent::Inspect));
    h.keys("cmd-a");
    assert_eq!(
        h.selection(),
        FlowSelection::View {
            view: 1,
            except: vec![]
        }
    );
    assert_eq!(h.focused(), Some(1), "⌘A keeps the focused flow");
    h.click(4, Modifiers::command());
    assert_eq!(
        h.selection(),
        FlowSelection::View {
            view: 1,
            except: vec![4]
        }
    );
    assert_eq!(h.grid.read_with(h.cx, |g, _| g.selection_count()), 49);
    h.keys("backspace");
    assert!(h.took(&FlowGridEvent::Delete(FlowSelection::View {
        view: 1,
        except: vec![4]
    })));
}

#[gpui_kit::test]
fn deltas_reset_invalidate_and_append(cx: &mut TestAppContext) {
    let fake = Fake::new(10);
    let mut h = setup(cx, &fake);
    h.click(2, Modifiers::none());
    fake.ids.lock().unwrap().extend([10, 11, 12]);
    let delta = ViewDelta {
        view: 1,
        len: 13,
        appended: 3,
        ..Default::default()
    };
    h.grid.update(h.cx, |g, cx| g.apply_delta(&delta, cx));
    h.cx.run_until_parked();
    assert_eq!(h.grid.read_with(h.cx, |g, _| g.row(12).map(GridRow::id)), Some(12));
    // A structural delta (a reorder) refetches; the selection stays by id.
    fake.ids.lock().unwrap().reverse();
    let delta = ViewDelta {
        view: 1,
        len: 13,
        structural: true,
        ..Default::default()
    };
    h.grid.update(h.cx, |g, cx| g.apply_delta(&delta, cx));
    h.cx.run_until_parked();
    assert_eq!(h.grid.read_with(h.cx, |g, _| g.row(0).map(GridRow::id)), Some(12));
    assert_eq!(h.focused(), Some(2));
    h.keys("down");
    assert_eq!(h.focused(), Some(1), "↓ moves from where flow 2 is now (row 10)");
}

#[gpui_kit::test]
fn auto_select_follows_the_newest_flow(cx: &mut TestAppContext) {
    let fake = Fake::new(500);
    let mut h = setup(cx, &fake);
    h.grid.update(h.cx, |g, cx| g.set_auto_select(true, cx));
    h.cx.run_until_parked();
    assert_eq!(h.focused(), Some(499));
    fake.ids.lock().unwrap().push(500);
    let delta = ViewDelta {
        view: 1,
        len: 501,
        appended: 1,
        ..Default::default()
    };
    h.grid.update(h.cx, |g, cx| g.apply_delta(&delta, cx));
    h.cx.run_until_parked();
    assert_eq!(h.focused(), Some(500));
}

#[gpui_kit::test]
fn sorting_resizing_and_hiding_change_the_prefs(cx: &mut TestAppContext) {
    let fake = Fake::new(20);
    let mut h = setup(cx, &fake);
    let url = ColumnId::Builtin(Builtin::Url);
    h.grid.update(h.cx, |g, cx| g.sort_by(&url, cx));
    let sort = h.grid.read_with(h.cx, |g, _| g.prefs().sort);
    assert_eq!(sort.key, SortKey::Url);
    assert!(h.events.lock().unwrap().contains(&FlowGridEvent::SortChanged(sort)));
    // The column menu's items are actions on the focused grid.
    let client = ColumnId::Builtin(Builtin::Client);
    h.cx.dispatch_action(ToggleColumn { id: client.clone() });
    assert!(
        h.grid
            .read_with(h.cx, |g, _| g.columns().iter().all(|c| c.id != client))
    );
    // The time column's edge, keyed: → widens by 8 px, ↩ restores the default.
    let time = ColumnId::Builtin(Builtin::Time);
    let focus = h.grid.update(h.cx, |g, _| g.header.handles[&time].clone());
    h.cx.update(|window, cx| window.focus(&focus, cx));
    h.keys("right shift-right");
    let px = |h: &mut Harness| {
        h.grid
            .read_with(h.cx, |g, _| g.columns().iter().find(|c| c.id == time).unwrap().px)
    };
    assert_eq!(px(&mut h), 92. + 8. + 32.);
    h.keys("enter");
    assert_eq!(px(&mut h), 92.);
    // Keys on the handle are its own: the selection didn't move.
    assert_eq!(h.focused(), None);
    h.cx.dispatch_action(ResetColumns);
    assert!(
        h.grid
            .read_with(h.cx, |g, _| g.columns().iter().any(|c| c.id == client))
    );
    assert!(h.took(&FlowGridEvent::PrefsChanged));
}

#[gpui_kit::test]
fn header_columns_show_their_values(cx: &mut TestAppContext) {
    let fake = Fake::new(5);
    let h = setup(cx, &fake);
    let mut prefs = TablePrefs::default();
    assert!(prefs.add_header_column(HeaderColumn {
        part: BodyPart::Request,
        name: "X-Trace".into(),
    }));
    h.grid.update(h.cx, |g, cx| g.set_prefs(prefs, cx));
    let cols = h.grid.read_with(h.cx, |g, _| {
        g.columns().iter().map(|c| c.id.to_string()).collect::<Vec<_>>()
    });
    assert_eq!(&cols[cols.len() - 2..], ["hdr:request:x-trace", "tools"]);
    h.cx.dispatch_action(RemoveHeaderColumn {
        id: ColumnId::header(BodyPart::Request, "x-trace"),
    });
    assert!(h.grid.read_with(h.cx, |g, _| g.prefs().header_columns.is_empty()));
}

/// The right edge of column `id`, from the placed widths (the URL column
/// takes what the others leave of the 1200 px window).
fn right_edge(h: &mut Harness, id: &ColumnId) -> f32 {
    h.grid.read_with(h.cx, |g, _| {
        let fixed: f32 = g.columns().iter().filter(|c| !c.grow).map(|c| c.px).sum();
        let mut x = 0.;
        for c in g.columns() {
            x += if c.grow { (1200. - fixed).max(c.px) } else { c.px };
            if &c.id == id {
                return x;
            }
        }
        panic!("no column {id}")
    })
}

#[gpui_kit::test]
fn dragging_a_column_edge_resizes_it_and_a_double_click_resets_it(cx: &mut TestAppContext) {
    let fake = Fake::new(20);
    let mut h = setup(cx, &fake);
    let time = ColumnId::Builtin(Builtin::Time);
    let x = right_edge(&mut h, &time) - 1.;
    let y = HEADER_HEIGHT / 2.;
    let left = gpui_kit::MouseButton::Left;
    h.cx.simulate_mouse_down(point(px(x), px(y)), left, Modifiers::none());
    for step in 1..=6 {
        h.cx.simulate_mouse_move(point(px(x + 5. * step as f32), px(y)), left, Modifiers::none());
    }
    h.cx.simulate_mouse_up(point(px(x + 30.), px(y)), left, Modifiers::none());
    h.cx.run_until_parked();
    let px_of = |h: &mut Harness| {
        h.grid
            .read_with(h.cx, |g, _| g.columns().iter().find(|c| c.id == time).unwrap().px)
    };
    assert_eq!(px_of(&mut h), 122., "92 px dragged 30 px wider");
    assert!(h.took(&FlowGridEvent::PrefsChanged), "saved once the drag ends");
    let x = right_edge(&mut h, &time) - 1.;
    h.cx.simulate_event(gpui_kit::MouseDownEvent {
        position: point(px(x), px(y)),
        modifiers: Modifiers::none(),
        button: left,
        click_count: 2,
        first_mouse: false,
    });
    h.cx.simulate_event(gpui_kit::MouseUpEvent {
        position: point(px(x), px(y)),
        modifiers: Modifiers::none(),
        button: left,
        click_count: 2,
    });
    h.cx.run_until_parked();
    assert_eq!(px_of(&mut h), 92., "double-click restores the default");
}

#[gpui_kit::test]
fn dragging_a_header_onto_another_reorders(cx: &mut TestAppContext) {
    let fake = Fake::new(20);
    let mut h = setup(cx, &fake);
    let method = ColumnId::Builtin(Builtin::Method);
    let id = ColumnId::Builtin(Builtin::Id);
    // From the middle of the Method title to over the # header.
    let from = right_edge(&mut h, &method) - 36.;
    let y = HEADER_HEIGHT / 2.;
    let left = gpui_kit::MouseButton::Left;
    h.cx.simulate_mouse_down(point(px(from), px(y)), left, Modifiers::none());
    for x in [from - 20., from - 200., 200., 30.] {
        h.cx.simulate_mouse_move(point(px(x), px(y)), left, Modifiers::none());
    }
    h.cx.simulate_mouse_up(point(px(30.), px(y)), left, Modifiers::none());
    h.cx.run_until_parked();
    let order = h.grid.read_with(h.cx, |g, _| {
        g.columns().iter().map(|c| c.id.clone()).collect::<Vec<_>>()
    });
    assert_eq!(&order[..2], [method, id], "Method now goes before #");
    assert!(h.took(&FlowGridEvent::PrefsChanged));
}

#[gpui_kit::test]
fn right_clicks_and_the_tools_button_ask_for_the_row_menu(cx: &mut TestAppContext) {
    let fake = Fake::new(40);
    let mut h = setup(cx, &fake);
    h.click(1, Modifiers::none());
    let row_y = |row: usize| HEADER_HEIGHT + ROW_HEIGHT * row as f32 + ROW_HEIGHT / 2.;
    // A right-click in the Status column of row 4: selects it, names the column.
    let status = ColumnId::Builtin(Builtin::Status);
    let x = right_edge(&mut h, &status) - 10.;
    h.cx.simulate_event(gpui_kit::MouseDownEvent {
        button: gpui_kit::MouseButton::Right,
        position: point(px(x), px(row_y(4))),
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    h.cx.run_until_parked();
    assert_eq!(h.focused(), Some(4));
    let menu = h.events.lock().unwrap().iter().find_map(|e| match e {
        FlowGridEvent::RowMenu { id, column, .. } => Some((*id, column.clone())),
        _ => None,
    });
    assert_eq!(menu, Some((4, Some(status))));
    h.events.lock().unwrap().clear();
    // The "…" in the Tools column (the last): the row's menu with no column.
    let tools = right_edge(&mut h, &ColumnId::Builtin(Builtin::Tools)) - 10.;
    h.cx.simulate_click(point(px(tools), px(row_y(6))), Modifiers::none());
    h.cx.run_until_parked();
    assert_eq!(h.focused(), Some(6));
    let menu = h.events.lock().unwrap().iter().find_map(|e| match e {
        FlowGridEvent::RowMenu { id, column, .. } => Some((*id, column.clone())),
        _ => None,
    });
    assert_eq!(menu, Some((6, None)));
}
