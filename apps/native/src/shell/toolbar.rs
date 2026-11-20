//! The toolbar (`components/Toolbar.tsx`), drawn in the transparent title
//! bar: Record (⌘E), Compose (G8 fills it), Clear, and the "Listening on…"
//! pill with its copy menu. Its empty space drags the window. The tools on
//! the right (Intercept, Rules, Scripts, throttling, breakpoints, system
//! proxy, certificate, MCP, Settings) arrive with G7, G10, G12 and G13 in
//! [`TOOLS_ID`]'s slot.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, MouseButton, MouseDownEvent, Role, SharedString, div, px};
use pm_model::{ListenerKind, ProxyStatus};
use pm_theme::Tokens;

use crate::features::chrome::{Compose, OpenDeviceSetup, OpenSettings};
use crate::features::commands::MAC;
use crate::features::context_menu::Menu;
use crate::features::{capture, flow_table};
use crate::shell::{MainWindow, TOOLBAR_HEIGHT, chrome};
use crate::workspace::Workspace;

/// The empty slot later phases put their toolbar buttons in.
pub const TOOLS_ID: &str = "toolbar-tools";

/// The pill's dot colour and text.
pub fn listening(proxy: &ProxyStatus, t: &Tokens) -> (pm_theme::Rgba, String) {
    match (&proxy.listening, &proxy.error) {
        (Some(addr), _) if proxy.recording => (t.s_2xx, format!("Listening on {addr}")),
        (Some(addr), _) => (t.s_paused, format!("Listening on {addr} · paused")),
        (None, Some(err)) => (t.s_5xx, format!("Proxy not running: {err}")),
        (None, None) => (t.s_1xx, "Starting proxy…".into()),
    }
}

/// The pill's menu: the proxy's addresses and how to use them.
pub fn listening_menu(proxy: &ProxyStatus, terminal_env: Option<String>) -> Menu {
    let addr = proxy.listening.clone();
    let mut menu = Menu::new();
    if let Some(addr) = &addr {
        menu = menu
            .copy("Copy address", addr.clone(), "proxy address")
            .copy(
                "Copy curl example",
                format!("curl -x http://{addr} http://example.com/"),
                "command",
            )
            .copy("Copy SOCKS5 address", format!("socks5h://{addr}"), "SOCKS5 address");
    }
    // The reverse proxies (SOCKS5 shares the main port).
    for l in proxy.listeners.iter().filter(|l| l.kind != ListenerKind::Socks5) {
        let Some(at) = &l.addr else { continue };
        let scheme = if l.kind == ListenerKind::ReverseTls {
            "https"
        } else {
            "http"
        };
        menu = menu.copy(
            format!("Copy reverse proxy {at} → {}", l.target.as_deref().unwrap_or("")),
            format!("{scheme}://{at}"),
            "reverse proxy address",
        );
    }
    if let (Some(_), Some(env)) = (&addr, terminal_env) {
        menu = menu.copy("Copy terminal environment", env, "terminal environment");
    }
    if let Some(err) = &proxy.error {
        menu = menu.copy("Copy error", err.clone(), "error");
    }
    menu.separator()
        .item("Device setup…", OpenDeviceSetup)
        .item("Proxy settings…", OpenSettings { page: "proxy".into() })
        .item("Network settings…", OpenSettings { page: "network".into() })
}

/// The listening pill's menu as the app would show it now.
pub fn current_listening_menu(cx: &App) -> Option<Menu> {
    let ws = Workspace::global(cx)?;
    let ws = ws.read(cx);
    Some(listening_menu(&ws.proxy, ws.terminal_env()))
}

fn shortcut(key: &str) -> String {
    pm_view::commands::shortcut_label(key, false, false, MAC)
}

pub fn render(proxy: &ProxyStatus, live: bool, t: &Tokens, cx: &mut Context<MainWindow>) -> impl IntoElement {
    let recording = proxy.recording;
    let record_tip: SharedString = if recording {
        format!("Stop recording · traffic is still forwarded · {}", shortcut("e")).into()
    } else {
        format!("Start recording · traffic is forwarded either way · {}", shortcut("e")).into()
    };
    let record_icon = if recording {
        // A filled square: Lucide's square is an outline.
        div()
            .size(px(10.))
            .rounded(px(2.))
            .bg(t.record_stop.hsla())
            .into_any_element()
    } else {
        pm_widgets::icon(IconName::Play, t.record_start.hsla(), 14.).into_any_element()
    };
    let record = Button::new("record-button")
        .ghost()
        .small()
        .accessibility_label(if recording { "Stop recording" } else { "Start recording" })
        .tooltip(record_tip)
        .child(record_icon)
        .on_click(|_, _, cx| capture::toggle_recording(cx));
    let compose = Button::new("compose-button")
        .ghost()
        .small()
        .accessibility_label("Compose")
        .tooltip("Compose a request · ⌘N")
        .child(pm_widgets::icon(IconName::Plus, t.foreground.hsla(), 15.))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(Compose), cx));
    let clear = Button::new("clear-button")
        .ghost()
        .small()
        .disabled(!live)
        .accessibility_label("Clear")
        .tooltip(SharedString::from(format!(
            "Clear all flows · {}",
            shortcut("backspace")
        )))
        .child(pm_widgets::icon(IconName::Trash, t.foreground.hsla(), 15.))
        .on_click(|_, _, cx| capture::clear(cx));
    chrome::drag("toolbar")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(TOOLBAR_HEIGHT))
        .px(px(8.))
        .bg(t.pane_toolbar.hsla())
        .border_b_1()
        .border_color(t.border.hsla())
        .child(record)
        .child(compose)
        .child(clear)
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(pill(proxy, t, cx)),
        )
        .child(div().id(TOOLS_ID).flex().items_center().gap(px(4.)))
}

