//! The Device setup sheet (`DeviceSetup/DeviceSetupSheet.tsx`), opened from
//! the listening pill: LAN access and the interface devices use, one tab
//! per kind of device (the iOS Simulator, iPhone, Android, the Android
//! emulator; [`simulator`], [`phones`], [`emulator`]), then the LAN clients
//! Proxymate knows. The Simulator and emulator tabs read their devices
//! every 4 s while shown. `simctl`, `adb`, the system proxy and LAN access
//! change the machine: each is a mutation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Disableable as _, Placement, Sizable as _, WindowExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Global, Image, ImageFormat, MouseButton, Role,
    SharedString, Subscription, Window, div, img, px,
};
use pm_model::{DeviceAccess, DeviceClient, DeviceDecision, DevicesSnapshot, SimInstallResult};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::devices as view;

use super::ui;
use crate::bridge::keys::{IntegrationCaches, integration_caches};
use crate::bridge::{self, mutations, stream::Latest};
use crate::features::context_menu::Menu;

pub mod emulator;
pub mod phones;
pub mod simulator;

/// How often the Simulator and emulator tabs read their devices again.
const POLL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceTab {
    Simulator,
    Iphone,
    Android,
    Emulator,
}

impl DeviceTab {
    pub const ALL: [DeviceTab; 4] = [
        DeviceTab::Simulator,
        DeviceTab::Iphone,
        DeviceTab::Android,
        DeviceTab::Emulator,
    ];

    pub fn id(self) -> &'static str {
        match self {
            DeviceTab::Simulator => "simulator",
            DeviceTab::Iphone => "iphone",
            DeviceTab::Android => "android",
            DeviceTab::Emulator => "emulator",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DeviceTab::Simulator => "iOS Simulator",
            DeviceTab::Iphone => "iPhone",
            DeviceTab::Android => "Android",
            DeviceTab::Emulator => "Emulator",
        }
    }

    pub fn parse(id: &str) -> Option<DeviceTab> {
        DeviceTab::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// Picks the LAN interface (its menu).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetLanInterface {
    pub name: SharedString,
}

/// Allows or denies a LAN client (a device row and its menu).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct DecideDevice {
    pub ip: SharedString,
    pub allow: bool,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ForgetDevice {
    pub ip: SharedString,
}

/// "Install certificate on this simulator" (a simulator row's menu).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct InstallSimulatorCert {
    pub udid: SharedString,
}

/// An emulator row's Push certificate.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct PushAndroidCert {
    pub serial: SharedString,
}

/// An emulator row's proxy switch.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetAndroidProxy {
    pub serial: SharedString,
    pub on: bool,
}

/// What a tab is doing, for its buttons' spinners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Busy {
    /// Installing on the simulators.
    SimInstall,
    /// Turning the system proxy on.
    SystemProxy,
    PushCert(String),
    EmulatorProxy(String),
}

