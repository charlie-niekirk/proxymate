//! macOS integration (G13): the certificate wizard and Certificate popover
//! ([`cert`]), the system proxy popover ([`sysproxy`]), the Device setup
//! sheet ([`devices`]), the LAN "Allow?" prompt and "N devices waiting"
//! ([`prompt`]), the Intercept popover ([`intercept`]) and the MCP
//! indicator ([`mcp`]).
//!
//! Every OS call goes through the Engine (and so `pm-platform`) as a
//! `bridge::mutations` write, which a test's `mutations::Journal` stops
//! before it runs. Views render from the `integration_caches` and the
//! `devices`/`mcp` pushes, so tests inject what the machine can't produce
//! (a waiting LAN device, a booted simulator).

use gpui_kit::{App, Entity, Global, WeakEntity, Window};
use pm_view::integration::INTERCEPT_TARGETS;

use crate::bridge::keys::integration_caches;
use crate::bridge::mutations;
use crate::features::chrome::{OpenCertWizard, OpenDeviceSetup};
use crate::features::commands::{self, always};
use crate::shell::toolbar::{self, order};

pub mod cert;
pub mod devices;
pub mod intercept;
pub mod mcp;
pub mod prompt;
pub mod sysproxy;
mod ui;

/// The toolbar tools with a popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Intercept,
    SystemProxy,
    Certificate,
}

/// The main window's tools, for closing their popovers and the dev remote.
#[derive(Default)]
struct Tools {
    intercept: Option<WeakEntity<intercept::InterceptTool>>,
    system_proxy: Option<WeakEntity<sysproxy::SystemProxyTool>>,
    certificate: Option<WeakEntity<cert::CertificateTool>>,
}

impl Global for Tools {}

fn track_tool<T: 'static>(tool: Tool, entity: &Entity<T>, cx: &mut App) {
    let weak = entity.downgrade();
    let tools = cx.default_global::<Tools>();
    let any: &dyn std::any::Any = &weak;
    match tool {
        Tool::Intercept => tools.intercept = any.downcast_ref().cloned(),
        Tool::SystemProxy => tools.system_proxy = any.downcast_ref().cloned(),
        Tool::Certificate => tools.certificate = any.downcast_ref().cloned(),
    }
}

pub fn intercept_tool(cx: &App) -> Option<Entity<intercept::InterceptTool>> {
    cx.try_global::<Tools>()?.intercept.as_ref()?.upgrade()
}

pub fn system_proxy_tool(cx: &App) -> Option<Entity<sysproxy::SystemProxyTool>> {
    cx.try_global::<Tools>()?.system_proxy.as_ref()?.upgrade()
}

pub fn certificate_tool(cx: &App) -> Option<Entity<cert::CertificateTool>> {
    cx.try_global::<Tools>()?.certificate.as_ref()?.upgrade()
}

/// Opens one tool's popover (and closes the others).
pub fn open_tool(tool: Tool, cx: &mut App) -> bool {
    close_tool_popovers(cx);
    match tool {
        Tool::Intercept => intercept_tool(cx).map(|t| t.update(cx, |t, cx| t.set_open(true, cx))),
        Tool::SystemProxy => system_proxy_tool(cx).map(|t| t.update(cx, |t, cx| t.set_open(true, cx))),
        Tool::Certificate => certificate_tool(cx).map(|t| t.update(cx, |t, cx| t.set_open(true, cx))),
    }
    .is_some()
}

/// Closes every tool's popover (a button in one opened a dialog).
pub fn close_tool_popovers(cx: &mut App) {
    if let Some(t) = intercept_tool(cx) {
        t.update(cx, |t, cx| t.set_open(false, cx));
    }
    if let Some(t) = system_proxy_tool(cx) {
        t.update(cx, |t, cx| t.set_open(false, cx));
    }
    if let Some(t) = certificate_tool(cx) {
        t.update(cx, |t, cx| t.set_open(false, cx));
    }
}

/// Runs `f` in the main window, after the current dispatch (an app-level
/// action handler runs while a window is still dispatching).
pub fn in_main_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(handle) = crate::shell::parts(cx).map(|p| p.window) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| f(window, cx));
    });
}

