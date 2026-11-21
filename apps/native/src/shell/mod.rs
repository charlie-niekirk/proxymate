//! The main window: a transparent window with the traffic lights where the
//! Tauri app puts them ([`TRAFFIC_LIGHTS`]), sidebar vibrancy behind it, and the three-pane layout of
//! `AppShell.tsx` (sidebar | toolbar, content, footer). Every pane paints an
//! opaque token except the sidebar, which is transparent on macOS so the
//! `NSVisualEffectView` shows through.
//!
//! G1 put the flow table in the content area, over the inspector, with the
//! sidebar's width and the table's share of the column draggable. G2 adds
//! the chrome around it: the [`toolbar`], [`sidebar`], document [`tabs`],
//! [`filter_bar`] and [`footer`], over the shared [`Workspace`]. G3 fills
//! the inspector ([`Inspector`]); ↩ in the table moves the keyboard into
//! it and Escape back.

mod chrome;
pub mod filter_bar;
pub mod footer;
pub mod sidebar;
pub mod tabs;
pub mod toolbar;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, ClickEvent, Context, DragMoveEvent, Entity, Focusable as _, Global, IntoElement, Orientation,
    Render, Role, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, div, point, px, size,
};
use pm_core::Engine;
use pm_model::{SidebarSnapshot, StorageStatus};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;

use crate::bridge::{self, Bridge, stream::Latest};
use crate::features::flow_table::{FlowTable, FlowTableEvent};
use crate::features::inspector::Inspector;
use crate::workspace::{Workspace, WorkspaceEvent};
use filter_bar::FilterBar;
use footer::Footer;
use sidebar::Sidebar;

/// `tauri.conf.json`'s window: 1440×900, at least 960×600.
pub const DEFAULT_SIZE: (f32, f32) = (1440., 900.);
pub const MIN_SIZE: (f32, f32) = (960., 600.);
/// Where the traffic lights sit: the same place as the Tauri app's
/// `trafficLightPosition` of (16, 22). tao measures that `y` down to the
/// buttons' centre line (about 22–24 px); gpui places the buttons' top edge
/// at `y`, so the same spot is (16, 17), measured from both apps' captures.
pub const TRAFFIC_LIGHTS: (f32, f32) = (16., 17.);
/// The toolbar strip, which the traffic lights share with the sidebar.
pub const TOOLBAR_HEIGHT: f32 = 52.;
pub const SIDEBAR_WIDTH: f32 = 220.;
/// How narrow and wide the sidebar can be dragged.
pub const SIDEBAR_RANGE: (f32, f32) = (160., 360.);
pub const FOOTER_HEIGHT: f32 = 28.;
/// The table's share of the main column, over the inspector.
pub const TABLE_SHARE: f32 = 0.58;
/// The table and the inspector are each at least this tall.
const PANE_MIN_HEIGHT: f32 = 120.;
/// The main column is at least this wide.
const MAIN_MIN_WIDTH: f32 = 600.;
/// A split handle's thickness.
const HANDLE: f32 = 5.;

pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Proxymate".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
        }),
        window_bounds: Some(WindowBounds::centered(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), cx)),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        // macOS: vibrancy shows through the transparent sidebar. Elsewhere
        // the sidebar paints its solid token.
        window_background: if cfg!(target_os = "macos") {
            WindowBackgroundAppearance::Transparent
        } else {
            WindowBackgroundAppearance::Opaque
        },
        // The toolbar strips move the window themselves (`chrome::drag`).
        app_owns_titlebar_drag: true,
        app_id: Some(pm_app::BUNDLE_ID.into()),
        ..Default::default()
    }
}

