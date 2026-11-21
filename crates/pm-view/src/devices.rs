//! Device setup's helpers (`lib/devices.ts` with `devices.test.ts`, G13):
//! how far a device has got, the allowed devices that showed up, the Wi-Fi
//! proxy fields, the Android snippets and the emulator's adb commands; plus
//! the texts of the sheet's live checks, the LAN prompt's countdown and the
//! toolbar's "N devices waiting".

use std::collections::HashSet;

use pm_model::{DeviceClient, DeviceDecision, LocalActivity, SimulatorStatus};

/// How long a LAN client's connections wait for an answer (`HOLD` in
/// `pm-core/src/devices.rs`), for the countdown's bar.
pub const HOLD_SECS: f64 = 30.0;

/// How far a device has got, for the wizard's live checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Connected,
    Untrusted,
    Decrypting,
}

/// From a device's flow counts: has it sent anything, and is its HTTPS
/// decrypted?
pub fn progress(flows: u32, https_decrypted: u32, https_refused: u32) -> Progress {
    if https_decrypted > 0 {
        Progress::Decrypting
    } else if https_refused > 0 {
        Progress::Untrusted
    } else if flows > 0 {
        Progress::Connected
    } else {
        Progress::Waiting
    }
}

pub fn activity_progress(a: &LocalActivity) -> Progress {
    progress(a.flows, a.https_decrypted, a.https_refused)
}

pub fn device_progress(d: &DeviceClient) -> Progress {
    progress(d.flows, d.https_decrypted, d.https_refused)
}

/// The state of one line of a live checklist (its icon and colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Ok,
    Warn,
    Pending,
    Idle,
}

/// One line of a checklist: its state, title and detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub state: CheckState,
    pub label: String,
    pub detail: Option<String>,
}

impl Check {
    pub fn new(state: CheckState, label: impl Into<String>, detail: Option<String>) -> Check {
        Check {
            state,
            label: label.into(),
            detail,
        }
    }
}

/// `ProgressCheck`: the live check for a device's first flows.
pub fn progress_check(progress: Progress, who: &str, flows: u32) -> Check {
    match progress {
        Progress::Decrypting => Check::new(
            CheckState::Ok,
            format!("Decrypting HTTPS from {who}"),
            Some(format!("{flows} flows so far.")),
        ),
        Progress::Untrusted => Check::new(
            CheckState::Warn,
            format!("{who} refused Proxymate's certificate"),
            Some("Traffic arrives, but HTTPS fails: install and trust the certificate, then reopen the app.".into()),
        ),
        Progress::Connected => Check::new(
            CheckState::Pending,
            format!("Waiting for the first HTTPS flow from {who}…"),
            Some(format!(
                "{flows} plain flow{} seen. Open an https:// site.",
                if flows == 1 { "" } else { "s" }
            )),
        ),
        Progress::Waiting => Check::new(
            CheckState::Pending,
            format!("Waiting for the first flow from {who}…"),
            None,
        ),
    }
}

/// Allowed LAN devices that have sent traffic or fetched the setup page,
/// most recent first.
pub fn active_devices(devices: &[DeviceClient]) -> Vec<&DeviceClient> {
    let mut out: Vec<&DeviceClient> = devices
        .iter()
        .filter(|d| d.decision == DeviceDecision::Allowed && (d.flows > 0 || d.onboarding))
        .collect();
    out.sort_by(|a, b| {
        let (a, b) = (a.last_seen_ms.unwrap_or(0.0), b.last_seen_ms.unwrap_or(0.0));
        b.total_cmp(&a)
    });
    out
}

/// `host:port` split for the Wi-Fi proxy fields.
pub fn split_proxy(proxy: Option<&str>) -> Option<(String, String)> {
    let proxy = proxy?;
    let at = proxy.rfind(':')?;
    if at == 0 {
        return None;
    }
    Some((proxy[..at].to_owned(), proxy[at + 1..].to_owned()))
}

/// `res/xml/network_security_config.xml` that trusts user CAs in debug
/// builds only.
pub const NETWORK_SECURITY_CONFIG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <debug-overrides>
        <trust-anchors>
            <certificates src="user" />
        </trust-anchors>
    </debug-overrides>
</network-security-config>
"#;

pub const MANIFEST_ATTRIBUTE: &str = r#"<application
    android:networkSecurityConfig="@xml/network_security_config"
    ... >"#;

/// The adb commands the emulator tab runs, for people who prefer a
/// terminal: (set, clear).
pub fn emulator_commands(port: Option<u16>) -> (String, String) {
    (
        format!(
            "adb shell settings put global http_proxy 10.0.2.2:{}",
            port.unwrap_or(9090)
        ),
        "adb shell settings put global http_proxy :0".into(),
    )
}

