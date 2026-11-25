//! macOS integration's surfaces (G13) rendered by gpui's real offscreen
//! renderer (Metal on macOS, WGPU on Linux; CI uses Mesa's lavapipe) in
//! both themes, from injected snapshots: no engine, nothing on the machine
//! touched. The certificate wizard (untrusted, then trusted), the
//! Certificate popover confirming a regeneration, the system proxy popover,
//! the Device setup sheet on each tab (booted simulators, the QR code, the
//! Android snippets, an emulator), the LAN "Allow?" prompt counting down
//! for a waiting device, the Intercept popover with a running launch, and
//! the toolbar's tools (the trust warning, "2 devices waiting", Intercept's
//! count, the green system proxy, the MCP count). Each capture's status
//! colours must be drawn, and is written to
//! `target/native-snapshots/integration-<case>-<theme>.png` (CI uploads
//! them).
//!
//! A sibling of `tests/render.rs` (same harness rules: its own `main`, the
//! renderer wants the main thread; it fails rather than passing vacuously
//! without a renderer).

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    render::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("render_integration: skipped; gpui has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod render {
    use std::path::PathBuf;
    use std::sync::Arc;

    use gpui_kit::prelude::*;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyView, App, Context, HeadlessAppContext, Render, Window, WindowBounds, WindowOptions, div, point, px, size,
    };
    use pm_model::{
        AndroidDevice, AndroidStatus, DeviceAccess, DeviceClient, DeviceDecision, DevicesSnapshot, InterceptApp,
        InterceptSession, InterceptTarget, KeychainTrust, LanInterface, LocalActivity, McpClient, McpStatus,
        ProxyStatus, SimDevice, SimulatorStatus, SystemProxyStatus, TrustStatus,
    };
    use pm_theme::kit::TokensExt as _;
    use pm_theme::{Mode, Rgba, Tokens};
    use proxymate_native::bridge;
    use proxymate_native::bridge::keys::integration_caches;
    use proxymate_native::features::integration::cert::{self, CertPanel, CertWizard};
    use proxymate_native::features::integration::devices::{DeviceSetup, DeviceTab};
    use proxymate_native::features::integration::{intercept, mcp, prompt, sysproxy};
    use proxymate_native::workspace::Workspace;

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(pm_widgets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            proxymate_native::prefs::init(None, cx);
            cx.set_global(proxymate_native::bridge::mutations::Journal::default());
            proxymate_native::features::register(cx);
            let ws = cx.new(|cx| {
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
            Workspace::set_global(ws, cx);
        });
        cx
    }

    fn snapshot_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("../native-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A surface on its pane colour, as a dialog, sheet or popover shows it.
    struct Host {
        view: AnyView,
        popover: bool,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let t = cx.tokens();
            let bg = if self.popover { t.popover } else { t.background };
            div()
                .size_full()
                .p(px(16.))
                .bg(bg.hsla())
                .text_color(t.foreground.hsla())
                .child(self.view.clone())
        }
    }

    type Make = fn(&mut Window, &mut App) -> AnyView;

    struct Case {
        name: &'static str,
        size: (f32, f32),
        popover: bool,
        /// Injects what the surface shows.
        setup: fn(&mut App),
        make: Make,
        /// Colours that must be drawn, with what they are.
        expect: Vec<(fn(&Tokens) -> Rgba, &'static str)>,
    }

    fn trust(cx: &mut App, keychain: KeychainTrust, verified: bool) {
        let caches = integration_caches(cx);
        let status = TrustStatus {
            keychain,
            verified,
            detail: (!verified).then(|| "invalid peer certificate: UnknownIssuer (InvalidCertificate)".into()),
        };
        caches.trust.update(cx, |c, cx| c.set((), status, cx));
        caches.ca.update(cx, |c, cx| c.set((), cert::sample_ca(), cx));
    }

    fn system_proxy(cx: &mut App, enabled: bool) {
        let caches = integration_caches(cx);
        let status = SystemProxyStatus {
            enabled,
            services: if enabled {
                vec!["Wi-Fi".into(), "Thunderbolt Bridge".into()]
            } else {
                vec![]
            },
            supported: true,
            error: None,
        };
        caches.system_proxy.update(cx, |c, cx| c.set((), status, cx));
    }

    fn devices(cx: &mut App, devices: Vec<DeviceClient>, simulator: LocalActivity) {
        let entity = bridge::devices(cx);
        entity.update(cx, |d, cx| {
            d.value = Some(DevicesSnapshot {
                devices,
                simulator,
                emulator: LocalActivity::default(),
            });
            cx.notify();
        });
    }

    fn device(ip: &str, decision: DeviceDecision, flows: u32) -> DeviceClient {
        DeviceClient {
            ip: ip.into(),
            nickname: None,
            decision,
            label: if decision == DeviceDecision::Allowed {
                "Charlie's iPhone".into()
            } else {
                ip.into()
            },
            flows,
            https_decrypted: flows / 2,
            https_refused: 0,
            onboarding: true,
            last_seen_ms: Some(1.0),
            waiting: if decision == DeviceDecision::Pending { 2 } else { 0 },
            held_until_ms: (decision == DeviceDecision::Pending).then(|| now_ms() + 18_000.0),
        }
    }

    fn now_ms() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            * 1000.0
    }

    /// A QR-like grid standing in for the engine's code (`pm_core` draws
    /// the real one; this test has no engine).
    fn qr_svg() -> String {
        let mut cells = String::new();
        for y in 0..21 {
            for x in 0..21 {
                let finder = |cx: i32, cy: i32| {
                    let (dx, dy) = ((x - cx).abs(), (y - cy).abs());
                    dx <= 3 && dy <= 3 && (dx.max(dy) != 2)
                };
                if finder(3, 3) || finder(17, 3) || finder(3, 17) || ((x * 7 + y * 13) % 5 == 0 && x > 7 && y > 7) {
                    cells.push_str(&format!(r#"<rect x="{x}" y="{y}" width="1" height="1"/>"#));
                }
            }
        }
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 25 25" shape-rendering="crispEdges"><rect x="-2" y="-2" width="25" height="25" fill="white"/>{cells}</svg>"#
        )
    }

    fn access(cx: &mut App, enabled: bool) {
        let caches = integration_caches(cx);
        let access = DeviceAccess {
            enabled,
            interface: Some("en0".into()),
            interfaces: vec![LanInterface {
                name: "en0".into(),
                ip: "192.168.1.20".into(),
            }],
            port: Some(9090),
            proxy: enabled.then(|| "192.168.1.20:9090".into()),
            setup_url: enabled.then(|| "http://192.168.1.20:9090/".into()),
            qr_svg: enabled.then(qr_svg),
            error: None,
        };
        caches.access.update(cx, |c, cx| c.set((), access, cx));
    }

    fn sheet(tab: DeviceTab) -> Make {
        match tab {
            DeviceTab::Simulator => |_, cx| cx.new(|cx| DeviceSetup::new(DeviceTab::Simulator, cx)).into(),
            DeviceTab::Iphone => |_, cx| cx.new(|cx| DeviceSetup::new(DeviceTab::Iphone, cx)).into(),
            DeviceTab::Android => |_, cx| cx.new(|cx| DeviceSetup::new(DeviceTab::Android, cx)).into(),
            DeviceTab::Emulator => |_, cx| cx.new(|cx| DeviceSetup::new(DeviceTab::Emulator, cx)).into(),
        }
    }

    fn cases() -> Vec<Case> {
        let s_2xx: fn(&Tokens) -> Rgba = |t| t.s_2xx;
        let s_4xx: fn(&Tokens) -> Rgba = |t| t.s_4xx;
        let primary: fn(&Tokens) -> Rgba = |t| t.primary;
        vec![
            Case {
                name: "wizard-untrusted",
                size: (520., 560.),
                popover: false,
                setup: |cx| trust(cx, KeychainTrust::NotTrusted, false),
                make: |_, cx| cx.new(CertWizard::new).into(),
                expect: vec![(s_4xx, "the failed checks"), (primary, "Install & Trust")],
            },
            Case {
                name: "wizard-trusted",
                size: (520., 560.),
                popover: false,
                setup: |cx| trust(cx, KeychainTrust::Trusted, true),
                make: |_, cx| cx.new(CertWizard::new).into(),
                expect: vec![(s_2xx, "the passed checks and the note"), (primary, "Done")],
            },
            Case {
                name: "certificate-popover",
                size: (420., 520.),
                popover: true,
                setup: |cx| trust(cx, KeychainTrust::Trusted, true),
                make: |_, cx| {
                    let panel = cx.new(CertPanel::new);
                    panel.update(cx, |p, cx| p.regenerate(cx));
                    panel.into()
                },
                expect: vec![(s_2xx, "Trusted by this Mac"), (|t| t.muted, "the copy fields")],
            },
            Case {
                name: "system-proxy-popover",
                size: (360., 300.),
                popover: true,
                setup: |cx| system_proxy(cx, true),
                make: |_, cx| cx.new(sysproxy::SystemProxyPanel::new).into(),
                expect: vec![(s_2xx, "the on dot")],
            },
            Case {
                name: "device-setup-simulator",
                size: (480., 900.),
                popover: false,
                setup: |cx| {
                    access(cx, false);
                    system_proxy(cx, false);
                    let caches = integration_caches(cx);
                    let sim = SimulatorStatus {
                        available: true,
                        detail: None,
                        booted: vec![
                            SimDevice {
                                udid: "8A1B".into(),
                                name: "iPhone 17 Pro".into(),
                                runtime: "iOS 27.0".into(),
                                cert_installed: true,
                            },
                            SimDevice {
                                udid: "9C2D".into(),
                                name: "iPad Air".into(),
                                runtime: "iOS 27.0".into(),
                                cert_installed: false,
                            },
                        ],
                    };
                    caches.simulator.update(cx, |c, cx| c.set((), sim, cx));
                    devices(
                        cx,
                        vec![
                            device("192.168.1.31", DeviceDecision::Allowed, 12),
                            device("192.168.1.44", DeviceDecision::Denied, 0),
                        ],
                        LocalActivity {
                            flows: 9,
                            https_decrypted: 7,
                            https_refused: 0,
                        },
                    );
                },
                make: sheet(DeviceTab::Simulator),
                expect: vec![
                    (s_2xx, "the passed checks and 'Certificate installed'"),
                    (s_4xx, "the system proxy check"),
                    (primary, "Install to all booted"),
                ],
            },
            Case {
                name: "device-setup-iphone",
                size: (480., 980.),
                popover: false,
                setup: |cx| {
                    access(cx, true);
                    devices(
                        cx,
                        vec![device("192.168.1.31", DeviceDecision::Allowed, 12)],
                        LocalActivity::default(),
                    );
                },
                make: sheet(DeviceTab::Iphone),
                expect: vec![
                    (s_2xx, "decrypting the iPhone's HTTPS"),
                    (|t| t.paper, "the QR code's paper"),
                ],
            },
            Case {
                name: "device-setup-android",
                size: (480., 1180.),
                popover: false,
                setup: |cx| {
                    access(cx, false);
                    devices(cx, vec![], LocalActivity::default());
                },
                make: sheet(DeviceTab::Android),
                expect: vec![(|t| t.muted, "the QR fallback and the snippets")],
            },
            Case {
                name: "device-setup-emulator",
                size: (480., 900.),
                popover: false,
                setup: |cx| {
                    access(cx, false);
                    let caches = integration_caches(cx);
                    let status = AndroidStatus {
                        available: true,
                        detail: Some("Android Debug Bridge version 1.0.41".into()),
                        devices: vec![AndroidDevice {
                            serial: "emulator-5554".into(),
                            model: Some("sdk_gphone64_arm64".into()),
                            emulator: true,
                            state: "device".into(),
                            proxy: Some("10.0.2.2:9090".into()),
                        }],
                        emulator_proxy: Some("10.0.2.2:9090".into()),
                    };
                    caches.android.update(cx, |c, cx| c.set((), status, cx));
                },
                make: sheet(DeviceTab::Emulator),
                expect: vec![(s_2xx, "adb and the running emulator")],
            },
            Case {
                name: "allow-prompt",
                size: (432., 300.),
                popover: false,
                setup: |cx| {
                    // The prompt itself, not the dialog the app would open.
                    let dismissed = &mut cx.default_global::<prompt::Prompts>().dismissed;
                    dismissed.insert("192.168.1.57".into());
                    devices(
                        cx,
                        vec![device("192.168.1.57", DeviceDecision::Pending, 0)],
                        LocalActivity::default(),
                    );
                },
                make: |window, cx| {
                    let p = cx.new(|cx| prompt::AllowPrompt::new("192.168.1.57".into(), cx));
                    p.update(cx, |p, cx| p.attach(window, cx));
                    p.into()
                },
                expect: vec![(s_4xx, "the countdown"), (primary, "Allow")],
            },
            Case {
                name: "intercept-popover",
                size: (360., 400.),
                popover: true,
                setup: |cx| {
                    let caches = integration_caches(cx);
                    let apps = pm_view::integration::INTERCEPT_TARGETS
                        .iter()
                        .map(|e| InterceptApp {
                            target: e.target,
                            name: e.name.into(),
                            installed: !matches!(e.target, InterceptTarget::Arc | InterceptTarget::Edge),
                        })
                        .collect();
                    caches.intercept_apps.update(cx, |c, cx| c.set((), apps, cx));
                    let running = vec![InterceptSession {
                        id: 1,
                        target: InterceptTarget::Chrome,
                        name: "Google Chrome".into(),
                        pid: Some(48211),
                        started_at_ms: 0.0,
                        notes: vec![],
                    }];
                    caches.intercepts.update(cx, |c, cx| c.set((), running, cx));
                },
                make: |_, cx| {
                    let caches = integration_caches(cx);
                    let apps = caches.intercept_apps.read(cx).peek(&()).cloned();
                    let running = caches.intercepts.read(cx).peek(&()).cloned().unwrap_or_default();
                    let t = *cx.tokens();
                    cx.new(move |_| Static {
                        element: Box::new(move || intercept::panel(apps.as_deref(), &running, &t, |_, _, _| {})),
                    })
                    .into()
                },
                expect: vec![(s_2xx, "the running dot")],
            },
            Case {
                name: "toolbar-tools",
                size: (640., 64.),
                popover: false,
                setup: |cx| {
                    trust(cx, KeychainTrust::NotTrusted, false);
                    system_proxy(cx, true);
                    devices(
                        cx,
                        vec![
                            device("192.168.1.57", DeviceDecision::Pending, 0),
                            device("192.168.1.58", DeviceDecision::Pending, 0),
                        ],
                        LocalActivity::default(),
                    );
                    let dismissed = &mut cx.default_global::<prompt::Prompts>().dismissed;
                    dismissed.extend(["192.168.1.57".to_owned(), "192.168.1.58".to_owned()]);
                    let mcp = bridge::mcp(cx);
                    mcp.update(cx, |m, cx| {
                        m.value = Some(McpStatus {
                            url: Some("http://127.0.0.1:9095/mcp".into()),
                            clients: vec![McpClient {
                                session: "s".into(),
                                name: "claude-code".into(),
                                version: "2".into(),
                                connected_at_ms: 0.0,
                            }],
                            ..Default::default()
                        });
                        cx.notify();
                    });
                },
                make: |window, cx| {
                    let views = vec![
                        prompt::waiting_devices(window, cx),
                        intercept::intercept_tool(window, cx),
                        cert::trust_warning(window, cx),
                        sysproxy::system_proxy_tool(window, cx),
                        cert::certificate_tool(window, cx),
                        mcp::mcp_indicator(window, cx),
                    ];
                    cx.new(move |_| Row { views }).into()
                },
                expect: vec![
                    (s_4xx, "the trust warning and the waiting devices"),
                    (s_2xx, "the system proxy's globe"),
                    (|t| t.rule_agent, "the MCP count"),
                ],
            },
        ]
    }

    struct Static {
        element: Box<dyn Fn() -> gpui_kit::AnyElement>,
    }

    impl Render for Static {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            (self.element)()
        }
    }

    struct Row {
        views: Vec<AnyView>,
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .p(px(4.))
                .bg(cx.tokens().pane_toolbar.hsla())
                .children(self.views.iter().cloned())
        }
    }

    fn count_near(image: &image::RgbaImage, want: Rgba, tolerance: i32) -> usize {
        let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
        image
            .pixels()
            .filter(|p| (0..3).all(|i| (p.0[i] as i32 - want[i]).abs() <= tolerance))
            .count()
    }

    fn render_case(cx: &mut HeadlessAppContext, mode: Mode, case: &Case) {
        cx.update(|cx| (case.setup)(cx));
        cx.run_until_parked();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(case.size.0), px(case.size.1)),
            })),
            focus: false,
            show: false,
            ..Default::default()
        };
        let make = case.make;
        let popover = case.popover;
        let handle = cx
            .update(|cx| {
                cx.open_window(options, |window, cx| {
                    let view = make(window, cx);
                    let host = cx.new(|_| Host { view, popover });
                    cx.new(|cx| gpui_kit::base::Root::new(host, window, cx))
                })
            })
            .expect("open a window");
        for _ in 0..12 {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let image = cx.capture_screenshot(handle.into()).expect("a frame");
        let theme = if mode.is_dark() { "dark" } else { "light" };
        let path = snapshot_dir().join(format!("integration-{}-{theme}.png", case.name));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
        let t = mode.tokens();
        let bg = if case.popover { t.popover } else { t.background };
        assert!(
            count_near(&image, bg, 3) > 1000,
            "{} ({theme}): the surface isn't on its pane colour",
            case.name
        );
        for (colour, what) in &case.expect {
            let n = count_near(&image, colour(&t), 10);
            assert!(n >= 12, "{} ({theme}): {what} not drawn ({n} pixels)", case.name);
        }
        cx.update_window(handle.into(), |_, window, _| window.remove_window())
            .unwrap();
    }

    pub fn run() {
        let mut total = 0;
        for mode in [Mode::Dark, Mode::Light] {
            let mut cx = context();
            cx.update(|cx| {
                use proxymate_native::prefs::{self, Appearance};
                let forced = if mode.is_dark() {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                prefs::update(cx, |p| p.appearance = forced);
                pm_theme::kit::apply(mode, cx);
            });
            for case in cases() {
                render_case(&mut cx, mode, &case);
                total += 1;
            }
        }
        println!("render_integration: {total} passed (real offscreen renderer, both themes)");
    }
}
