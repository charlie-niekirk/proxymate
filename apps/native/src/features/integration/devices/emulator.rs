//! The Android emulator tab (`DeviceSetup/EmulatorTab.tsx`). The emulator
//! reaches this Mac's loopback at 10.0.2.2, so it needs no LAN access:
//! point its global proxy there over adb (cleared again on quit) and
//! install the CA as a user certificate.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, ElementId, MouseButton, Role, SharedString, div, px};
use pm_model::{AndroidDevice, DeviceAccess, DevicesSnapshot};
use pm_theme::Tokens;
use pm_view::devices::{self as view, Check, CheckState};

use super::{Busy, DeviceSetup, PushAndroidCert, SetAndroidProxy};
use crate::features::context_menu::Menu;
use crate::features::integration::ui;

/// An emulator row's menu: its values, Push certificate and the proxy.
pub fn row_menu(d: &AndroidDevice, emulator_proxy: Option<&str>, busy: bool) -> Menu {
    let ours = d.proxy.is_some() && d.proxy.as_deref() == emulator_proxy;
    let ready = !busy && d.state == "device";
    let serial: SharedString = d.serial.clone().into();
    let mut menu = Menu::new().copy("Copy serial", d.serial.clone(), "serial");
    if let Some(model) = &d.model {
        menu = menu.copy("Copy model", model.clone(), "model");
    }
    if let Some(proxy) = &d.proxy {
        menu = menu.copy("Copy proxy", proxy.clone(), "proxy");
    }
    let push = PushAndroidCert { serial: serial.clone() };
    let proxy_label = if ours {
        "Stop using Proxymate as its proxy"
    } else {
        "Use Proxymate as its proxy"
    };
    let proxy = SetAndroidProxy { serial, on: !ours };
    menu = menu.separator();
    if ready {
        menu.item("Push certificate", push).item(proxy_label, proxy)
    } else {
        menu.disabled("Push certificate", push).disabled(proxy_label, proxy)
    }
}

