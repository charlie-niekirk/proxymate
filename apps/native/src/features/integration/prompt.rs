//! The LAN "Allow 10.0.0.23?" prompt (`DeviceAllowPrompt.tsx`) and the
//! toolbar's "N devices waiting" (`Toolbar.tsx` `WaitingDevices`). The
//! prompt asks about the first LAN client nobody has answered for, with a
//! nickname for the Client column and a countdown: its connections wait up
//! to 30 s. Closing it leaves the device pending (the toolbar shows it
//! waiting, and its connections are refused once they time out); the
//! toolbar's button asks again.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, App, Context, Entity, FocusHandle, Focusable, FontWeight, Global, MouseButton, Role, SharedString,
    Subscription, Window, div, px,
};
use pm_model::{DeviceClient, DevicesSnapshot};
use pm_theme::kit::TokensExt as _;
use pm_view::devices as view;

use super::ui;
use crate::bridge::{self, mutations, stream::Latest};
use crate::features::context_menu::Menu;

/// How often the countdown redraws.
const TICK: Duration = Duration::from_millis(250);

/// Which device the prompt shows, and which prompts were closed.
#[derive(Default)]
pub struct Prompts {
    pub dismissed: HashSet<String>,
    showing: Option<(String, Entity<AllowPrompt>)>,
    /// Set while this module closes the dialog itself, so its close isn't
    /// taken for the user's.
    closing: bool,
}

impl Global for Prompts {}

impl Prompts {
    /// The device the open prompt asks about.
    pub fn showing(cx: &App) -> Option<String> {
        cx.try_global::<Prompts>()?.showing.as_ref().map(|(ip, _)| ip.clone())
    }

    pub fn prompt(cx: &App) -> Option<Entity<AllowPrompt>> {
        cx.try_global::<Prompts>()?.showing.as_ref().map(|(_, p)| p.clone())
    }
}

pub fn register(cx: &mut App) {
    let devices = bridge::devices(cx);
    cx.observe(&devices, |_, cx| sync(cx)).detach();
}

fn devices_now(cx: &mut App) -> Vec<DeviceClient> {
    bridge::devices(cx)
        .read(cx)
        .value
        .as_ref()
        .map(|s| s.devices.clone())
        .unwrap_or_default()
}

/// Shows the prompt for the device that should have it now: opens,
/// replaces or closes the dialog.
pub fn sync(cx: &mut App) {
    let devices = devices_now(cx);
    let prompts = cx.default_global::<Prompts>();
    let want = view::prompt_for(&devices, &prompts.dismissed).map(|d| d.ip.clone());
    let showing = prompts.showing.as_ref().map(|(ip, _)| ip.clone());
    if want == showing {
        return;
    }
    if showing.is_some() {
        prompts.showing = None;
        prompts.closing = true;
        super::in_main_window(cx, |window, cx| {
            window.close_dialog(cx);
            cx.default_global::<Prompts>().closing = false;
        });
    }
    if let Some(ip) = want {
        let entity = cx.new(|cx| AllowPrompt::new(ip.clone(), cx));
        cx.default_global::<Prompts>().showing = Some((ip, entity.clone()));
        super::in_main_window(cx, move |window, cx| open_dialog(entity, window, cx));
    }
}

fn open_dialog(prompt: Entity<AllowPrompt>, window: &mut Window, cx: &mut App) {
    // Still the one to show (a decision may have landed meanwhile)?
    if Prompts::prompt(cx).as_ref() != Some(&prompt) {
        return;
    }
    let ip = prompt.read(cx).ip.clone();
    let content = prompt.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let ip = ip.clone();
        dialog
            .w(px(400.))
            .on_close(move |_, _, cx| dismiss(&ip, cx))
            .child(content.clone())
    });
    // After the dialog took the focus: the nickname field has it.
    prompt.update(cx, |p, cx| p.attach(window, cx));
}

/// The user closed the prompt: the device stays pending, waiting in the
/// toolbar; the next one (if any) is asked about.
pub fn dismiss(ip: &str, cx: &mut App) {
    let prompts = cx.default_global::<Prompts>();
    if prompts.closing {
        return;
    }
    prompts.dismissed.insert(ip.to_owned());
    if prompts.showing.as_ref().is_some_and(|(s, _)| s == ip) {
        prompts.showing = None;
    }
    sync(cx);
}

/// "N devices waiting": ask about them again.
pub fn reopen(cx: &mut App) {
    cx.default_global::<Prompts>().dismissed.clear();
    sync(cx);
}

