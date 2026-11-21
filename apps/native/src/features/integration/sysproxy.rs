//! The toolbar's system proxy popover (`SystemProxyMenu.tsx`): the macOS
//! web and secure web proxy on every active network service, the services
//! it set, and "Copy terminal env" for tools that ignore it. The previous
//! settings come back on quit, on a signal and, after `kill -9`, from the
//! guardian (`pm_app`, `--sysproxy-guardian`).

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Anchor, AnyView, App, Context, Entity, FontWeight, MouseButton, Role, Subscription, Window, div, px};
use pm_model::SystemProxyStatus;
use pm_theme::kit::TokensExt as _;
use pm_view::integration as view;

use super::ui;
use crate::bridge::keys::{IntegrationCaches, integration_caches};
use crate::bridge::mutations;
use crate::features::clipboard;
use crate::features::context_menu::Menu;
use crate::workspace::Workspace;

const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The popover's content.
pub struct SystemProxyPanel {
    caches: IntegrationCaches,
    pub busy: bool,
    copied: bool,
    _subscriptions: Vec<Subscription>,
}

impl SystemProxyPanel {
    pub fn new(cx: &mut Context<Self>) -> SystemProxyPanel {
        let caches = integration_caches(cx);
        let mut subscriptions = vec![cx.observe(&caches.system_proxy, |_, _, cx| cx.notify())];
        if let Some(ws) = Workspace::global(cx) {
            subscriptions.push(cx.observe(&ws, |_, _, cx| cx.notify()));
        }
        SystemProxyPanel {
            caches,
            busy: false,
            copied: false,
            _subscriptions: subscriptions,
        }
    }

    /// Use as system proxy / Turn off.
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let on = !self.caches.system_proxy.read(cx).peek(&()).is_some_and(|s| s.enabled);
        self.busy = true;
        cx.notify();
        let this = cx.weak_entity();
        mutations::set_system_proxy(
            on,
            move |cx| {
                let _ = this.update(cx, |p, cx| {
                    p.busy = false;
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn copy_env(&mut self, cx: &mut Context<Self>) {
        let Some(env) = Workspace::global(cx).and_then(|w| w.read(cx).terminal_env()) else {
            return;
        };
        clipboard::copy_silent(&env, cx);
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |p, cx| {
                p.copied = false;
                cx.notify();
            });
        })
        .detach();
    }
}

/// The services line's menu.
pub fn services_menu(status: &SystemProxyStatus) -> Menu {
    Menu::new().copy("Copy services", status.services.join("\n"), "services")
}

impl Render for SystemProxyPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let status = self
            .caches
            .system_proxy
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .cloned();
        let listening = Workspace::global(cx).and_then(|w| w.read(cx).proxy.listening.clone());
        let enabled = status.as_ref().is_some_and(|s| s.enabled);
        let supported = status.as_ref().is_some_and(|s| s.supported);
        let head = div()
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(t.border.hsla())
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .id("system-proxy-title")
                    .role(Role::Heading)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::dot(if enabled { t.s_2xx } else { t.muted_foreground }))
                    .child(view::system_proxy_title(status.as_ref())),
            )
            .child(
                div()
                    .text_color(t.muted_foreground.hsla())
                    .child(view::system_proxy_description(listening.as_deref())),
            );
        let services = status.as_ref().filter(|s| !s.services.is_empty()).map(|s| {
            let menu_status = s.clone();
            div()
                .id("system-proxy-services")
                .flex()
                .flex_wrap()
                .child(div().text_color(t.muted_foreground.hsla()).child("Services:\u{a0}"))
                .child(s.services.join(", "))
                .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    services_menu(&menu_status).show(e.position, window, cx);
                })
        });
        let toggle = {
            let b = Button::new("system-proxy-toggle")
                .small()
                .w_full()
                .disabled(self.busy || !supported || listening.is_none())
                .loading(self.busy)
                .label(if enabled { "Turn off" } else { "Use as system proxy" })
                .on_click(cx.listener(|p, _, _, cx| p.toggle(cx)));
            if enabled { b.outline() } else { b.primary() }
        };
        let body = div()
            .px(px(12.))
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(services)
            .when(status.as_ref().is_some_and(|s| !s.supported), |d| {
                d.child(
                    div()
                        .text_color(t.muted_foreground.hsla())
                        .child("Not available on this platform yet."),
                )
            })
            .when_some(status.as_ref().and_then(|s| s.error.clone()), |d, e| {
                d.child(ui::error(e, &t))
            })
            .child(toggle);
        let copied = self.copied;
        let env = div()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.border.hsla())
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                div()
                    .text_color(t.muted_foreground.hsla())
                    .child("For curl, Node, Python and other terminal tools"),
            )
            .child(
                Button::new("copy-terminal-env")
                    .outline()
                    .xsmall()
                    .icon(IconName::Copy)
                    .label(if copied { "Copied" } else { "Copy terminal env" })
                    .on_click(cx.listener(|p, _, _, cx| p.copy_env(cx))),
            );
        div()
            .id("system-proxy-panel")
            .w(px(320.))
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(head)
            .child(body)
            .child(env)
            .test_support()
    }
}

/// The toolbar's globe: green while the system proxy points at us.
pub struct SystemProxyTool {
    caches: IntegrationCaches,
    panel: Entity<SystemProxyPanel>,
    pub open: bool,
    _subscriptions: Vec<Subscription>,
}

pub fn system_proxy_tool(_: &mut Window, cx: &mut App) -> AnyView {
    let tool = cx.new(|cx| {
        let caches = integration_caches(cx);
        let subscriptions = vec![cx.observe(&caches.system_proxy, |_, _, cx| cx.notify())];
        SystemProxyTool {
            caches,
            panel: cx.new(SystemProxyPanel::new),
            open: false,
            _subscriptions: subscriptions,
        }
    });
    super::track_tool(super::Tool::SystemProxy, &tool, cx);
    tool.into()
}

impl SystemProxyTool {
    pub fn panel(&self) -> &Entity<SystemProxyPanel> {
        &self.panel
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            // Someone may have changed the network settings meanwhile.
            self.caches
                .system_proxy
                .update(cx, |c, cx| c.invalidate(|_, _| true, cx));
        }
        cx.notify();
    }
}

impl Render for SystemProxyTool {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let enabled = self
            .caches
            .system_proxy
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .is_some_and(|s| s.enabled);
        let colour = if enabled { t.s_2xx } else { t.muted_foreground };
        let panel = self.panel.clone();
        let this = cx.weak_entity();
        Popover::new("system-proxy-popover")
            .anchor(Anchor::TopRight)
            .p_0()
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let _ = this.update(cx, |tool, cx| tool.set_open(*open, cx));
            })
            .trigger(
                Button::new("system-proxy-menu")
                    .ghost()
                    .small()
                    .accessibility_label(if enabled {
                        "System proxy: on"
                    } else {
                        "System proxy: off"
                    })
                    .tooltip("System proxy")
                    .child(pm_widgets::icon(IconName::Globe, colour.hsla(), 15.)),
            )
            .content(move |_, _, _| panel.clone())
    }
}