fn pill(proxy: &ProxyStatus, t: &Tokens, cx: &mut Context<MainWindow>) -> impl IntoElement {
    let (dot, text) = listening(proxy, t);
    let failed = proxy.listening.is_none() && proxy.error.is_some();
    let tip: SharedString = if failed {
        format!(
            "{} Click to change the port.",
            proxy.error.as_deref().unwrap_or("The proxy isn't running.")
        )
        .into()
    } else {
        "Device setup · connect a phone, simulator or emulator".into()
    };
    let label: SharedString = text.clone().into();
    div()
        .id("listening-status")
        .role(Role::Button)
        .aria_label(label)
        .aria_description("Right-click to copy the proxy's address")
        .h(px(28.))
        .max_w(px(520.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .text_size(px(12.))
        .text_color(t.muted_foreground.hsla())
        .cursor_pointer()
        .hover(|s| s.bg(t.accent.hsla()))
        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot.hsla()))
        .child(div().truncate().child(text))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        // The press is the pill's: the toolbar under it moves the window.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |_, window, cx| {
            if failed {
                window.dispatch_action(Box::new(OpenSettings { page: "proxy".into() }), cx);
            } else {
                window.dispatch_action(Box::new(OpenDeviceSetup), cx);
            }
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|_, e: &MouseDownEvent, window, cx| {
                if let Some(menu) = current_listening_menu(cx) {
                    menu.show(e.position, window, cx);
                }
            }),
        )
        .test_support()
}

/// The empty table's "Copy command" menu (`FlowTable/EmptyState.tsx`).
pub fn empty_state_menu(cx: &App) -> Option<Menu> {
    let table = flow_table::first(cx)?;
    let command = table.read(cx).waiting_command()?;
    Some(Menu::new().copy("Copy command", command, "command"))
}

#[cfg(test)]
mod tests {
    use pm_model::ListenerStatus;

    use super::*;

    fn status() -> ProxyStatus {
        ProxyStatus {
            listening: Some("127.0.0.1:9090".into()),
            error: None,
            recording: true,
            listeners: vec![
                ListenerStatus {
                    kind: ListenerKind::Socks5,
                    addr: Some("127.0.0.1:9090".into()),
                    target: None,
                    error: None,
                },
                ListenerStatus {
                    kind: ListenerKind::ReverseTls,
                    addr: Some("127.0.0.1:8443".into()),
                    target: Some("https://api.example.com".into()),
                    error: None,
                },
            ],
        }
    }

    #[test]
    fn the_pill_menu_copies_every_address() {
        let menu = listening_menu(&status(), Some("export HTTPS_PROXY=http://127.0.0.1:9090\n".into()));
        assert_eq!(
            menu.labels(),
            [
                "Copy address",
                "Copy curl example",
                "Copy SOCKS5 address",
                "Copy reverse proxy 127.0.0.1:8443 → https://api.example.com",
                "Copy terminal environment",
                "—",
                "Device setup…",
                "Proxy settings…",
                "Network settings…",
            ]
        );
        assert_eq!(menu.copy_text("Copy address"), Some("127.0.0.1:9090"));
        assert_eq!(
            menu.copy_text("Copy curl example"),
            Some("curl -x http://127.0.0.1:9090 http://example.com/")
        );
        assert_eq!(menu.copy_text("Copy SOCKS5 address"), Some("socks5h://127.0.0.1:9090"));
        assert_eq!(
            menu.copy_text("Copy reverse proxy 127.0.0.1:8443 → https://api.example.com"),
            Some("https://127.0.0.1:8443")
        );
    }

    #[test]
    fn a_failed_proxy_offers_its_error() {
        let failed = ProxyStatus {
            listening: None,
            error: Some("address in use".into()),
            recording: true,
            listeners: vec![],
        };
        let menu = listening_menu(&failed, Some("unused".into()));
        assert_eq!(menu.copy_text("Copy error"), Some("address in use"));
        assert!(menu.copy_text("Copy address").is_none());
        assert!(menu.copy_text("Copy terminal environment").is_none());
        let t = pm_theme::Mode::Dark.tokens();
        assert_eq!(listening(&failed, &t).1, "Proxy not running: address in use");
        let paused = ProxyStatus {
            recording: false,
            ..status()
        };
        assert_eq!(listening(&paused, &t).1, "Listening on 127.0.0.1:9090 · paused");
        assert_eq!(listening(&ProxyStatus::default(), &t).1, "Starting proxy…");
    }
}
