//! macOS integration (G13) driven like a user would, from injected
//! snapshots and with a `mutations::Journal` set, so nothing here can
//! change the machine (no keychain, system proxy, simctl, adb or launch):
//! each write lands in the journal instead. The certificate wizard and
//! popover, the system proxy popover, the Device setup sheet and its tabs,
//! the LAN "Allow?" prompt with "N devices waiting", the Intercept popover
//! and palette commands, and the MCP indicator.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Entity, TestAppContext, VisualTestContext};
use pm_model::{
    AndroidDevice, AndroidStatus, CaInfo, DeviceAccess, DeviceClient, DeviceDecision, DevicesSnapshot, InterceptApp,
    InterceptSession, InterceptTarget, KeychainTrust, LanInterface, McpClient, McpStatus, ProxyStatus, SimDevice,
    SimulatorStatus, StorageStatus, SystemProxyStatus, TrustStatus,
};
use proxymate_native::bridge::keys::integration_caches;
use proxymate_native::bridge::mutations::Journal;
use proxymate_native::bridge::stream::Latest;
use proxymate_native::bridge::{self};
use proxymate_native::features::chrome::{OpenCertWizard, OpenDeviceSetup};
use proxymate_native::features::clipboard::Toasts;
use proxymate_native::features::commands;
use proxymate_native::features::context_menu::Menu;
use proxymate_native::features::flow_table::FlowTable;
use proxymate_native::features::integration::cert::{self, CertWizard};
use proxymate_native::features::integration::devices::{self, DeviceTab};
use proxymate_native::features::integration::prompt::{self, Prompts};
use proxymate_native::features::integration::{self as integration, Tool};
use proxymate_native::shell::{self, Parts};
use proxymate_native::workspace::Workspace;

fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        gpui_kit::init(cx);
        proxymate_native::prefs::init(None, cx);
        cx.set_global(Journal::default());
        proxymate_native::features::register(cx);
    });
    let workspace = cx.new(|cx| {
        Workspace::new(
            None,
            ProxyStatus {
                listening: Some("127.0.0.1:9090".into()),
                recording: true,
                ..Default::default()
            },
            cx,
        )
    });
    cx.update(|cx| Workspace::set_global(workspace.clone(), cx));
    let storage = cx.new(|_| Latest {
        value: Some(StorageStatus::default()),
    });
    let sidebar = cx.new(|_| Latest { value: None });
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let table = cx.new(|cx| FlowTable::snapshot(vec![], 0, cx));
        let parts = Parts {
            workspace,
            storage,
            sidebar,
            engine: None,
        };
        let main = cx.new(|cx| shell::MainWindow::new(parts, table, window, cx));
        shell::root(main, window, cx)
    });
    cx.run_until_parked();
    cx
}

fn journal(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| cx.global::<Journal>().0.clone())
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn click(cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        let id = ElementId::Name(id.to_owned().into());
        assert!(window.try_find(id.clone()).is_some(), "no element {id:?}");
        window.click(id, cx);
    });
    cx.run_until_parked();
}

fn exists(cx: &mut VisualTestContext, id: &str) -> bool {
    cx.update(|window, _| window.try_find(ElementId::Name(id.to_owned().into())).is_some())
}

fn run(cx: &mut VisualTestContext, menu: Menu, label: &str) {
    let ran = cx.update(|window, cx| menu.run(label, window, cx));
    assert!(ran, "no entry {label:?}");
    cx.run_until_parked();
}

fn set_trust(cx: &mut VisualTestContext, keychain: KeychainTrust, verified: bool) {
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let status = TrustStatus {
            keychain,
            verified,
            detail: (!verified).then(|| "invalid peer certificate: UnknownIssuer (InvalidCertificate)".into()),
        };
        caches.trust.update(cx, |c, cx| c.set((), status, cx));
        caches.ca.update(cx, |c, cx| c.set((), cert::sample_ca(), cx));
    });
    cx.run_until_parked();
}