pub struct DeviceSetup {
    caches: IntegrationCaches,
    devices: Entity<Latest<DevicesSnapshot>>,
    pub tab: DeviceTab,
    pub lan_busy: bool,
    pub busy: Option<Busy>,
    /// The last "Install to all booted"'s per-device results.
    pub sim_results: Option<Vec<SimInstallResult>>,
    /// The current tab's last failure.
    pub error: Option<String>,
    /// What the emulator tab last did ("Copied to … on …").
    pub note: Option<String>,
    /// A device row's last failure, by IP.
    pub device_errors: HashMap<String, String>,
    /// The QR code's image, made once per SVG.
    qr: Option<(String, Arc<Image>)>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct SheetOpen(Option<Entity<DeviceSetup>>);

impl Global for SheetOpen {}

/// The open sheet's content.
pub fn open_one(cx: &App) -> Option<Entity<DeviceSetup>> {
    cx.try_global::<SheetOpen>().and_then(|o| o.0.clone())
}

/// Opens the sheet (on `tab`, or the last one), or switches its tab.
pub fn open(tab: Option<DeviceTab>, window: &mut Window, cx: &mut App) {
    super::close_tool_popovers(cx);
    if let Some(sheet) = open_one(cx) {
        if let Some(tab) = tab {
            sheet.update(cx, |s, cx| s.set_tab(tab, cx));
        }
        return;
    }
    let sheet = cx.new(|cx| DeviceSetup::new(tab.unwrap_or(DeviceTab::Simulator), cx));
    cx.set_global(SheetOpen(Some(sheet.clone())));
    // What the sheet shows may have changed since it was last open.
    let caches = integration_caches(cx);
    caches.simulator.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
    caches.android.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
    caches.access.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
    caches.system_proxy.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
    window.open_sheet_at(Placement::Right, cx, move |sheet_el, _, cx| {
        let t = *cx.tokens();
        sheet_el
            .size(px(480.))
            .resizable(false)
            .p_0()
            .title(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .child(pm_widgets::icon(IconName::MonitorSmartphone, t.foreground.hsla(), 16.))
                    .child("Device setup"),
            )
            .on_close(|_, _, cx| cx.set_global(SheetOpen(None)))
            .child(sheet.clone())
    });
}

/// Closes the sheet if it is open.
pub fn close(window: &mut Window, cx: &mut App) {
    if open_one(cx).is_some() {
        cx.set_global(SheetOpen(None));
        window.close_sheet(cx);
    }
}

pub fn register(cx: &mut App) {
    cx.on_action(|a: &SetLanInterface, cx| mutations::set_lan_interface(a.name.to_string(), cx));
    cx.on_action(|a: &DecideDevice, cx| decide(a.ip.to_string(), a.allow, cx));
    cx.on_action(|a: &ForgetDevice, cx| forget(a.ip.to_string(), cx));
    cx.on_action(|a: &InstallSimulatorCert, cx| {
        let udid = a.udid.to_string();
        with_sheet(cx, move |s, cx| s.install_simulators(vec![udid], cx));
    });
    cx.on_action(|a: &PushAndroidCert, cx| {
        let serial = a.serial.to_string();
        with_sheet(cx, move |s, cx| s.push_cert(serial, cx));
    });
    cx.on_action(|a: &SetAndroidProxy, cx| {
        let (serial, on) = (a.serial.to_string(), a.on);
        with_sheet(cx, move |s, cx| s.set_emulator_proxy(serial, on, cx));
    });
}

fn with_sheet(cx: &mut App, f: impl FnOnce(&mut DeviceSetup, &mut Context<DeviceSetup>) + 'static) {
    if let Some(sheet) = open_one(cx) {
        sheet.update(cx, f);
    }
}

/// Allows or denies a device (its row, its menu or Settings › Clients); a
/// failure shows on its row.
pub fn decide(ip: String, allow: bool, cx: &mut App) {
    let nickname = bridge::devices(cx)
        .read(cx)
        .value
        .as_ref()
        .and_then(|s| s.devices.iter().find(|d| d.ip == ip))
        .and_then(|d| d.nickname.clone());
    let row = ip.clone();
    mutations::decide_device(ip, allow, nickname, move |answer, cx| row_answer(row, answer, cx), cx);
}

pub fn forget(ip: String, cx: &mut App) {
    let row = ip.clone();
    mutations::forget_device(ip, move |answer, cx| row_answer(row, answer, cx), cx);
}

fn row_answer(ip: String, answer: Result<(), mutations::Failure>, cx: &mut App) {
    with_sheet(cx, move |s, cx| {
        match answer {
            Ok(()) => s.device_errors.remove(&ip),
            Err(err) => s.device_errors.insert(ip, err.message),
        };
        cx.notify();
    });
}

/// A LAN device row's menu (here and in Settings › Clients).
pub fn device_menu(d: &DeviceClient) -> Menu {
    let ip: SharedString = d.ip.clone().into();
    Menu::new()
        .copy("Copy IP address", d.ip.clone(), "IP address")
        .when(d.nickname.is_some(), |m| {
            m.copy("Copy nickname", d.nickname.clone().unwrap_or_default(), "nickname")
        })
        .separator()
        .when(d.decision != DeviceDecision::Allowed, |m| {
            m.item(
                "Allow",
                DecideDevice {
                    ip: ip.clone(),
                    allow: true,
                },
            )
        })
        .when(d.decision != DeviceDecision::Denied, |m| {
            m.item(
                "Deny",
                DecideDevice {
                    ip: ip.clone(),
                    allow: false,
                },
            )
        })
        .item("Forget", ForgetDevice { ip })
}

/// The interface picker's menu, with Copy IP address.
pub fn interface_menu(access: &DeviceAccess) -> Menu {
    let chosen_ip = access
        .interfaces
        .iter()
        .find(|i| Some(&i.name) == access.interface.as_ref())
        .map(|i| i.ip.clone());
    let menu = access.interfaces.iter().fold(Menu::new(), |m, i| {
        m.check(
            format!("{} · {}", i.name, i.ip),
            Some(&i.name) == access.interface.as_ref(),
            SetLanInterface {
                name: i.name.clone().into(),
            },
        )
    });
    match chosen_ip {
        Some(ip) => menu.separator().copy("Copy IP address", ip, "IP address"),
        None => menu,
    }
}

/// The menus the sheet shows now, by name (`device:<ip>`,
/// `simulator:<udid>`, `emulator:<serial>`, `interface`), for the dev
/// remote and tests.
pub fn menu_for(name: &str, cx: &mut App) -> Option<Menu> {
    let caches = integration_caches(cx);
    let busy = open_one(cx).is_some_and(|s| s.read(cx).busy.is_some());
    if let Some(ip) = name.strip_prefix("device:") {
        let snapshot = bridge::devices(cx).read(cx).value.clone()?;
        return snapshot.devices.iter().find(|d| d.ip == ip).map(device_menu);
    }
    if let Some(udid) = name.strip_prefix("simulator:") {
        let sim = caches.simulator.read(cx).peek(&()).cloned()?;
        return sim
            .booted
            .iter()
            .find(|d| d.udid == udid)
            .map(|d| simulator::row_menu(d, busy));
    }
    if let Some(serial) = name.strip_prefix("emulator:") {
        let status = caches.android.read(cx).peek(&()).cloned()?;
        return status
            .devices
            .iter()
            .find(|d| d.serial == serial)
            .map(|d| emulator::row_menu(d, status.emulator_proxy.as_deref(), busy));
    }
    if name == "interface" {
        return caches.access.read(cx).peek(&()).map(interface_menu);
    }
    None
}

impl DeviceSetup {
    pub fn new(tab: DeviceTab, cx: &mut Context<Self>) -> DeviceSetup {
        let caches = integration_caches(cx);
        let devices = bridge::devices(cx);
        let subscriptions = vec![
            cx.observe(&devices, |_, _, cx| cx.notify()),
            cx.observe(&caches.access, |_, _, cx| cx.notify()),
            cx.observe(&caches.simulator, |_, _, cx| cx.notify()),
            cx.observe(&caches.android, |_, _, cx| cx.notify()),
            cx.observe(&caches.system_proxy, |_, _, cx| cx.notify()),
        ];
        // The status poll: the Simulator's and emulator's devices change
        // without telling anyone.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let polled = this.update(cx, |s, cx| match s.tab {
                    DeviceTab::Simulator => {
                        let cache = s.caches.simulator.clone();
                        cache.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
                    }
                    DeviceTab::Emulator => {
                        let cache = s.caches.android.clone();
                        cache.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
                    }
                    _ => {}
                });
                if polled.is_err() {
                    break;
                }
            }
        })
        .detach();
        DeviceSetup {
            caches,
            devices,
            tab,
            lan_busy: false,
            busy: None,
            sim_results: None,
            error: None,
            note: None,
            device_errors: HashMap::new(),
            qr: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_tab(&mut self, tab: DeviceTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.error = None;
            self.note = None;
            cx.notify();
        }
    }

    pub fn snapshot(&self, cx: &App) -> DevicesSnapshot {
        self.devices.read(cx).value.clone().unwrap_or_default()
    }

    pub fn access(&self, cx: &mut Context<Self>) -> Option<DeviceAccess> {
        self.caches.access.update(cx, |c, cx| c.get(&(), cx)).ready().cloned()
    }

    /// "Allow devices on your network".
    pub fn set_lan(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.lan_busy {
            return;
        }
        self.lan_busy = true;
        cx.notify();
        let this = cx.weak_entity();
        mutations::set_lan_access(
            on,
            move |cx| {
                let _ = this.update(cx, |s, cx| {
                    s.lan_busy = false;
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn start(&mut self, busy: Busy, cx: &mut Context<Self>) -> bool {
        if self.busy.is_some() {
            return false;
        }
        self.busy = Some(busy);
        self.error = None;
        self.note = None;
        cx.notify();
        true
    }

    fn finish(this: &gpui_kit::WeakEntity<Self>, error: Option<String>, note: Option<String>, cx: &mut App) {
        let _ = this.update(cx, |s, cx| {
            s.busy = None;
            s.error = error;
            if note.is_some() {
                s.note = note;
            }
            cx.notify();
        });
    }

    /// Installs the CA on `udids`, or every booted simulator.
    pub fn install_simulators(&mut self, udids: Vec<String>, cx: &mut Context<Self>) {
        if !self.start(Busy::SimInstall, cx) {
            return;
        }
        let this = cx.weak_entity();
        mutations::install_simulator_cert(
            udids,
            move |answer, cx| {
                let (results, error) = match answer {
                    Ok(results) => (Some(results), None),
                    Err(err) => (None, Some(err.message)),
                };
                let _ = this.update(cx, |s, _| s.sim_results = results);
                DeviceSetup::finish(&this, error, None, cx);
            },
            cx,
        );
    }

    /// The Simulator tab's "Turn on system proxy".
    pub fn system_proxy_on(&mut self, cx: &mut Context<Self>) {
        if !self.start(Busy::SystemProxy, cx) {
            return;
        }
        let this = cx.weak_entity();
        mutations::set_system_proxy(true, move |cx| DeviceSetup::finish(&this, None, None, cx), cx);
    }

    pub fn push_cert(&mut self, serial: String, cx: &mut Context<Self>) {
        if !self.start(Busy::PushCert(serial.clone()), cx) {
            return;
        }
        let this = cx.weak_entity();
        let on = serial.clone();
        mutations::push_android_cert(
            serial,
            move |answer, cx| match answer {
                Ok(path) => DeviceSetup::finish(&this, None, Some(format!("Copied to {path} on {on}.")), cx),
                Err(err) => DeviceSetup::finish(&this, Some(err.message), None, cx),
            },
            cx,
        );
    }

    pub fn set_emulator_proxy(&mut self, serial: String, on: bool, cx: &mut Context<Self>) {
        if !self.start(Busy::EmulatorProxy(serial.clone()), cx) {
            return;
        }
        let this = cx.weak_entity();
        mutations::set_android_proxy(
            serial,
            on,
            move |answer, cx| DeviceSetup::finish(&this, answer.err().map(|e| e.message), None, cx),
            cx,
        );
    }

    /// Refresh: reads the tab's devices again now.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            DeviceTab::Emulator => self.caches.android.update(cx, |c, cx| c.retry(&(), cx)),
            _ => self.caches.simulator.update(cx, |c, cx| c.retry(&(), cx)),
        }
    }

    fn qr_image(&mut self, svg: &str) -> Arc<Image> {
        if let Some((s, image)) = &self.qr
            && s == svg
        {
            return image.clone();
        }
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec()));
        self.qr = Some((svg.to_owned(), image.clone()));
        image
    }

    fn access_bar(&mut self, access: Option<&DeviceAccess>, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let enabled = access.is_some_and(|a| a.enabled);
        let lan_busy = self.lan_busy;
        let switch = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(ui::switch(
                "lan-access",
                Switch::new("lan-access-switch")
                    .checked(enabled)
                    .disabled(access.is_none() || lan_busy)
                    .accessibility_label("Allow devices on your network")
                    .on_click(cx.listener(|s, on: &bool, _, cx| s.set_lan(*on, cx))),
            ))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Allow devices on your network"),
            )
            .when(lan_busy, |d| {
                d.child(pm_widgets::icon(IconName::LoaderCircle, t.muted_foreground.hsla(), 14.))
            });
        let mut bar = div()
            .id("device-access")
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .border_b_1()
            .border_color(t.border.hsla())
            .bg(t.muted.hsla())
            .child(switch)
            .child(div().text_color(t.muted_foreground.hsla()).child(if enabled {
                "Listening on every interface. A device's first connection asks you to allow it."
            } else {
                "Proxymate only listens on this Mac (127.0.0.1)."
            }));
        if !enabled {
            bar = bar.child(phones::no_lan_hint(t));
        }
        if let Some(access) = access.filter(|a| a.enabled) {
            let chosen = access
                .interfaces
                .iter()
                .find(|i| Some(&i.name) == access.interface.as_ref());
            let picker: AnyElement = if access.interfaces.is_empty() {
                div()
                    .text_color(t.s_4xx.hsla())
                    .child("No network interface with an IPv4 address")
                    .into_any_element()
            } else {
                let label = chosen.map_or_else(|| "Choose…".to_owned(), |i| format!("{} · {}", i.name, i.ip));
                let menu_access = access.clone();
                let right_access = access.clone();
                div()
                    .id("lan-interface-picker")
                    .child(
                        Button::new("lan-interface")
                            .outline()
                            .small()
                            .w_full()
                            .accessibility_label(SharedString::from(format!("Interface: {label}")))
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(label)
                                    .child(pm_widgets::icon(IconName::ChevronDown, t.muted_foreground.hsla(), 14.)),
                            )
                            .on_click(move |_, window, cx| {
                                interface_menu(&menu_access).show(window.mouse_position(), window, cx);
                            }),
                    )
                    .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                        interface_menu(&right_access).show(e.position, window, cx);
                    })
                    .into_any_element()
            };
            bar = bar.child(ui::field("Interface", 64., picker, t));
            if let Some(proxy) = &access.proxy {
                bar = bar.child(ui::copy_row(
                    "Proxy",
                    64.,
                    ui::copy_field(
                        "copy-lan-proxy",
                        proxy.clone(),
                        "Copy proxy address",
                        "proxy address",
                        false,
                        vec![],
                        t,
                    ),
                    t,
                ));
            }
        }
        if let Some(err) = access.and_then(|a| a.error.clone()) {
            bar = bar.child(ui::error(err, t));
        }
        bar.into_any_element()
    }

    fn known_devices(&self, devices: &[DeviceClient], t: &Tokens) -> Option<AnyElement> {
        if devices.is_empty() {
            return None;
        }
        let rows = devices.iter().map(|d| {
            let colour = match d.decision {
                DeviceDecision::Allowed => t.s_2xx,
                DeviceDecision::Pending => t.s_4xx,
                DeviceDecision::Denied => t.s_5xx,
            };
            let ip: SharedString = d.ip.clone().into();
            let error = self.device_errors.get(&d.ip).cloned();
            let menu_device = d.clone();
            let icon_button = |id: &str, icon: IconName, label: String, action: Box<dyn gpui_kit::Action>| {
                Button::new(ElementId::Name(format!("{id}-{}", d.ip).into()))
                    .ghost()
                    .xsmall()
                    .accessibility_label(SharedString::from(label.clone()))
                    .tooltip(SharedString::from(label))
                    .child(pm_widgets::icon(icon, t.muted_foreground.hsla(), 13.))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            };
            div()
                .id(ElementId::Name(format!("device-row-{}", d.ip).into()))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.border.hsla())
                .role(Role::ListItem)
                .aria_label(SharedString::from(format!("{}: {}", d.label, view::device_line(d))))
                .child(ui::dot(colour))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().truncate().font_weight(FontWeight::MEDIUM).child(d.label.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_color(t.muted_foreground.hsla())
                                .child(view::device_line(d))
                                .when_some(error, |e, err| {
                                    e.child(div().text_color(t.s_failed.hsla()).child(format!(" · {err}")))
                                }),
                        ),
                )
                .when(d.decision != DeviceDecision::Allowed, |r| {
                    r.child(icon_button(
                        "device-allow",
                        IconName::Check,
                        format!("Allow {}", d.ip),
                        Box::new(DecideDevice {
                            ip: ip.clone(),
                            allow: true,
                        }),
                    ))
                })
                .when(d.decision != DeviceDecision::Denied, |r| {
                    r.child(icon_button(
                        "device-deny",
                        IconName::X,
                        format!("Deny {}", d.ip),
                        Box::new(DecideDevice {
                            ip: ip.clone(),
                            allow: false,
                        }),
                    ))
                })
                .child(icon_button(
                    "device-forget",
                    IconName::Trash,
                    format!("Forget {}", d.ip),
                    Box::new(ForgetDevice { ip: ip.clone() }),
                ))
                .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    device_menu(&menu_device).show(e.position, window, cx);
                })
        });
        Some(
            div()
                .id("known-devices")
                .flex()
                .flex_col()
                .gap(px(8.))
                .pt(px(12.))
                .border_t_1()
                .border_color(t.border.hsla())
                .child(
                    div()
                        .id("known-devices-title")
                        .role(Role::Heading)
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Devices on your network"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(t.border.hsla())
                        .id("known-devices-list")
                        .role(Role::List)
                        .children(rows),
                )
                .into_any_element(),
        )
    }
}