impl DeviceSetup {
    pub(super) fn emulator_tab(
        &mut self,
        access: Option<&DeviceAccess>,
        snapshot: &DevicesSnapshot,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = self.caches.android.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        let fetching = self.caches.android.read(cx).is_fetching(&());
        let busy = self.busy.clone();
        let emulators: Vec<AndroidDevice> = status
            .as_ref()
            .map(|s| s.devices.iter().filter(|d| d.emulator).cloned().collect())
            .unwrap_or_default();
        let emulator_proxy = status.as_ref().and_then(|s| s.emulator_proxy.clone());
        let adb = Check::new(
            match &status {
                None => CheckState::Pending,
                Some(s) if s.available => CheckState::Ok,
                Some(_) => CheckState::Warn,
            },
            if status.as_ref().is_some_and(|s| !s.available) {
                "adb wasn't found"
            } else {
                "Android SDK platform-tools"
            },
            status.as_ref().and_then(|s| s.detail.clone()),
        );
        let n = emulators.len();
        let running = Check::new(
            match &status {
                None => CheckState::Pending,
                Some(_) if n > 0 => CheckState::Ok,
                Some(_) => CheckState::Warn,
            },
            if n > 0 {
                format!("{n} running emulator{}", if n == 1 { "" } else { "s" })
            } else {
                "No emulator is running".into()
            },
            status
                .as_ref()
                .filter(|s| s.available && n == 0)
                .map(|_| "Start one from Android Studio's Device Manager.".into()),
        );
        let mut lines = vec![ui::check(&adb, t), ui::check(&running, t)];
        for d in &emulators {
            let ours = d.proxy.is_some() && d.proxy == emulator_proxy;
            let ready = busy.is_none() && d.state == "device";
            let line = if d.state != "device" {
                d.state.clone()
            } else {
                match &d.proxy {
                    Some(p) => format!("proxy {p}"),
                    None => "no proxy".into(),
                }
            };
            let serial = d.serial.clone();
            let toggle_serial = d.serial.clone();
            let menu_device = d.clone();
            let menu_proxy = emulator_proxy.clone();
            let menu_busy = busy.is_some();
            lines.push(
                div()
                    .id(ElementId::Name(format!("emulator-row-{}", d.serial).into()))
                    .ml(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(format!(
                        "{}: {line}",
                        d.model.clone().unwrap_or_else(|| d.serial.clone())
                    )))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .child(d.model.clone().unwrap_or_else(|| d.serial.clone())),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(t.muted_foreground.hsla())
                                    .child(format!("{} · {line}", d.serial)),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("emulator-push-{}", d.serial).into()))
                            .outline()
                            .xsmall()
                            .disabled(!ready)
                            .loading(busy == Some(Busy::PushCert(d.serial.clone())))
                            .icon(IconName::Download)
                            .label("Push certificate")
                            .on_click(cx.listener(move |s, _, _, cx| s.push_cert(serial.clone(), cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                Switch::new(ElementId::Name(format!("emulator-proxy-{}", d.serial).into()))
                                    .small()
                                    .checked(ours)
                                    .disabled(!ready)
                                    .accessibility_label(SharedString::from(format!(
                                        "Use Proxymate as {}'s proxy",
                                        d.serial
                                    )))
                                    .on_click(cx.listener(move |s, on: &bool, _, cx| {
                                        s.set_emulator_proxy(toggle_serial.clone(), *on, cx)
                                    })),
                            )
                            .child("Proxy"),
                    )
                    .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                        row_menu(&menu_device, menu_proxy.as_deref(), menu_busy).show(e.position, window, cx);
                    })
                    .into_any_element(),
            );
        }
        lines.push(ui::check(
            &view::progress_check(
                view::activity_progress(&snapshot.emulator),
                "the emulator",
                snapshot.emulator.flows,
            ),
            t,
        ));
        let (set, clear) = view::emulator_commands(access.and_then(|a| a.port));
        let proxy_text = emulator_proxy.unwrap_or_else(|| "10.0.2.2:9090".into());
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(ui::checklist(lines, t))
            .when_some(self.note.clone(), |d, n| {
                d.child(div().text_color(t.s_2xx.hsla()).child(n))
            })
            .when_some(self.error.clone(), |d, e| d.child(ui::error(e, t)))
            .child(
                div().flex().child(
                    Button::new("emulator-refresh")
                        .ghost()
                        .small()
                        .disabled(fetching)
                        .loading(fetching)
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .on_click(cx.listener(|s, _, _, cx| s.refresh(cx))),
                ),
            )
            .child(ui::section(
                Some("Set up the emulator"),
                vec![ui::steps(
                    vec![
                        ui::rich(
                            &[
                                ("Turn on ", false),
                                ("Proxy", true),
                                (" for the emulator above. It is set to ", false),
                                (&proxy_text, true),
                                (
                                    ", the emulator's address for this Mac, and cleared again when Proxymate quits.",
                                    false,
                                ),
                            ],
                            t,
                        ),
                        ui::rich(
                            &[
                                ("Click ", false),
                                ("Push certificate", true),
                                (", then on the emulator open ", false),
                                (
                                    "Settings › Security & privacy › More security settings › Encryption & \
                                     credentials › Install a certificate › CA certificate",
                                    true,
                                ),
                                (" and pick ", false),
                                ("proxymate-ca.crt", true),
                                (" from Downloads.", false),
                            ],
                            t,
                        ),
                        ui::rich(
                            &[
                                ("For your own app, add the debug-only ", false),
                                ("network_security_config.xml", true),
                                (" from the Android tab.", false),
                            ],
                            t,
                        ),
                    ],
                    t,
                )],
            ))
            .child(ui::section(
                Some("From a terminal"),
                vec![
                    ui::copy_field(
                        "copy-emulator-set",
                        set,
                        "Copy set-proxy command",
                        "command",
                        false,
                        vec![],
                        t,
                    ),
                    ui::copy_field(
                        "copy-emulator-clear",
                        clear,
                        "Copy clear-proxy command",
                        "command",
                        false,
                        vec![],
                        t,
                    ),
                ],
            ))
            .into_any_element()
    }
}