fn set_devices(cx: &mut VisualTestContext, devices: Vec<DeviceClient>) {
    cx.update(|_, cx| {
        let entity = bridge::devices(cx);
        entity.update(cx, |d, cx| {
            d.value = Some(DevicesSnapshot {
                devices,
                ..Default::default()
            });
            cx.notify();
        });
    });
    cx.run_until_parked();
}

fn pending(ip: &str) -> DeviceClient {
    DeviceClient {
        ip: ip.into(),
        nickname: None,
        decision: DeviceDecision::Pending,
        label: ip.into(),
        flows: 0,
        https_decrypted: 0,
        https_refused: 0,
        onboarding: false,
        last_seen_ms: None,
        waiting: 2,
        held_until_ms: Some(f64::MAX),
    }
}

#[gpui_kit::test]
fn the_wizard_installs_trust_through_the_journal(cx: &mut TestAppContext) {
    let cx = open(cx);
    set_trust(cx, KeychainTrust::NotTrusted, false);
    // The toolbar warns, and its button opens the wizard.
    assert!(exists(cx, "trust-warning"));
    click(cx, "trust-warning");
    assert!(cx.update(|_, cx| CertWizard::open_one(cx).is_some()));
    assert!(exists(cx, "cert-wizard"));
    click(cx, "install-ca");
    assert_eq!(journal(cx), ["install_ca_trust"]);
    let wizard = cx.update(|_, cx| CertWizard::open_one(cx).unwrap());
    assert_eq!(
        cx.read(|cx| wizard.read(cx).busy),
        Some(cert::Busy::Install),
        "the button waits for macOS"
    );
    // Remove is off until the keychain trusts the CA; a second open is the same wizard.
    cx.update(|window, cx| window.dispatch_action(Box::new(OpenCertWizard), cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| CertWizard::open_one(cx)), Some(wizard));

    // Trusted and verified: no warning, and Done closes the wizard.
    set_trust(cx, KeychainTrust::Trusted, true);
    assert!(!exists(cx, "trust-warning"));
    let wizard = cx.update(|_, cx| CertWizard::open_one(cx).unwrap());
    cx.update(|_, cx| {
        wizard.update(cx, |w, cx| {
            w.busy = None;
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "uninstall-ca");
    assert_eq!(journal(cx), ["install_ca_trust", "uninstall_ca_trust"]);
    cx.update(|_, cx| wizard.update(cx, |w, _| w.busy = None));
    cx.run_until_parked();
    click(cx, "cert-wizard-done");
    assert!(cx.update(|_, cx| CertWizard::open_one(cx).is_none()));
}

#[gpui_kit::test]
fn the_certificate_popover_copies_and_regenerates(cx: &mut TestAppContext) {
    let cx = open(cx);
    set_trust(cx, KeychainTrust::Trusted, true);
    assert!(cx.update(|_, cx| integration::open_named("certificate", cx)));
    cx.run_until_parked();
    assert!(exists(cx, "certificate-panel"));
    let ca: CaInfo = cert::sample_ca();
    let menu = cx.update(|_, cx| cert::current_ca_menu(cx).unwrap());
    assert_eq!(
        menu.labels(),
        [
            "Copy common name",
            "Copy expiry date",
            "Copy SHA-256",
            "Copy SHA-256 without colons",
            "Copy PEM file path",
            "Copy curl example",
        ]
    );
    run(cx, menu, "Copy SHA-256");
    assert!(clipboard(cx).unwrap().starts_with("5e:8c:0f:6b"));
    let menu = cx.update(|_, cx| cert::current_ca_menu(cx).unwrap());
    run(cx, menu, "Copy curl example");
    assert_eq!(
        clipboard(cx).unwrap(),
        format!(
            "curl -x http://127.0.0.1:9090 --cacert \"{}\" https://example.com/",
            ca.cert_path
        )
    );

    // Regenerate… asks; the switch defaults to removing the old trust here.
    click(cx, "regenerate-ca");
    assert!(journal(cx).is_empty());
    assert!(exists(cx, "untrust-old-ca"));
    click(cx, "regenerate-ca");
    assert_eq!(journal(cx), ["regenerate_ca remove_old_trust=true"]);

    // Manage… opens the wizard and closes the popover.
    click(cx, "open-cert-wizard");
    assert!(cx.update(|_, cx| CertWizard::open_one(cx).is_some()));
    let open = cx.update(|_, cx| integration::certificate_tool(cx).unwrap().read(cx).open);
    assert!(!open);
}

#[gpui_kit::test]
fn the_system_proxy_popover_turns_it_on(cx: &mut TestAppContext) {
    let cx = open(cx);
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let status = SystemProxyStatus {
            enabled: false,
            services: vec![],
            supported: true,
            error: None,
        };
        caches.system_proxy.update(cx, |c, cx| c.set((), status, cx));
        integration::open_tool(Tool::SystemProxy, cx);
    });
    cx.run_until_parked();
    // Opening reads the settings again; without an engine the value stays.
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let status = SystemProxyStatus {
            enabled: false,
            services: vec![],
            supported: true,
            error: None,
        };
        caches.system_proxy.update(cx, |c, cx| c.set((), status, cx));
    });
    cx.run_until_parked();
    click(cx, "system-proxy-toggle");
    assert_eq!(journal(cx), ["set_system_proxy true"]);
    // On: the services can be copied.
    let on = SystemProxyStatus {
        enabled: true,
        services: vec!["Wi-Fi".into(), "Thunderbolt Bridge".into()],
        supported: true,
        error: None,
    };
    run(
        cx,
        proxymate_native::features::integration::sysproxy::services_menu(&on),
        "Copy services",
    );
    assert_eq!(clipboard(cx).as_deref(), Some("Wi-Fi\nThunderbolt Bridge"));
}

