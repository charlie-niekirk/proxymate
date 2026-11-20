//! The main window: a transparent window with the traffic lights where the
//! Tauri app puts them ([`TRAFFIC_LIGHTS`]), sidebar vibrancy behind it, and the three-pane layout of
//! `AppShell.tsx` (sidebar | toolbar, content, footer). Every pane paints an
//! opaque token except the sidebar, which is transparent on macOS so the
//! `NSVisualEffectView` shows through.
//!
//! G1 puts the flow table in the content area, over an inspector
//! placeholder (G3 fills it), with the sidebar's width and the table's
//! share of the column draggable; G2 fills the toolbar, sidebar and footer.

mod chrome;

use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, DragMoveEvent, Entity, Focusable as _, FontWeight, IntoElement, Orientation, Render,
    Role, SharedString, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    div, point, px, size,
};
use pm_model::{ProxyStatus, StorageStatus};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;

use crate::bridge::{self, Bridge, stream::Latest};
use crate::features::flow_table::FlowTable;

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
    let storage = bridge::storage(cx);
    // An O(1) read of the listener state, fine on the foreground thread.
    let proxy = Bridge::engine(cx).proxy_status();
    let addr: SharedString = proxy
        .listening
        .clone()
        .unwrap_or_else(|| "127.0.0.1:9090".into())
        .into();
    let opened = cx.open_window(options, |window, cx| {
        if let Err(err) = pm_platform::window::apply_sidebar_vibrancy(window) {
            eprintln!("vibrancy unavailable: {err}");
        }
        let table = cx.new(|cx| FlowTable::live(addr, cx));
        let view = cx.new(|cx| MainWindow::new(storage, proxy, table, window, cx));
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

pub struct MainWindow {
    storage: Entity<Latest<StorageStatus>>,
    proxy: ProxyStatus,
    table: Entity<FlowTable>,
    sidebar_width: f32,
    table_share: f32,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    /// The window over a storage stream, the proxy's status and a flow
    /// table: the bridge's in the app, injected snapshots in tests.
    pub fn new(
        storage: Entity<Latest<StorageStatus>>,
        proxy: ProxyStatus,
        table: Entity<FlowTable>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&storage, |_, _, cx| cx.notify()),
            cx.observe(&table, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| crate::appearance::window_changed(window, cx)),
        ];
        // The table has the keyboard from the start.
        let focus = table.read(cx).grid().focus_handle(cx);
        window.focus(&focus, cx);
        MainWindow {
            storage,
            proxy,
            table,
            sidebar_width: SIDEBAR_WIDTH,
            table_share: TABLE_SHARE,
            _subscriptions: subscriptions,
        }
    }

    pub fn table(&self) -> &Entity<FlowTable> {
        &self.table
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

    fn sidebar(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let section = |title: &'static str, item: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .pb(px(10.))
                .child(
                    div()
                        .px(px(8.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted_foreground.hsla())
                        .child(title),
                )
                .child(
                    div()
                        .px(px(8.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .text_color(t.sidebar_foreground.hsla())
                        .child(item),
                )
        };
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
            .child(chrome::drag("sidebar-drag").h(px(TOOLBAR_HEIGHT)).flex_none())
            .child(
                div()
                    .px(px(8.))
                    .flex()
                    .flex_col()
                    .child(section("Favorites", "No favorites yet"))
                    .child(section("Pinned", "Pinned"))
                    .child(section("Saved", "No saved sessions"))
                    .child(section("All", "All traffic")),
            )
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

    fn toolbar(&self, t: &Tokens) -> impl IntoElement {
        let listening: SharedString = match (&self.proxy.listening, &self.proxy.error) {
            (Some(addr), _) => format!("Listening on {addr}").into(),
            (None, Some(err)) => format!("Proxy stopped: {err}").into(),
            (None, None) => "Proxy starting…".into(),
        };
        chrome::drag("toolbar")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(8.))
            .bg(t.pane_toolbar.hsla())
            .border_b_1()
            .border_color(t.border.hsla())
            .child(
                div()
                    .id("listening")
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border.hsla())
                    .text_size(px(12.))
                    .text_color(t.muted_foreground.hsla())
                    .child(listening),
            )
    }

    /// The flow table over the inspector placeholder.
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
            .child(self.inspector(t, cx))
    }

    /// G3's inspector goes here; until then, what is selected.
    fn inspector(&self, t: &Tokens, cx: &App) -> impl IntoElement {
        let grid = self.table.read(cx).grid().read(cx);
        let count = grid.selection_count();
        let focused = grid.selected().focused;
        let line: SharedString = match (count, focused) {
            (0, _) => "No flow selected".into(),
            (1, Some(id)) => {
                let url = (0..grid.len())
                    .filter_map(|i| grid.row(i))
                    .find(|r| r.id() == id)
                    .map(|r| format!("{} {}", r.method, r.url));
                format!("Flow #{}{}", id + 1, url.map(|u| format!(" · {u}")).unwrap_or_default()).into()
            }
            (n, _) => format!("{n} flows selected").into(),
        };
        div()
            .id("inspector")
            .role(Role::Region)
            .aria_label("Inspector")
            .flex_1()
            .min_h(px(PANE_MIN_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .bg(t.background.hsla())
            .text_color(t.muted_foreground.hsla())
            .child(div().text_color(t.foreground.hsla()).child(line))
            .child(div().text_size(px(12.)).child("The inspector arrives in G3."))
    }

    fn footer(&self, t: &Tokens, cx: &App) -> impl IntoElement {
        let text: SharedString = match &self.storage.read(cx).value {
            Some(s) => format!(
                "{} {} · {} on disk",
                s.flows,
                if s.flows == 1 { "flow" } else { "flows" },
                human_bytes(s.disk_bytes)
            )
            .into(),
            None => "…".into(),
        };
        div()
            .id("footer")
            .flex()
            .flex_none()
            .items_center()
            .h(px(FOOTER_HEIGHT))
            .px(px(8.))
            .bg(t.pane_toolbar.hsla())
            .border_t_1()
            .border_color(t.border.hsla())
            .text_size(px(12.))
            .text_color(t.muted_foreground.hsla())
            .child(text)
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
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
            .child(self.sidebar(&t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(MAIN_MIN_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(t.background.hsla())
                    .child(self.toolbar(&t))
                    .child(self.content(&t, cx))
                    .child(self.footer(&t, cx)),
            )
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn sizes_read_like_the_footer() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