pub fn open_main_window(cx: &mut App) {
    let options = window_options(cx);
    let engine = Bridge::engine(cx);
    let storage = bridge::storage(cx);
    let sidebar = bridge::sidebar(cx);
    let workspace = match Workspace::global(cx) {
        Some(ws) => ws,
        None => {
            // An O(1) read of the listener state, fine on the foreground thread.
            let proxy = engine.proxy_status();
            let engine = engine.clone();
            let ws = cx.new(|cx| Workspace::new(Some(engine), proxy, cx));
            Workspace::set_global(ws.clone(), cx);
            ws
        }
    };
    let addr: SharedString = workspace
        .read(cx)
        .proxy
        .listening
        .clone()
        .unwrap_or_else(|| "127.0.0.1:9090".into())
        .into();
    let opened = cx.open_window(options, |window, cx| {
        if let Err(err) = pm_platform::window::apply_sidebar_vibrancy(window) {
            eprintln!("vibrancy unavailable: {err}");
        }
        let table = cx.new(|cx| FlowTable::live(addr, cx));
        let parts = Parts {
            workspace,
            storage,
            sidebar,
            engine: Some(engine),
        };
        let view = cx.new(|cx| MainWindow::new(parts, table, window, cx));
        cx.new(|cx| root(view, window, cx))
    });
    if let Err(err) = opened {
        eprintln!("can't open the main window: {err}");
    }
}

/// gpui-kit's root around `view`. The root paints the theme background
/// under everything; on macOS it must stay clear so the sidebar shows the
/// vibrancy (styles on the Root win over its plugins' defaults).
pub fn root(
    view: Entity<MainWindow>,
    window: &mut Window,
    cx: &mut Context<gpui_kit::base::Root>,
) -> gpui_kit::base::Root {
    let root = gpui_kit::base::Root::new(view, window, cx);
    if cfg!(target_os = "macos") {
        root.bg(gpui_kit::transparent_black())
    } else {
        root
    }
}

/// A split handle being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    Sidebar,
    Inspector,
}

struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// What a main window shows, besides its flow table: the workspace and the
/// engine's streams (the bridge's in the app, injected snapshots in tests).
pub struct Parts {
    pub workspace: Entity<Workspace>,
    pub storage: Entity<Latest<StorageStatus>>,
    pub sidebar: Entity<Latest<SidebarSnapshot>>,
    pub engine: Option<Arc<Engine>>,
}

/// The main window's views, for commands that focus them and the dev remote.
pub struct ShellParts {
    pub window: AnyWindowHandle,
    pub main: WeakEntity<MainWindow>,
    pub sidebar: WeakEntity<Sidebar>,
    pub filter_bar: WeakEntity<FilterBar>,
    pub table: WeakEntity<FlowTable>,
    pub inspector: WeakEntity<Inspector>,
}

impl Global for ShellParts {}

pub fn parts(cx: &App) -> Option<&ShellParts> {
    cx.try_global::<ShellParts>()
}

/// ⌘F: the search box, its text selected.
pub fn focus_search(window: &mut Window, cx: &mut App) {
    if let Some(bar) = parts(cx).and_then(|p| p.filter_bar.upgrade()) {
        bar.update(cx, |b, cx| b.focus(window, cx));
    }
}

/// The search box with the caret at the end.
pub fn focus_search_at_end(window: &mut Window, cx: &mut App) {
    if let Some(bar) = parts(cx).and_then(|p| p.filter_bar.upgrade()) {
        bar.update(cx, |b, cx| b.focus_at_end(window, cx));
    }
}

/// ⌘L: the sidebar's filter box.
pub fn focus_sidebar_filter(window: &mut Window, cx: &mut App) {
    if let Some(sidebar) = parts(cx).and_then(|p| p.sidebar.upgrade()) {
        sidebar.update(cx, |s, cx| s.focus_filter(window, cx));
    }
}

/// The flow table (Escape or ↩ in the search box).
pub fn focus_table(window: &mut Window, cx: &mut App) {
    if let Some(table) = parts(cx).and_then(|p| p.table.upgrade()) {
        let focus = table.read(cx).grid().focus_handle(cx);
        window.focus(&focus, cx);
    }
}