fn sim(udids: &[(&str, bool)]) -> SimulatorStatus {
    SimulatorStatus {
        available: true,
        detail: None,
        booted: udids
            .iter()
            .map(|(udid, installed)| SimDevice {
                udid: (*udid).into(),
                name: format!("iPhone {udid}"),
                runtime: "iOS 27.0".into(),
                cert_installed: *installed,
            })
            .collect(),
    }
}

#[gpui_kit::test]
fn device_setup_installs_to_simulators_and_switches_tabs(cx: &mut TestAppContext) {
    let cx = open(cx);
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        caches
            .simulator
            .update(cx, |c, cx| c.set((), sim(&[("A1", false), ("B2", true)]), cx));
        caches.access.update(cx, |c, cx| c.set((), DeviceAccess::default(), cx));
    });
    // The listening pill opens the sheet.
    click(cx, "listening-status");
    assert!(cx.update(|_, cx| devices::open_one(cx).is_some()));
    assert!(exists(cx, "device-tab-simulator"));
    click(cx, "simulator-install");
    assert_eq!(journal(cx), ["install_simulator_cert []"]);

    let sheet = cx.update(|_, cx| devices::open_one(cx).unwrap());
    cx.update(|_, cx| sheet.update(cx, |s, _| s.busy = None));
    let menu = cx.update(|_, cx| devices::menu_for("simulator:A1", cx).unwrap());
    assert_eq!(
        menu.labels(),
        [
            "Copy name",
            "Copy UDID",
            "Copy runtime",
            "—",
            "Install certificate on this simulator"
        ]
    );
    run(cx, menu, "Install certificate on this simulator");
    assert_eq!(journal(cx)[1], "install_simulator_cert [A1]");
    // The journal never answers: let the next write start.
    cx.update(|_, cx| sheet.update(cx, |s, _| s.busy = None));

    // LAN access, then the phone tabs show the QR code's fallback.
    click(cx, "lan-access");
    assert_eq!(journal(cx)[2], "set_lan_access true");
    cx.update(|window, cx| devices::open(Some(DeviceTab::Iphone), window, cx));
    cx.run_until_parked();
    assert!(exists(cx, "device-tab-iphone"));
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let access = DeviceAccess {
            enabled: true,
            interface: Some("en0".into()),
            interfaces: vec![
                LanInterface {
                    name: "en0".into(),
                    ip: "192.168.1.20".into(),
                },
                LanInterface {
                    name: "bridge100".into(),
                    ip: "192.168.2.1".into(),
                },
            ],
            port: Some(9090),
            proxy: Some("192.168.1.20:9090".into()),
            setup_url: Some("http://192.168.1.20:9090/".into()),
            qr_svg: pm_core_free_qr(),
            error: None,
        };
        caches.access.update(cx, |c, cx| c.set((), access, cx));
    });
    cx.run_until_parked();
    assert!(exists(cx, "device-qr"));
    let menu = cx.update(|_, cx| devices::menu_for("interface", cx).unwrap());
    assert_eq!(
        menu.labels(),
        ["en0 · 192.168.1.20", "bridge100 · 192.168.2.1", "—", "Copy IP address"]
    );
    run(cx, menu, "bridge100 · 192.168.2.1");
    assert_eq!(journal(cx)[3], "set_lan_interface bridge100");

    // The emulator tab: Push certificate and the proxy, by menu.
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let status = AndroidStatus {
            available: true,
            detail: None,
            devices: vec![AndroidDevice {
                serial: "emulator-5554".into(),
                model: Some("sdk_gphone64_arm64".into()),
                emulator: true,
                state: "device".into(),
                proxy: None,
            }],
            emulator_proxy: Some("10.0.2.2:9090".into()),
        };
        caches.android.update(cx, |c, cx| c.set((), status, cx));
    });
    cx.update(|_, cx| sheet.update(cx, |s, cx| s.set_tab(DeviceTab::Emulator, cx)));
    cx.run_until_parked();
    click(cx, "emulator-push-emulator-5554");
    assert_eq!(journal(cx)[4], "push_android_cert emulator-5554");
    cx.update(|_, cx| sheet.update(cx, |s, _| s.busy = None));
    let menu = cx.update(|_, cx| devices::menu_for("emulator:emulator-5554", cx).unwrap());
    run(cx, menu, "Use Proxymate as its proxy");
    assert_eq!(journal(cx)[5], "set_android_proxy emulator-5554 true");
}

