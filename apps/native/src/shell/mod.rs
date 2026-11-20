//! The main window: a transparent window with the traffic lights where the
//! Tauri app puts them ([`TRAFFIC_LIGHTS`]), sidebar vibrancy behind it, and the three-pane layout of
//! `AppShell.tsx` (sidebar | toolbar, content, footer). Every pane paints an
//! opaque token except the sidebar, which is transparent on macOS so the
//! `NSVisualEffectView` shows through.
//!
//! G0 fills the panes with placeholders; G1 puts the flow table and the
//! inspector in the content area, G2 the toolbar, sidebar and footer.

mod chrome;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Selectable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontWeight, IntoElement, Render, SharedString, Subscription, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, div, point, px, size,
};
use pm_model::{ProxyStatus, StorageStatus};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_widgets::Pill;

use crate::bridge::{self, Bridge, stream::Latest};
use crate::prefs::{self, Appearance};

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
pub const FOOTER_HEIGHT: f32 = 28.;

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
    let opened = cx.open_window(options, |window, cx| {
        if let Err(err) = pm_platform::window::apply_sidebar_vibrancy(window) {
            eprintln!("vibrancy unavailable: {err}");
        }
        let view = cx.new(|cx| MainWindow::new(storage, proxy, window, cx));
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

pub struct MainWindow {
    storage: Entity<Latest<StorageStatus>>,
    proxy: ProxyStatus,
    _subscriptions: Vec<Subscription>,
}

impl MainWindow {
    /// The window over a storage stream and the proxy's status: the bridge's
    /// in the app, injected snapshots in tests.
    pub fn new(
        storage: Entity<Latest<StorageStatus>>,
        proxy: ProxyStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&storage, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| crate::appearance::window_changed(window, cx)),
        ];
        MainWindow {
            storage,
            proxy,
            _subscriptions: subscriptions,
        }
    }

    fn sidebar(&self, t: &Tokens) -> impl IntoElement {
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
            .w(px(SIDEBAR_WIDTH))
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

    fn content(&self, t: &Tokens, appearance: Appearance) -> impl IntoElement {
        let pills = [
            ("GET", t.m_get, t.m_get_bg),
            ("POST", t.m_post, t.m_post_bg),
            ("PUT", t.m_put, t.m_put_bg),
            ("PATCH", t.m_patch, t.m_patch_bg),
            ("DELETE", t.m_delete, t.m_delete_bg),
            ("200", t.s_2xx, t.s_2xx_bg),
            ("301", t.s_3xx, t.s_3xx_bg),
            ("404", t.s_4xx, t.s_4xx_bg),
            ("500", t.s_5xx, t.s_5xx_bg),
        ];
        // gpui-kit buttons: real controls with accessibility roles and names.
        let choice = |id: &'static str, label: &'static str, value: Appearance| {
            let button = Button::new(id)
                .label(label)
                .small()
                .selected(appearance == value)
                .on_click(move |_, _, cx| crate::appearance::set(value, cx));
            if appearance == value {
                button.primary()
            } else {
                button.ghost()
            }
        };
        div()
            .id("content")
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.pane_table.hsla())
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .children(pills.map(|(label, fg, bg)| Pill::new(label, fg, bg))),
            )
            .child(
                div()
                    .text_color(t.muted_foreground.hsla())
                    .child("Proxymate Native — the flow table arrives in G1."),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border.hsla())
                    .bg(t.background.hsla())
                    .child(choice("appearance-system", "System", Appearance::System))
                    .child(choice("appearance-light", "Light", Appearance::Light))
                    .child(choice("appearance-dark", "Dark", Appearance::Dark)),
            )
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
        let appearance = prefs::get(cx).appearance;
        div()
            .id("main-window")
            .size_full()
            .flex()
            .flex_row()
            .text_size(px(13.))
            .text_color(t.foreground.hsla())
            .child(self.sidebar(&t))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(t.background.hsla())
                    .child(self.toolbar(&t))
                    .child(self.content(&t, appearance))
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