/// The inspector, while it shows a flow (↩ in the table, "Focus the
/// inspector").
pub fn focus_inspector(window: &mut Window, cx: &mut App) {
    let Some(inspector) = parts(cx).and_then(|p| p.inspector.upgrade()) else {
        return;
    };
    if inspector.read(cx).key(cx).is_some() {
        let focus = inspector.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

/// Numbers with thousands separators, as the Tauri UI's `toLocaleString`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub struct MainWindow {
    workspace: Entity<Workspace>,
    table: Entity<FlowTable>,
    sidebar: Entity<Sidebar>,
    filter_bar: Entity<FilterBar>,
    footer: Entity<Footer>,
    inspector: Entity<Inspector>,
    sidebar_width: f32,
    table_share: f32,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    /// The window over a workspace, the engine's streams and a flow table.
    pub fn new(parts: Parts, table: Entity<FlowTable>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Parts {
            workspace,
            storage,
            sidebar: snapshot,
            engine,
        } = parts;
        let sidebar = cx.new(|cx| {
            Sidebar::new(
                workspace.clone(),
                snapshot.clone(),
                storage.clone(),
                engine.clone(),
                window,
                cx,
            )
        });
        let filter_bar = cx.new(|cx| FilterBar::new(workspace.clone(), table.clone(), snapshot.clone(), window, cx));
        let footer = cx.new(|cx| Footer::new(workspace.clone(), table.clone(), storage.clone(), cx));
        let inspector = cx.new(|cx| Inspector::new(workspace.clone(), table.clone(), cx));
        let subscriptions = vec![
            cx.subscribe_in(&table, window, |this, _, e: &FlowTableEvent, window, cx| match e {
                FlowTableEvent::Inspect => {
                    if this.inspector.read(cx).key(cx).is_some() {
                        let focus = this.inspector.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }
                }
            }),
            cx.observe(&table, |_, _, cx| cx.notify()),
            cx.observe(&workspace, |this, ws, cx| {
                let addr = ws.read(cx).proxy.listening.clone();
                if let Some(addr) = addr {
                    this.table.update(cx, |t, cx| t.set_proxy_addr(addr.into(), cx));
                }
                cx.notify();
            }),
            cx.subscribe(&workspace, |this, ws, e: &WorkspaceEvent, cx| {
                let (filter, store) = {
                    let ws = ws.read(cx);
                    (ws.filter.clone(), ws.store)
                };
                match e {
                    WorkspaceEvent::FilterChanged { text_only } => {
                        let debounce = *text_only;
                        this.table.update(cx, |t, cx| t.apply_filter(filter, debounce, cx));
                    }
                    WorkspaceEvent::StoreChanged => this.table.update(cx, |t, cx| t.set_store(store, cx)),
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| crate::appearance::window_changed(window, cx)),
        ];
        // A filter from before the window (PROXYMATE_DEV_FILTER).
        let filter = workspace.read(cx).filter.clone();
        if &filter != table.read(cx).filter() {
            table.update(cx, |t, cx| t.apply_filter(filter, false, cx));
        }
        cx.set_global(ShellParts {
            window: window.window_handle(),
            main: cx.weak_entity(),
            sidebar: sidebar.downgrade(),
            filter_bar: filter_bar.downgrade(),
            table: table.downgrade(),
            inspector: inspector.downgrade(),
        });
        // The table has the keyboard from the start.
        let focus = table.read(cx).grid().focus_handle(cx);
        window.focus(&focus, cx);
        MainWindow {
            workspace,
            table,
            sidebar,
            filter_bar,
            footer,
            inspector,
            sidebar_width: SIDEBAR_WIDTH,
            table_share: TABLE_SHARE,
            _subscriptions: subscriptions,
        }
    }

    pub fn table(&self) -> &Entity<FlowTable> {
        &self.table
    }

    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    pub fn filter_bar(&self) -> &Entity<FilterBar> {
        &self.filter_bar
    }

    pub fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    pub fn inspector(&self) -> &Entity<Inspector> {
        &self.inspector
    }

    /// A split's handle: drag to resize, double-click for the default.
    fn handle(&self, split: Split, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, label, orientation) = match split {
            Split::Sidebar => ("sidebar-resize", "Resize the sidebar", Orientation::Vertical),
            Split::Inspector => ("inspector-resize", "Resize the flow list", Orientation::Horizontal),
        };
        let value = match split {
            Split::Sidebar => self.sidebar_width,
            Split::Inspector => self.table_share * 100.,
        };
        div()
            .id(id)
            .role(Role::Splitter)
            .aria_label(label)
            .aria_orientation(orientation)
            .aria_numeric_value(value as f64)
            .flex_none()
            .map(|d| match split {
                Split::Sidebar => d.w(px(HANDLE)).h_full().cursor_col_resize(),
                Split::Inspector => d.h(px(HANDLE)).w_full().cursor_row_resize(),
            })
            .hover(|s| s.bg(t.primary.hsla().opacity(0.3)))
            .on_drag(split, |_, _, _, cx| cx.new(|_| NoGhost))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() >= 2 {
                    match split {
                        Split::Sidebar => this.sidebar_width = SIDEBAR_WIDTH,
                        Split::Inspector => this.table_share = TABLE_SHARE,
                    }
                    cx.notify();
                }
            }))
    }

    fn sidebar_pane(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(px(self.sidebar_width))
            .h_full()
            .bg(t.sidebar.hsla())
            .border_r_1()
            .border_color(t.sidebar_border.hsla())
            .child(self.sidebar.clone())
            // Over the sidebar's right edge, so the window's vibrancy shows
            // nowhere it shouldn't.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.))
                    .child(self.handle(Split::Sidebar, t, cx)),
            )
    }

    /// The flow table over the inspector.
    fn content(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let share = self.table_share;
        div()
            .id("content")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.pane_table.hsla())
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<Split>, _, cx| {
                if *e.drag(cx) != Split::Inspector {
                    return;
                }
                let top = f32::from(e.bounds.origin.y);
                let height = f32::from(e.bounds.size.height).max(1.);
                let max = (height - PANE_MIN_HEIGHT - HANDLE).max(PANE_MIN_HEIGHT);
                let table = (f32::from(e.event.position.y) - top).clamp(PANE_MIN_HEIGHT, max);
                this.table_share = table / height;
                cx.notify();
            }))
            .child(
                div()
                    .id("flow-table-pane")
                    .w_full()
                    .flex_none()
                    .h(gpui_kit::relative(share))
                    .min_h(px(PANE_MIN_HEIGHT))
                    .child(self.table.clone()),
            )
            .child(
                div()
                    .w_full()
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .child(self.handle(Split::Inspector, t, cx)),
            )
            .child(
                div()
                    .id("inspector-pane")
                    .flex_1()
                    .min_h(px(PANE_MIN_HEIGHT))
                    .flex()
                    .bg(t.background.hsla())
                    .child(self.inspector.clone()),
            )
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let (proxy, live) = {
            let ws = self.workspace.read(cx);
            (ws.proxy.clone(), ws.is_live())
        };
        let toolbar = toolbar::render(&proxy, live, &t, cx).into_any_element();
        let tabs = tabs::render(self.workspace.read(cx), &t);
        div()
            .id("main-window")
            .size_full()
            .flex()
            .flex_row()
            .text_size(px(13.))
            .text_color(t.foreground.hsla())
            .on_drag_move(cx.listener(|this, e: &DragMoveEvent<Split>, _, cx| {
                if *e.drag(cx) != Split::Sidebar {
                    return;
                }
                let x = f32::from(e.event.position.x) - f32::from(e.bounds.origin.x);
                this.sidebar_width = x.clamp(SIDEBAR_RANGE.0, SIDEBAR_RANGE.1);
                cx.notify();
            }))
            .child(self.sidebar_pane(&t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(MAIN_MIN_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(t.background.hsla())
                    .child(toolbar)
                    .children(tabs)
                    .child(self.filter_bar.clone())
                    .child(self.content(&t, cx))
                    .child(self.footer.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