impl Render for DeviceSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let access = self.access(cx);
        let snapshot = self.snapshot(cx);
        let bar = self.access_bar(access.as_ref(), &t, cx);
        let selected = DeviceTab::ALL.iter().position(|x| *x == self.tab).unwrap_or(0);
        let tabs = TabBar::new("device-tabs")
            .small()
            .segmented()
            .w_full()
            .selected_index(selected)
            .children(DeviceTab::ALL.iter().map(|x| Tab::new().label(x.label()).flex_1()))
            .on_click(cx.listener(|s, ix: &usize, _, cx| {
                if let Some(tab) = DeviceTab::ALL.get(*ix) {
                    s.set_tab(*tab, cx);
                }
            }));
        let body: AnyElement = match self.tab {
            DeviceTab::Simulator => self.simulator_tab(&snapshot, &t, cx),
            DeviceTab::Iphone => {
                let qr = phones::qr_parts(access.as_ref()).map(|svg| self.qr_image(&svg));
                phones::iphone_tab(access.as_ref(), &snapshot, qr, &t)
            }
            DeviceTab::Android => {
                let qr = phones::qr_parts(access.as_ref()).map(|svg| self.qr_image(&svg));
                phones::android_tab(access.as_ref(), &snapshot, qr, &t)
            }
            DeviceTab::Emulator => self.emulator_tab(access.as_ref(), &snapshot, &t, cx),
        };
        div()
            .id("device-setup")
            .flex()
            .flex_col()
            .text_size(px(12.))
            .role(Role::Region)
            .aria_label("Device setup")
            .child(
                div()
                    .px(px(12.))
                    .pb(px(8.))
                    .text_color(t.muted_foreground.hsla())
                    .child("Send a phone, simulator or emulator through Proxymate and trust its certificate there."),
            )
            .child(bar)
            .child(div().px(px(12.)).pt(px(12.)).child(tabs))
            .child(
                div()
                    .id(ElementId::Name(format!("device-tab-{}", self.tab.id()).into()))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(12.))
                    .child(body)
                    .children(self.known_devices(&snapshot.devices, &t))
                    .test_support(),
            )
    }
}

/// A QR code image (an SVG the engine drew), on paper.
pub(super) fn qr_element(image: Arc<Image>, alt: String, t: &Tokens) -> AnyElement {
    div()
        .flex_none()
        .size(px(132.))
        .rounded(px(4.))
        .bg(t.paper.hsla())
        .id("qr-code")
        .role(Role::Image)
        .aria_label(SharedString::from(alt))
        .child(img(image).size(px(132.)))
        .into_any_element()
}
