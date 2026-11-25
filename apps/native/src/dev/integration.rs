//! The dev remote's macOS integration verbs (G13), as in the Tauri
//! remote (`deviceTab`, `intercept`, `neverRecord`; `click` is shared):
//!
//! - `{"open": "cert-wizard" | "device-setup[:<tab>]" | "intercept" |
//!   "system-proxy" | "certificate"}` opens the wizard, the sheet (on a
//!   tab) or a toolbar popover; `{"close": true}` closes them (not the
//!   LAN prompt, which follows the devices).
//! - `{"deviceTab": "simulator" | "iphone" | "android" | "emulator"}`
//!   opens the sheet on that tab (or switches it).
//! - `{"intercept": "chrome"}` **really launches** that app through the
//!   Engine (the user's OK only), then answers with the running launches.
//! - `{"neverRecord": "example.com"}` stops recording a host.
//! - `{"injectDevices": <DevicesSnapshot>}` shows LAN clients the machine
//!   can't produce (a waiting device, for the "Allow?" prompt).
//! - `{"integration": true}` answers with the state: trust, the CA, the
//!   system proxy, LAN access, devices, the prompt, the sheet, the
//!   popovers, intercepts and MCP.
//!
//! `contextMenu` takes `integration:<name>` (`ca`, `services`,
//! `allow-title`, `device:<ip>`, `simulator:<udid>`, `emulator:<serial>`,
//! `interface`).

use std::time::Duration;

use gpui_kit::{App, AsyncApp};
use pm_model::DevicesSnapshot;
use serde_json::{Value, json};

use crate::bridge::{self, keys, mutations};
use crate::features::integration::{
    self as integration, cert::CertWizard, devices, devices::DeviceTab, prompt::Prompts,
};

const VERBS: [&str; 7] = [
    "open",
    "close",
    "deviceTab",
    "intercept",
    "neverRecord",
    "injectDevices",
    "integration",
];

/// `None` when `command` has none of these verbs.
pub async fn command(command: &Value, cx: &mut AsyncApp) -> Option<Value> {
    if !VERBS.iter().any(|v| command.get(v).is_some()) {
        return None;
    }
    let result: Result<(), String> = cx.update(|cx| {
        if let Some(name) = command.get("open").and_then(Value::as_str)
            && !integration::open_named(name, cx)
        {
            return Err(format!("nothing called {name:?} to open"));
        }
        if command.get("close").and_then(Value::as_bool) == Some(true) {
            close_all(cx);
        }
        if let Some(tab) = command.get("deviceTab").and_then(Value::as_str) {
            let tab = DeviceTab::parse(tab).ok_or_else(|| format!("no device tab {tab:?}"))?;
            integration::in_main_window(cx, move |window, cx| devices::open(Some(tab), window, cx));
        }
        if let Some(id) = command.get("intercept").and_then(Value::as_str) {
            let entry = pm_view::integration::intercept_entry(id).ok_or_else(|| format!("no target {id:?}"))?;
            mutations::intercept(entry.target, cx);
        }
        if let Some(host) = command.get("neverRecord").and_then(Value::as_str) {
            mutations::never_record(host.to_owned(), cx);
        }
        if let Some(v) = command.get("injectDevices") {
            let snapshot: DevicesSnapshot =
                serde_json::from_value(v.clone()).map_err(|e| format!("injectDevices: {e}"))?;
            let devices = bridge::devices(cx);
            devices.update(cx, |d, cx| {
                d.value = Some(snapshot);
                cx.notify();
            });
        }
        Ok(())
    });
    if let Err(err) = result {
        return Some(json!({ "error": err }));
    }
    // Let the dialog, the sheet and a write's answer land.
    let wait = if command.get("intercept").is_some() { 1500 } else { 300 };
    cx.background_executor().timer(Duration::from_millis(wait)).await;
    Some(cx.update(state))
}

fn close_all(cx: &mut App) {
    integration::close_tool_popovers(cx);
    integration::in_main_window(cx, |window, cx| {
        devices::close(window, cx);
        integration::cert::close_wizard(window, cx);
    });
}

/// What the integration surfaces show now.
fn state(cx: &mut App) -> Value {
    let caches = keys::integration_caches(cx);
    let trust = caches.trust.read(cx).peek(&()).cloned();
    let ca = caches.ca.read(cx).peek(&()).cloned();
    let sysproxy = caches.system_proxy.read(cx).peek(&()).cloned();
    let access = caches.access.read(cx).peek(&()).cloned();
    let simulator = caches.simulator.read(cx).peek(&()).cloned();
    let android = caches.android.read(cx).peek(&()).cloned();
    let intercepts = caches.intercepts.read(cx).peek(&()).cloned();
    let devices = bridge::devices(cx).read(cx).value.clone();
    let mcp = bridge::mcp(cx).read(cx).value.clone();
    let sheet = devices::open_one(cx).map(|s| {
        let s = s.read(cx);
        json!({
            "tab": s.tab.id(),
            "busy": s.busy.as_ref().map(|b| format!("{b:?}")),
            "error": s.error,
            "note": s.note,
            "results": s.sim_results,
        })
    });
    let wizard = CertWizard::open_one(cx).map(|w| {
        let w = w.read(cx);
        json!({ "busy": w.busy.map(|b| format!("{b:?}")), "error": w.error })
    });
    let popover = |open: Option<bool>| open.unwrap_or(false);
    let open = json!({
        "intercept": popover(integration::intercept_tool(cx).map(|t| t.read(cx).open)),
        "systemProxy": popover(integration::system_proxy_tool(cx).map(|t| t.read(cx).open)),
        "certificate": popover(integration::certificate_tool(cx).map(|t| t.read(cx).open)),
    });
    let waiting = devices.as_ref().map_or(0, |d| {
        cx.try_global::<Prompts>()
            .map_or(0, |p| pm_view::devices::waiting_count(&d.devices, &p.dismissed))
    });
    json!({
        "trust": trust,
        "ca": ca,
        "systemProxy": sysproxy,
        "access": access.map(|a| json!({
            "enabled": a.enabled, "interface": a.interface, "proxy": a.proxy, "setupUrl": a.setup_url,
            "error": a.error, "qr": a.qr_svg.is_some(),
        })),
        "simulator": simulator,
        "android": android,
        "devices": devices,
        "prompt": Prompts::showing(cx),
        "waiting": waiting,
        "intercepts": intercepts,
        "mcp": mcp.map(|m| json!({
            "url": m.url, "error": m.error, "clients": m.clients.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
            "tip": pm_view::integration::mcp_tip(Some(&m)),
        })),
        "wizard": wizard,
        "sheet": sheet,
        "popovers": open,
        "toast": crate::features::clipboard::Toasts::last(cx)
            .map(|t| json!({"title": t.title, "description": t.description})),
    })
}
