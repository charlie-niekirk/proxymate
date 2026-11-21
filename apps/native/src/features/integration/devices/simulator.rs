//! The iOS Simulator tab (`DeviceSetup/SimulatorTab.tsx`). The Simulator
//! uses this Mac's network settings, so it needs the system proxy and the
//! CA in each device's keychain (`simctl keychain add-root-cert`).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, ElementId, MouseButton, Role, SharedString, div, px};
use pm_model::{DevicesSnapshot, SimDevice};
use pm_theme::Tokens;
use pm_view::devices::{self as view, Check, CheckState};

use super::{Busy, DeviceSetup, InstallSimulatorCert};
use crate::features::context_menu::Menu;
use crate::features::integration::ui;

/// A booted simulator's menu.
pub fn row_menu(d: &SimDevice, busy: bool) -> Menu {
    let menu = Menu::new()
        .copy("Copy name", d.name.clone(), "name")
        .copy("Copy UDID", d.udid.clone(), "UDID")
        .copy("Copy runtime", d.runtime.clone(), "runtime")
        .separator();
    let action = InstallSimulatorCert {
        udid: d.udid.clone().into(),
    };
    if busy {
        menu.disabled("Install certificate on this simulator", action)
    } else {
        menu.item("Install certificate on this simulator", action)
    }
}

impl DeviceSetup {
    pub(super) fn simulator_tab(
        &mut self,
        snapshot: &DevicesSnapshot,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sim = self
            .caches
            .simulator
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .cloned();
        let fetching = self.caches.simulator.read(cx).is_fetching(&());
        let sysproxy = self
            .caches
            .system_proxy
            .update(cx, |c, cx| c.get(&(), cx))
            .ready()
            .cloned();
        let busy = self.busy.clone();
        let booted = sim.as_ref().map(|s| s.booted.clone()).unwrap_or_default();
        let [tools, booted_check] = view::simulator_checks(sim.as_ref());
        let mut lines = vec![ui::check(&tools, t), ui::check(&booted_check, t)];

        if !booted.is_empty() {
            let rows = booted.iter().map(|d| {
                let result = self
                    .sim_results
                    .as_ref()
                    .and_then(|r| r.iter().find(|r| r.udid == d.udid));
                let failed = result.is_some_and(|r| r.error.is_some());
                let (state, colour) = if failed {
                    ("Failed", t.s_failed)
                } else if d.cert_installed {
                    ("Certificate installed", t.s_2xx)
                } else {
                    ("Not installed", t.muted_foreground)
                };
                let menu_device = d.clone();
                let menu_busy = busy.is_some();
                div()
                    .id(ElementId::Name(format!("simulator-{}", d.udid).into()))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(format!("{} {}: {state}", d.name, d.runtime)))
                    .child(div().truncate().child(d.name.clone()))
                    .child(div().text_color(t.muted_foreground.hsla()).child(d.runtime.clone()))
                    .child(div().ml_auto().flex_none().text_color(colour.hsla()).child(state))
                    .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                        row_menu(&menu_device, menu_busy).show(e.position, window, cx);
                    })
                    .into_any_element()
            });
            lines.push(
                div()
                    .id("simulator-devices")
                    .ml(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .role(Role::List)
                    .children(rows)
                    .into_any_element(),
            );
        }
        if let Some(results) = &self.sim_results {
            for r in results.iter().filter(|r| r.error.is_some()) {
                lines.push(
                    div()
                        .ml(px(24.))
                        .text_color(t.s_failed.hsla())
                        .child(format!("{}: {}", r.name, r.error.clone().unwrap_or_default()))
                        .into_any_element(),
                );
            }
        }

        let proxy_on = sysproxy.as_ref().is_some_and(|s| s.enabled);
        let proxy_check = Check::new(
            match &sysproxy {
                None => CheckState::Pending,
                Some(s) if s.enabled => CheckState::Ok,
                Some(_) => CheckState::Warn,
            },
            if proxy_on {
                "The system proxy points at Proxymate"
            } else {
                "The system proxy is off"
            },
            None,
        );
        let proxy_extra = (!proxy_on).then(|| {
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(6.))
                .text_color(t.muted_foreground.hsla())
                .child("Simulators use this Mac's network settings, so they need the system proxy.")
                .child(
                    Button::new("simulator-system-proxy")
                        .outline()
                        .xsmall()
                        .disabled(busy.is_some() || !sysproxy.as_ref().is_some_and(|s| s.supported))
                        .loading(busy == Some(Busy::SystemProxy))
                        .label("Turn on system proxy")
                        .on_click(cx.listener(|s, _, _, cx| s.system_proxy_on(cx))),
                )
                .into_any_element()
        });
        lines.push(ui::check_with(&proxy_check, proxy_extra, t));
        lines.push(ui::check(
            &view::progress_check(
                view::activity_progress(&snapshot.simulator),
                "the Simulator",
                snapshot.simulator.flows,
            ),
            t,
        ));

        let installing = busy == Some(Busy::SimInstall);
        let buttons = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                Button::new("simulator-install")
                    .primary()
                    .small()
                    .disabled(busy.is_some() || booted.is_empty())
                    .loading(installing)
                    .icon(IconName::ShieldPlus)
                    .label(if view::all_installed(sim.as_ref()) {
                        "Install again"
                    } else {
                        "Install to all booted"
                    })
                    .on_click(cx.listener(|s, _, _, cx| s.install_simulators(Vec::new(), cx))),
            )
            .child(
                Button::new("simulator-refresh")
                    .ghost()
                    .small()
                    .disabled(fetching)
                    .loading(fetching)
                    .icon(IconName::RefreshCw)
                    .label("Refresh")
                    .on_click(cx.listener(|s, _, _, cx| s.refresh(cx))),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(ui::checklist(lines, t))
            .when_some(self.error.clone(), |d, e| d.child(ui::error(e, t)))
            .child(buttons)
            .child(ui::section(
                Some("How it works"),
                vec![ui::rich(
                    &[
                        ("Proxymate runs ", false),
                        ("xcrun simctl keychain <device> add-root-cert", true),
                        (
                            " for each booted simulator, which trusts the CA for every app on it. Quit and reopen \
                             apps that were already running, then open an https:// page in Safari. A simulator you \
                             erase or create later needs installing again.",
                            false,
                        ),
                    ],
                    t,
                )],
            ))
            .into_any_element()
    }
}