pub fn register(cx: &mut App) {
    cx.on_action(|_: &OpenCertWizard, cx| in_main_window(cx, cert::open_wizard));
    cx.on_action(|_: &OpenDeviceSetup, cx| in_main_window(cx, |window, cx| devices::open(None, window, cx)));
    commands::add(
        "certificate",
        always,
        |window, cx| window.dispatch_action(Box::new(OpenCertWizard), cx),
        cx,
    );
    commands::add(
        "devices",
        always,
        |window, cx| window.dispatch_action(Box::new(OpenDeviceSetup), cx),
        cx,
    );
    for entry in INTERCEPT_TARGETS {
        let target = entry.target;
        commands::add(
            &format!("intercept-{}", entry.id),
            move |cx| installed(target, cx),
            move |_, cx| mutations::intercept(target, cx),
            cx,
        );
    }
    devices::register(cx);
    prompt::register(cx);
    // Left of Intercept: the Tauri toolbar centres it beside the listening
    // pill, a slot this toolbar doesn't have.
    toolbar::add_tool(order::INTERCEPT - 5, prompt::waiting_devices, cx);
    toolbar::add_tool(order::INTERCEPT, intercept::intercept_tool, cx);
    toolbar::add_tool(order::TRUST, cert::trust_warning, cx);
    toolbar::add_tool(order::SYSTEM_PROXY, sysproxy::system_proxy_tool, cx);
    toolbar::add_tool(order::CERTIFICATE, cert::certificate_tool, cx);
    toolbar::add_tool(order::MCP, mcp::mcp_indicator, cx);
}

/// Whether `target`'s app is installed (unknown counts as installed, as in
/// the Tauri palette): the palette hides the rest.
fn installed(target: pm_model::InterceptTarget, cx: &App) -> bool {
    cx.try_global::<crate::bridge::keys::IntegrationCaches>()
        .and_then(|c| c.intercept_apps.read(cx).peek(&()).cloned())
        .is_none_or(|apps| apps.iter().find(|a| a.target == target).is_none_or(|a| a.installed))
}

/// The integration surfaces' menus by name, as shown now: `ca` (the
/// Certificate popover's details), `services` (the system proxy's),
/// `allow-title` (the open LAN prompt's title), and the Device setup
/// sheet's (`device:<ip>`, `simulator:<udid>`, `emulator:<serial>`,
/// `interface`).
pub fn menu_for(name: &str, cx: &mut App) -> Option<crate::features::context_menu::Menu> {
    match name {
        "ca" => cert::current_ca_menu(cx),
        "services" => {
            let status = integration_caches(cx).system_proxy.read(cx).peek(&()).cloned()?;
            (!status.services.is_empty()).then(|| sysproxy::services_menu(&status))
        }
        "allow-title" => prompt::Prompts::showing(cx).map(|ip| prompt::title_menu(&ip)),
        _ => devices::menu_for(name, cx),
    }
}

/// `PROXYMATE_DEV_OPEN=cert-wizard` or `device-setup:<tab>` (debug builds
/// or `dev-harness`).
pub fn open_from_env(cx: &mut App) {
    let Some(open) = crate::dev_env::value("PROXYMATE_DEV_OPEN") else {
        return;
    };
    open_named(&open, cx);
}

/// What `PROXYMATE_DEV_OPEN` and the dev remote's `open` name: the wizard,
/// the sheet on a tab, or a tool's popover. False for anything else.
pub fn open_named(name: &str, cx: &mut App) -> bool {
    if name == "cert-wizard" {
        in_main_window(cx, cert::open_wizard);
        return true;
    }
    if let Some(rest) = name.strip_prefix("device-setup") {
        let tab = rest.strip_prefix(':').and_then(devices::DeviceTab::parse);
        in_main_window(cx, move |window, cx| devices::open(tab, window, cx));
        return true;
    }
    let tool = match name {
        "intercept" => Tool::Intercept,
        "system-proxy" => Tool::SystemProxy,
        "certificate" => Tool::Certificate,
        _ => return false,
    };
    // Reads what the popover shows first, then opens it.
    let _ = integration_caches(cx);
    open_tool(tool, cx)
}