/// The first LAN client nobody has answered for yet, whose prompt wasn't
/// closed: the one the "Allow?" prompt asks about.
pub fn prompt_for<'a>(devices: &'a [DeviceClient], dismissed: &HashSet<String>) -> Option<&'a DeviceClient> {
    devices
        .iter()
        .find(|d| d.decision == DeviceDecision::Pending && !dismissed.contains(&d.ip))
}

/// Pending devices whose prompt was closed: the toolbar's "N devices
/// waiting".
pub fn waiting_count(devices: &[DeviceClient], dismissed: &HashSet<String>) -> usize {
    devices
        .iter()
        .filter(|d| d.decision == DeviceDecision::Pending && dismissed.contains(&d.ip))
        .count()
}

pub fn waiting_label(n: usize) -> String {
    format!("{n} device{} waiting", if n == 1 { "" } else { "s" })
}

/// The prompt's countdown: "2 connections waiting · refused in 12 s", and
/// the bar's fill (0–1), from the newest held connection's deadline.
pub fn countdown(waiting: u32, until_ms: Option<f64>, now_ms: f64) -> (String, Option<f32>) {
    let mut text = format!("{waiting} connection{} waiting", if waiting == 1 { "" } else { "s" });
    let Some(until) = until_ms else {
        return (text, None);
    };
    let left = ((until - now_ms) / 1000.0).ceil().max(0.0);
    if left > 0.0 {
        text.push_str(&format!(" · refused in {left} s"));
    } else {
        text.push_str(" · being refused");
    }
    (text, Some((left / HOLD_SECS).clamp(0.0, 1.0) as f32))
}

/// A LAN device row's second line in the sheet.
pub fn device_line(d: &DeviceClient) -> String {
    match d.decision {
        DeviceDecision::Pending if d.waiting > 0 => format!("Waiting for your answer · {} held", d.waiting),
        DeviceDecision::Pending => "Waiting for your answer".into(),
        DeviceDecision::Denied => "Denied".into(),
        DeviceDecision::Allowed => format!("{} flows · {} HTTPS decrypted", d.flows, d.https_decrypted),
    }
}

/// The Simulator tab's first two checks: simctl, then the booted devices.
pub fn simulator_checks(sim: Option<&SimulatorStatus>) -> [Check; 2] {
    let Some(sim) = sim else {
        return [
            Check::new(CheckState::Pending, "Xcode simulator tools", None),
            Check::new(CheckState::Pending, "No simulator is booted", None),
        ];
    };
    let tools = if sim.available {
        Check::new(CheckState::Ok, "Xcode simulator tools", None)
    } else {
        Check::new(CheckState::Warn, "Xcode's simctl isn't available", sim.detail.clone())
    };
    let n = sim.booted.len();
    let booted = if n > 0 {
        Check::new(
            CheckState::Ok,
            format!("{n} booted simulator{}", if n == 1 { "" } else { "s" }),
            sim.available.then(|| sim.detail.clone()).flatten(),
        )
    } else {
        Check::new(
            CheckState::Warn,
            "No simulator is booted",
            sim.available
                .then(|| "Start one from Xcode (Window › Devices and Simulators) or the Simulator app.".into()),
        )
    };
    [tools, booted]
}