/// A tiny SVG in place of the engine's QR code.
fn pm_core_free_qr() -> Option<String> {
    Some(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><rect width="1" height="1"/></svg>"#.into())
}

#[gpui_kit::test]
fn the_lan_prompt_asks_counts_down_and_waits_when_closed(cx: &mut TestAppContext) {
    let cx = open(cx);
    set_devices(cx, vec![pending("10.0.0.23"), pending("10.0.0.24")]);
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)).as_deref(), Some("10.0.0.23"));
    assert!(exists(cx, "device-allow-prompt"));
    assert!(exists(cx, "device-countdown"));
    run(cx, prompt::title_menu("10.0.0.23"), "Copy IP address");
    assert_eq!(clipboard(cx).as_deref(), Some("10.0.0.23"));

    // Typing a nickname and Allow.
    cx.simulate_input("Pixel");
    click(cx, "device-allow");
    assert_eq!(journal(cx), ["decide_device 10.0.0.23 allow=true nickname=Pixel"]);

    // Closing the prompt keeps the device waiting; the next one is asked about.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)).as_deref(), Some("10.0.0.24"));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)), None);
    assert!(exists(cx, "devices-waiting"));
    // "2 devices waiting" asks again.
    click(cx, "devices-waiting");
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)).as_deref(), Some("10.0.0.23"));
    click(cx, "device-deny");
    assert_eq!(journal(cx)[1], "decide_device 10.0.0.23 allow=false nickname=");

    // The decision lands: the prompt moves on, then closes.
    let mut allowed = pending("10.0.0.23");
    allowed.decision = DeviceDecision::Denied;
    set_devices(cx, vec![allowed.clone(), pending("10.0.0.24")]);
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)).as_deref(), Some("10.0.0.24"));
    let mut other = pending("10.0.0.24");
    other.decision = DeviceDecision::Allowed;
    set_devices(cx, vec![allowed, other]);
    assert_eq!(cx.update(|_, cx| Prompts::showing(cx)), None);
    assert!(!exists(cx, "devices-waiting"));
    // A known device's menu: Deny is still offered for an allowed one.
    let menu = devices::device_menu(&DeviceClient {
        decision: DeviceDecision::Allowed,
        nickname: Some("Pixel".into()),
        ..pending("10.0.0.24")
    });
    assert_eq!(
        menu.labels(),
        ["Copy IP address", "Copy nickname", "—", "Deny", "Forget"]
    );
}