/// The prompt's content.
pub struct AllowPrompt {
    pub ip: String,
    nickname: Option<Entity<InputState>>,
    pub error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AllowPrompt {
    pub fn new(ip: String, cx: &mut Context<Self>) -> AllowPrompt {
        let devices = bridge::devices(cx);
        let subscriptions = vec![cx.observe(&devices, |_, _, cx| cx.notify())];
        // The countdown.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        AllowPrompt {
            ip,
            nickname: None,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Makes the nickname field (it needs the window) and focuses it.
    pub fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.nickname.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Charlie's iPhone"));
        let sub = cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                this.decide(true, cx);
            }
        });
        self._subscriptions.push(sub);
        input.update(cx, |s, cx| s.focus(window, cx));
        self.nickname = Some(input);
    }

    fn nickname(&self, cx: &App) -> Option<String> {
        let text = self.nickname.as_ref()?.read(cx).value().trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// Allow (with the nickname) or Deny.
    pub fn decide(&mut self, allow: bool, cx: &mut Context<Self>) {
        self.error = None;
        let nickname = if allow { self.nickname(cx) } else { None };
        let this = cx.weak_entity();
        mutations::decide_device(
            self.ip.clone(),
            allow,
            nickname,
            move |answer, cx| {
                if let Err(err) = answer {
                    let _ = this.update(cx, |p, cx| {
                        p.error = Some(err.message);
                        cx.notify();
                    });
                }
            },
            cx,
        );
        cx.notify();
    }
}

impl Focusable for AllowPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The title's menu: Copy IP address.
pub fn title_menu(ip: &str) -> Menu {
    Menu::new().copy("Copy IP address", ip.to_owned(), "IP address")
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

impl Render for AllowPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let device = devices_now(cx).into_iter().find(|d| d.ip == self.ip);
        let ip = self.ip.clone();
        let title = div()
            .id("device-allow-title")
            .flex()
            .items_center()
            .gap(px(8.))
            .role(Role::Heading)
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(pm_widgets::icon(IconName::Smartphone, t.foreground.hsla(), 16.))
            .child(format!("Allow {ip}?"))
            .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                title_menu(&ip).show(e.position, window, cx);
            });
        let countdown = device.as_ref().filter(|d| d.waiting > 0).map(|d| {
            let (text, fill) = view::countdown(d.waiting, d.held_until_ms, now_ms());
            div()
                .id("device-countdown")
                .flex()
                .items_center()
                .gap(px(8.))
                .role(Role::Status)
                .aria_label(SharedString::from(text.clone()))
                .text_color(t.muted_foreground.hsla())
                .child(div().min_w_0().flex_1().child(text))
                .when_some(fill, |d, fill| {
                    d.child(
                        div()
                            .flex_none()
                            .w(px(80.))
                            .h(px(4.))
                            .rounded_full()
                            .bg(t.muted.hsla())
                            .child(div().h_full().w(px(80. * fill)).rounded_full().bg(t.s_4xx.hsla())),
                    )
                })
                .test_support()
        });
        let footer = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .mt(px(4.))
            .child(
                Button::new("device-deny")
                    .ghost()
                    .small()
                    .label("Deny")
                    .on_click(cx.listener(|p, _, _, cx| p.decide(false, cx))),
            )
            .child(
                Button::new("device-allow")
                    .primary()
                    .small()
                    .label("Allow")
                    .on_click(cx.listener(|p, _, _, cx| p.decide(true, cx))),
            );
        div()
            .id("device-allow-prompt")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(10.))
            .text_size(px(12.))
            .role(Role::Dialog)
            .aria_label(SharedString::from(format!("Allow {}?", self.ip)))
            .child(title)
            .child(div().text_color(t.muted_foreground.hsla()).child(
                "A device on your network wants to use Proxymate as its proxy. Only allow devices you are setting \
                 up: an allowed device's traffic is recorded here.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_color(t.muted_foreground.hsla())
                            .child("Nickname (shown in the Client column)"),
                    )
                    .children(
                        self.nickname
                            .as_ref()
                            .map(|n| Input::new(n).small().aria_label("Nickname").into_any_element()),
                    ),
            )
            .children(countdown)
            .when_some(self.error.clone(), |d, e| d.child(ui::error(e, &t)))
            .child(footer)
            .test_support()
    }
}

/// The toolbar's "N devices waiting".
pub struct WaitingDevices {
    devices: Entity<Latest<DevicesSnapshot>>,
    _subscription: Subscription,
}

pub fn waiting_devices(_: &mut Window, cx: &mut App) -> AnyView {
    let devices = bridge::devices(cx);
    cx.new(|cx| WaitingDevices {
        _subscription: cx.observe(&devices, |_, _, cx| cx.notify()),
        devices,
    })
    .into()
}

impl WaitingDevices {
    pub fn count(&self, cx: &App) -> usize {
        let Some(snapshot) = self.devices.read(cx).value.as_ref() else {
            return 0;
        };
        let empty = HashSet::new();
        let dismissed = cx.try_global::<Prompts>().map_or(&empty, |p| &p.dismissed);
        view::waiting_count(&snapshot.devices, dismissed)
    }
}

impl Render for WaitingDevices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let n = self.count(cx);
        if n == 0 {
            return div().into_any_element();
        }
        let label = view::waiting_label(n);
        Button::new("devices-waiting")
            .outline()
            .small()
            .accessibility_label(SharedString::from(label.clone()))
            .tooltip("A device on your network wants to use the proxy · click to allow or deny it")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(t.s_4xx.hsla())
                    .child(pm_widgets::icon(IconName::Smartphone, t.s_4xx.hsla(), 14.))
                    .child(label),
            )
            .on_click(|_, _, cx| reopen(cx))
            .into_any_element()
    }
}