/// Whether "Install to all booted" says "Install again".
pub fn all_installed(sim: Option<&SimulatorStatus>) -> bool {
    sim.is_some_and(|s| !s.booted.is_empty() && s.booted.iter().all(|d| d.cert_installed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(ip: &str, f: impl FnOnce(&mut DeviceClient)) -> DeviceClient {
        let mut d = DeviceClient {
            ip: ip.into(),
            nickname: None,
            decision: DeviceDecision::Allowed,
            label: ip.into(),
            flows: 0,
            https_decrypted: 0,
            https_refused: 0,
            onboarding: false,
            last_seen_ms: None,
            waiting: 0,
            held_until_ms: None,
        };
        f(&mut d);
        d
    }

    #[test]
    fn reports_progress_from_flow_counts() {
        assert_eq!(progress(0, 0, 0), Progress::Waiting);
        assert_eq!(progress(3, 0, 0), Progress::Connected);
        assert_eq!(progress(3, 0, 2), Progress::Untrusted);
        assert_eq!(progress(3, 1, 2), Progress::Decrypting);
    }

    #[test]
    fn lists_allowed_devices_that_showed_up_newest_first() {
        let devices = [
            device("10.0.0.2", |d| {
                d.flows = 1;
                d.last_seen_ms = Some(10.0);
            }),
            device("10.0.0.3", |d| {
                d.decision = DeviceDecision::Pending;
                d.flows = 4;
            }),
            device("10.0.0.4", |d| {
                d.onboarding = true;
                d.last_seen_ms = Some(20.0);
            }),
            device("10.0.0.5", |_| {}),
        ];
        let ips: Vec<&str> = active_devices(&devices).iter().map(|d| d.ip.as_str()).collect();
        assert_eq!(ips, ["10.0.0.4", "10.0.0.2"]);
    }

    #[test]
    fn splits_the_proxy_address_for_the_wifi_fields() {
        assert_eq!(
            split_proxy(Some("192.168.1.20:9090")),
            Some(("192.168.1.20".into(), "9090".into()))
        );
        assert_eq!(split_proxy(None), None);
        assert_eq!(split_proxy(Some("nope")), None);
    }

    #[test]
    fn builds_emulator_commands() {
        assert_eq!(
            emulator_commands(Some(9091)).0,
            "adb shell settings put global http_proxy 10.0.2.2:9091"
        );
        assert_eq!(emulator_commands(None).1, "adb shell settings put global http_proxy :0");
    }

    #[test]
    fn the_prompt_asks_about_the_first_pending_device_not_dismissed() {
        let devices = [
            device("10.0.0.2", |d| d.decision = DeviceDecision::Pending),
            device("10.0.0.3", |d| d.decision = DeviceDecision::Pending),
            device("10.0.0.4", |_| {}),
        ];
        let mut dismissed = HashSet::new();
        assert_eq!(
            prompt_for(&devices, &dismissed).map(|d| d.ip.as_str()),
            Some("10.0.0.2")
        );
        assert_eq!(waiting_count(&devices, &dismissed), 0);
        dismissed.insert("10.0.0.2".to_owned());
        assert_eq!(
            prompt_for(&devices, &dismissed).map(|d| d.ip.as_str()),
            Some("10.0.0.3")
        );
        assert_eq!(waiting_count(&devices, &dismissed), 1);
        assert_eq!(waiting_label(1), "1 device waiting");
        assert_eq!(waiting_label(2), "2 devices waiting");
    }

    #[test]
    fn counts_down_to_the_refusal() {
        assert_eq!(countdown(1, None, 0.0), ("1 connection waiting".into(), None));
        let (text, fill) = countdown(2, Some(12_000.0), 0.5);
        assert_eq!(text, "2 connections waiting · refused in 12 s");
        assert!((fill.unwrap() - 0.4).abs() < 1e-6);
        assert_eq!(
            countdown(2, Some(1_000.0), 5_000.0).0,
            "2 connections waiting · being refused"
        );
    }

    #[test]
    fn live_checks_say_where_a_device_is() {
        let c = progress_check(Progress::Connected, "the Simulator", 1);
        assert_eq!(c.label, "Waiting for the first HTTPS flow from the Simulator…");
        assert_eq!(c.detail.as_deref(), Some("1 plain flow seen. Open an https:// site."));
        assert_eq!(
            progress_check(Progress::Decrypting, "Pixel", 4).label,
            "Decrypting HTTPS from Pixel"
        );
        assert_eq!(progress_check(Progress::Untrusted, "Pixel", 4).state, CheckState::Warn);
        let pending = device("10.0.0.9", |d| {
            d.decision = DeviceDecision::Pending;
            d.waiting = 2;
        });
        assert_eq!(device_line(&pending), "Waiting for your answer · 2 held");
    }

    #[test]
    fn simulator_checks_follow_simctl() {
        let missing = SimulatorStatus {
            available: false,
            detail: Some("xcrun: error".into()),
            booted: vec![],
        };
        let [tools, booted] = simulator_checks(Some(&missing));
        assert_eq!(tools.label, "Xcode's simctl isn't available");
        assert_eq!(tools.detail.as_deref(), Some("xcrun: error"));
        assert_eq!(booted.label, "No simulator is booted");
        assert_eq!(booted.detail, None);
        let one = SimulatorStatus {
            available: true,
            detail: None,
            booted: vec![pm_model::SimDevice {
                udid: "A".into(),
                name: "iPhone 17".into(),
                runtime: "iOS 27.0".into(),
                cert_installed: true,
            }],
        };
        assert_eq!(simulator_checks(Some(&one))[1].label, "1 booted simulator");
        assert!(all_installed(Some(&one)));
        assert!(!all_installed(None));
    }
}