#[gpui_kit::test]
fn intercept_launches_through_the_journal(cx: &mut TestAppContext) {
    let cx = open(cx);
    cx.update(|_, cx| {
        let caches = integration_caches(cx);
        let apps = pm_view::integration::INTERCEPT_TARGETS
            .iter()
            .map(|e| InterceptApp {
                target: e.target,
                name: e.name.into(),
                installed: e.target != InterceptTarget::Arc,
            })
            .collect();
        caches.intercept_apps.update(cx, |c, cx| c.set((), apps, cx));
        let running = vec![InterceptSession {
            id: 1,
            target: InterceptTarget::Terminal,
            name: "Terminal".into(),
            pid: Some(4242),
            started_at_ms: 0.0,
            notes: vec![],
        }];
        caches.intercepts.update(cx, |c, cx| c.set((), running, cx));
        integration::open_tool(Tool::Intercept, cx);
    });
    cx.run_until_parked();
    assert!(exists(cx, "intercept-popover"));
    click(cx, "intercept-chrome");
    assert_eq!(journal(cx), ["intercept Chrome"]);
    // The popover closed for the launch.
    assert!(!cx.update(|_, cx| integration::intercept_tool(cx).unwrap().read(cx).open));
    // The palette hides what isn't installed.
    assert!(cx.update(|_, cx| commands::is_available("intercept-firefox", cx)));
    assert!(!cx.update(|_, cx| commands::is_available("intercept-arc", cx)));
    cx.update(|window, cx| commands::run("intercept-terminal", window, cx));
    cx.run_until_parked();
    assert_eq!(journal(cx)[1], "intercept Terminal");
    // The other palette entries open the wizard and the sheet.
    cx.update(|window, cx| commands::run("devices", window, cx));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| devices::open_one(cx).is_some()));
    cx.update(|window, cx| window.dispatch_action(Box::new(OpenDeviceSetup), cx));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| Toasts::last(cx).is_none()), "no G2 stub toast");
}

#[gpui_kit::test]
fn the_mcp_indicator_counts_agents(cx: &mut TestAppContext) {
    let cx = open(cx);
    let set = |cx: &mut VisualTestContext, clients: &[&str]| {
        cx.update(|_, cx| {
            let entity: Entity<Latest<McpStatus>> = bridge::mcp(cx);
            let status = McpStatus {
                url: Some("http://127.0.0.1:9095/mcp".into()),
                clients: clients
                    .iter()
                    .map(|n| McpClient {
                        session: (*n).into(),
                        name: (*n).into(),
                        version: "1".into(),
                        connected_at_ms: 0.0,
                    })
                    .collect(),
                ..Default::default()
            };
            entity.update(cx, |e, cx| {
                e.value = Some(status);
                cx.notify();
            });
        });
        cx.run_until_parked();
    };
    set(cx, &[]);
    assert!(exists(cx, "mcp-indicator"));
    assert!(!exists(cx, "mcp-client-count"));
    set(cx, &["claude-code", "cursor"]);
    assert!(exists(cx, "mcp-client-count"));
}
