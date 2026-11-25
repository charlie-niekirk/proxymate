//! The toolbar's Intercept popover (`InterceptMenu.tsx`, M21): a terminal
//! or a browser with a fresh profile, started already pointed at the proxy
//! and trusting its CA; their flows show as `<App> (Intercepted)`. The
//! running launches are read every 2 s while it is open, 10 s otherwise
//! (they end when their app quits). Launching is a mutation: a real one
//! needs the user's OK, and tests journal it.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, Context, ElementId, Role, SharedString, Subscription, Window, div, px,
};
use pm_model::{InterceptApp, InterceptSession, InterceptTarget};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::integration::{self as view, INTERCEPT_TARGETS, InterceptEntry};

use super::ui;
use crate::bridge::keys::{IntegrationCaches, integration_caches};
use crate::bridge::mutations;

const POLL_OPEN: Duration = Duration::from_secs(2);
const POLL_CLOSED: Duration = Duration::from_secs(10);

pub struct InterceptTool {
    caches: IntegrationCaches,
    pub open: bool,
    _subscriptions: Vec<Subscription>,
}

pub fn intercept_tool(_: &mut Window, cx: &mut App) -> AnyView {
    let tool = cx.new(InterceptTool::new);
    super::track_tool(super::Tool::Intercept, &tool, cx);
    tool.into()
}

impl InterceptTool {
    pub fn new(cx: &mut Context<Self>) -> InterceptTool {
        let caches = integration_caches(cx);
        let subscriptions = vec![
            cx.observe(&caches.intercepts, |_, _, cx| cx.notify()),
            cx.observe(&caches.intercept_apps, |_, _, cx| cx.notify()),
        ];
        // Launches end when their app quits: read them again now and then.
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(open) = this.read_with(cx, |t, _| t.open) else {
                    break;
                };
                cx.background_executor()
                    .timer(if open { POLL_OPEN } else { POLL_CLOSED })
                    .await;
                let polled = this.update(cx, |t, cx| {
                    t.caches.intercepts.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
                });
                if polled.is_err() {
                    break;
                }
            }
        })
        .detach();
        InterceptTool {
            caches,
            open: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            self.caches.intercepts.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
            self.caches
                .intercept_apps
                .update(cx, |c, cx| c.invalidate(|_, _| true, cx));
        }
        cx.notify();
    }

    /// Starts `target` (the popover closes first).
    pub fn start(&mut self, target: InterceptTarget, cx: &mut Context<Self>) {
        self.set_open(false, cx);
        mutations::intercept(target, cx);
    }
}

fn installed(apps: Option<&[InterceptApp]>, target: InterceptTarget) -> bool {
    apps.and_then(|a| a.iter().find(|a| a.target == target))
        .is_none_or(|a| a.installed)
}

/// The popover's content, from what's installed and running.
pub fn panel(
    apps: Option<&[InterceptApp]>,
    running: &[InterceptSession],
    t: &Tokens,
    start: impl Fn(InterceptTarget, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let item = |e: &InterceptEntry| -> AnyElement {
        let ok = installed(apps, e.target);
        let detail = if e.terminal {
            "curl, node, python, git…"
        } else if ok {
            ""
        } else {
            "not installed"
        };
        let target = e.target;
        let start = start.clone();
        Button::new(ElementId::Name(format!("intercept-{}", e.id).into()))
            .ghost()
            .small()
            .w_full()
            .disabled(!ok)
            .accessibility_label(SharedString::from(if e.terminal {
                format!("New {} window", e.name)
            } else {
                format!("Launch {}", e.name)
            }))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(pm_widgets::icon(
                        if e.terminal {
                            IconName::SquareTerminal
                        } else {
                            IconName::AppWindow
                        },
                        t.foreground.hsla(),
                        14.,
                    ))
                    .child(div().flex_1().text_size(px(12.)).child(e.name))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.muted_foreground.hsla())
                            .child(detail),
                    ),
            )
            .on_click(move |_, window, cx| start(target, window, cx))
            .into_any_element()
    };
    let heading = |text: &'static str, first: bool| {
        div()
            .px(px(8.))
            .pt(px(if first { 4. } else { 8. }))
            .pb(px(4.))
            .when(!first, |d| d.mt(px(4.)).border_t_1().border_color(t.border.hsla()))
            .text_color(t.muted_foreground.hsla())
            .child(text)
            .into_any_element()
    };
    let mut children = vec![heading("New window with the proxy and CA set", true)];
    children.extend(INTERCEPT_TARGETS.iter().filter(|e| e.terminal).map(item));
    children.push(heading("Browser with a fresh profile", false));
    children.extend(INTERCEPT_TARGETS.iter().filter(|e| !e.terminal).map(item));
    if !running.is_empty() {
        children.push(heading("Running", false));
        children.extend(running.iter().map(|r| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .id(("intercept-running", r.id as usize))
                .role(Role::ListItem)
                .child(ui::dot(t.s_2xx))
                .child(div().flex_1().truncate().child(format!("{} (Intercepted)", r.name)))
                .child(
                    div()
                        .font_family(ui::MONO)
                        .text_size(px(11.))
                        .text_color(t.muted_foreground.hsla())
                        .child(view::intercept_pid(r)),
                )
                .into_any_element()
        }));
    }
    div()
        .id("intercept-popover")
        .w(px(320.))
        .p(px(4.))
        .flex()
        .flex_col()
        .text_size(px(12.))
        .role(Role::Menu)
        .aria_label("Intercept")
        .children(children)
        .test_support()
        .into_any_element()
}

impl Render for InterceptTool {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let apps = self
            .caches
            .intercept_apps
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .cloned();
        let running = self
            .caches
            .intercepts
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .cloned()
            .unwrap_or_default();
        let count = running.len();
        let this = cx.weak_entity();
        let start_this = this.clone();
        let start = move |target, _: &mut Window, cx: &mut App| {
            let _ = start_this.update(cx, |tool, cx| tool.start(target, cx));
        };
        Popover::new("intercept-menu-popover")
            .anchor(Anchor::TopRight)
            .p_0()
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let _ = this.update(cx, |tool, cx| tool.set_open(*open, cx));
            })
            .trigger(
                Button::new("intercept-menu")
                    .ghost()
                    .small()
                    .accessibility_label(SharedString::from(if count > 0 {
                        format!("Intercept: {count} running")
                    } else {
                        "Intercept".into()
                    }))
                    .tooltip(view::INTERCEPT_TIP)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(pm_widgets::icon(IconName::Crosshair, t.foreground.hsla(), 15.))
                            .when(count > 0, |d| {
                                d.child(div().text_size(px(12.)).child(count.to_string()))
                            }),
                    ),
            )
            .content(move |_, _, _| panel(apps.as_deref(), &running, &t, start.clone()))
    }
}
