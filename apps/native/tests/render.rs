//! The main window's shell with the flow table (G1) and its chrome (G2:
//! toolbar, sidebar, document tabs, filter bar, footer; the ⌘K palette),
//! rendered by gpui's real
//! offscreen renderer (Metal on macOS, WGPU on Linux; CI uses Mesa's
//! lavapipe) in both themes, from injected snapshots: no engine, no proxy.
//! The pane and row pixels must be their tokens, and each capture is
//! written to `target/native-snapshots/shell-<theme>.png` for review (CI
//! uploads them).
//!
//! It fails, rather than passing vacuously, when no renderer is available.
//! Its own harness (`harness = false`): the renderer wants the main thread.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    render::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("render: skipped; gpui has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod render {
    use std::path::PathBuf;
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size};
    use pm_model::{
        AppCount, ContentGroup, ContentKind, DocumentInfo, DocumentKind, DomainNode, FlowLabel, FlowRow, FlowState,
        HostCount, HttpVersion, ProxyStatus, SaveProgress, Scheme, SentBy, SidebarSnapshot, StorageStatus, TlsStatus,
    };
    use pm_theme::{Mode, Rgba};
    use pm_view::columns::{HEADER_HEIGHT, ROW_HEIGHT};
    use proxymate_native::bridge::stream::Latest;
    use proxymate_native::features::flow_table::FlowTable;
    use proxymate_native::shell::{self, MainWindow, Parts, filter_bar, tabs};
    use proxymate_native::workspace::{SaveJob, Workspace};

    const SIZE: (f32, f32) = (1200., 760.);

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(pm_widgets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            proxymate_native::prefs::init(None, cx);
            proxymate_native::features::register(cx);
        });
        cx
    }

    /// Rows like the zoo's: every method, status class, state and badge.
    fn rows() -> Vec<FlowRow> {
        let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"];
        let statuses = [Some(200), Some(304), Some(404), Some(500), Some(101), None];
        let hosts = ["api.example.com", "cdn.example.net", "localhost", "auth.example.org"];
        (0..60u32)
            .map(|i| {
                let i_ = i as usize;
                let state = match i % 11 {
                    5 => FlowState::Failed,
                    7 => FlowState::Active,
                    9 => FlowState::Tunnel,
                    _ => FlowState::Completed,
                };
                FlowRow(
                    i,
                    state,
                    if i % 4 == 2 { Scheme::Http } else { Scheme::Https },
                    methods[i_ % methods.len()].into(),
                    hosts[i_ % hosts.len()].into(),
                    if i % 4 == 2 { 3000 } else { 443 },
                    format!("/v1/items/{i}"),
                    if i % 3 == 0 { format!("page={i}") } else { String::new() },
                    statuses[i_ % statuses.len()],
                    ContentKind::Json,
                    Some(["Safari", "node", "Firefox"][i_ % 3].into()),
                    1_700_000_000_000.0 + i as f64 * 37.0,
                    Some(i * 13),
                    (i as u64 % 3) * 700,
                    i as u64 * 1500,
                    i % 6 == 0,
                    if i % 2 == 0 {
                        HttpVersion::Http2
                    } else {
                        HttpVersion::Http11
                    },
                    [
                        TlsStatus::Intercepted,
                        TlsStatus::Passthrough,
                        TlsStatus::None,
                        TlsStatus::LikelyPinned,
                    ][i_ % 4],
                    (i % 8 == 1).then(|| "GetViewer".into()),
                    [None, Some(SentBy::User), None, Some(SentBy::Agent)][i_ % 4],
                    i % 10 == 0,
                    (i % 9 == 4).then_some(5),
                    (i % 5 == 3).then_some(FlowLabel::Blue),
                    (i % 7 == 2).then(|| "check".into()),
                    vec![],
                )
            })
            .collect()
    }

    /// Apps and domains like a browsing session's.
    fn sidebar() -> SidebarSnapshot {
        let app = |name: &str, count| AppCount {
            name: name.into(),
            bundle_id: None,
            has_icon: false,
            remote: false,
            count,
        };
        SidebarSnapshot {
            store: 0,
            total: 60,
            apps: vec![app("Firefox", 20), app("node", 20), app("Safari", 20)],
            domains: vec![
                DomainNode {
                    domain: "example.com".into(),
                    count: 45,
                    hosts: vec![
                        HostCount {
                            host: "api.example.com".into(),
                            count: 30,
                        },
                        HostCount {
                            host: "cdn.example.net".into(),
                            count: 15,
                        },
                    ],
                },
                DomainNode {
                    domain: "localhost".into(),
                    count: 15,
                    hosts: vec![],
                },
            ],
        }
    }

    fn snapshot_dir() -> PathBuf {
        // `target/tmp` is Cargo's scratch dir for integration tests; its
        // sibling holds the captures (CARGO_TARGET_DIR is honoured).
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("../native-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assert_pixel(image: &image::RgbaImage, scale: f32, at: (f32, f32), want: Rgba, what: &str) {
        let (x, y) = ((at.0 * scale) as u32, (at.1 * scale) as u32);
        let px = image.get_pixel(x, y).0;
        let got = [px[0], px[1], px[2]];
        let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
        let close = (0..3).all(|i| (got[i] as i32 - want[i]).abs() <= 3);
        assert!(close, "{what} at {at:?}: got {got:?}, want {want:?}");
    }

    /// `fg` (maybe translucent, like `--pane-row-alt`) composited over `bg`.
    fn over(fg: Rgba, bg: Rgba) -> Rgba {
        let mix = |f: f32, b: f32| f * fg.a + b * (1. - fg.a);
        Rgba {
            r: mix(fg.r, bg.r),
            g: mix(fg.g, bg.g),
            b: mix(fg.b, bg.b),
            a: 1.,
        }
    }

    fn shell_renders(mode: Mode) {
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
        let handle = cx
            .update(|cx| {
                let storage = cx.new(|_| Latest {
                    value: Some(StorageStatus {
                        flows: 1234,
                        pinned: 3,
                        disk_bytes: 5 << 20,
                        unrecorded: 12,
                        ..Default::default()
                    }),
                });
                let sidebar = cx.new(|_| Latest { value: Some(sidebar()) });
                let proxy = ProxyStatus {
                    listening: Some("127.0.0.1:9090".into()),
                    recording: true,
                    ..Default::default()
                };
                let workspace = cx.new(|cx| {
                    let mut ws = Workspace::new(None, proxy, cx);
                    ws.documents = vec![DocumentInfo {
                        store: 1,
                        kind: DocumentKind::Har,
                        name: "checkout.har".into(),
                        path: "/tmp/checkout.har".into(),
                        flows: 42,
                    }];
                    ws.filter.content = vec![ContentGroup::Json];
                    ws.filter.client = Some("Safari".into());
                    ws.set_save(
                        Some(SaveJob::new(
                            "Saving session",
                            SaveProgress {
                                done: 60,
                                total: 60,
                                finished: true,
                                error: None,
                                path: "/tmp/Proxymate 2026-11-20 10.00.proxymate".into(),
                            },
                        )),
                        cx,
                    );
                    ws
                });
                Workspace::set_global(workspace.clone(), cx);
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(SIZE.0), px(SIZE.1)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                };
                cx.open_window(options, |window, cx| {
                    let table = cx.new(|cx| FlowTable::snapshot(rows(), 60, cx));
                    let parts = Parts {
                        workspace,
                        storage,
                        sidebar,
                        engine: None,
                    };
                    let view = cx.new(|cx| MainWindow::new(parts, table, window, cx));
                    cx.new(|cx| shell::root(view, window, cx))
                })
            })
            .expect("open the window");
        // Let the rows load, then select row 1 (flow #2).
        cx.run_until_parked();
        cx.update(|cx| {
            let table = proxymate_native::features::flow_table::first(cx).expect("a table");
            let grid = table.read(cx).grid().clone();
            grid.update(cx, |g, cx| g.select_index(1, cx));
        });
        cx.run_until_parked();
        let scale = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.scale_factor()
            })
            .unwrap();
        let image = cx
            .capture_screenshot(handle.into())
            .expect("gpui's offscreen renderer must be available");
        let name = if mode.is_dark() { "dark" } else { "light" };
        let path = snapshot_dir().join(format!("shell-{name}.png"));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());

        let t = mode.tokens();
        let main_x = shell::SIDEBAR_WIDTH + 40.;
        assert_pixel(&image, scale, (main_x, 8.), t.pane_toolbar, "toolbar");
        // The table: its header, then rows 0 (plain), 1 (selected) and 3 (odd),
        // sampled in the # column's empty left part.
        // The document tabs (a document is open) and the filter bar.
        let tabs_y = shell::TOOLBAR_HEIGHT;
        assert_pixel(
            &image,
            scale,
            (main_x + 400., tabs_y + 4.),
            t.pane_toolbar,
            "document tabs",
        );
        let bar_y = tabs_y + tabs::TABS_HEIGHT;
        assert_pixel(&image, scale, (main_x + 380., bar_y + 4.), t.pane_toolbar, "filter bar");
        // The All protocol chip is on; JSON is the content chip on.
        assert_pixel(
            &image,
            scale,
            (shell::SIDEBAR_WIDTH + 11., bar_y + filter_bar::BAR_HEIGHT / 2.),
            over(t.primary_chip_bg, t.pane_toolbar),
            "the All protocol chip",
        );
        let top = bar_y + filter_bar::BAR_HEIGHT;
        let row_y = |i: f32| top + HEADER_HEIGHT + ROW_HEIGHT * i + ROW_HEIGHT / 2.;
        let x = shell::SIDEBAR_WIDTH + 6.;
        assert_pixel(
            &image,
            scale,
            (x, top + HEADER_HEIGHT / 2.),
            over(t.pane_header, t.pane_table),
            "table header",
        );
        assert_pixel(&image, scale, (x, row_y(0.)), t.pane_table, "row 0");
        assert_pixel(
            &image,
            scale,
            (x, row_y(1.)),
            over(t.selection_bg, t.pane_table),
            "selected row 1",
        );
        assert_pixel(
            &image,
            scale,
            (x, row_y(3.)),
            over(t.pane_row_alt, t.pane_table),
            "odd row 3",
        );
        // The inspector placeholder under the table's 58%.
        let content = SIZE.1 - top - shell::FOOTER_HEIGHT;
        assert_pixel(
            &image,
            scale,
            (main_x, top + content * shell::TABLE_SHARE + 40.),
            t.background,
            "inspector",
        );
        assert_pixel(&image, scale, (main_x + 300., SIZE.1 - 3.), t.pane_toolbar, "footer");
        if !cfg!(target_os = "macos") {
            // macOS leaves the sidebar clear for the vibrancy behind the window.
            // Under the sections, above the filter box at the bottom.
            assert_pixel(&image, scale, (4., SIZE.1 - 60.), t.sidebar, "sidebar");
        }

        // The ⌘K palette over the shell, with the commands G2 registers.
        cx.update_window(handle.into(), |_, window, cx| {
            proxymate_native::features::commands::open_palette(window, cx)
        })
        .unwrap();
        for _ in 0..30 {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let image = cx.capture_screenshot(handle.into()).expect("a palette frame");
        let path = snapshot_dir().join(format!("palette-{name}.png"));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
        palette_checks(&image, scale, &t);
    }

    /// The palette's panel is drawn in the popover colour, centred under
    /// the toolbar.
    fn palette_checks(image: &image::RgbaImage, scale: f32, t: &pm_theme::Tokens) {
        // The panel's left padding (520 px wide, centred, 80 px down).
        let x = (SIZE.0 - 520.) / 2. + 6.;
        let px = image.get_pixel((x * scale) as u32, (300. * scale) as u32).0;
        let close = |want: pm_theme::Rgba| {
            let want = [want.r, want.g, want.b].map(|c| (c * 255.0).round() as i32);
            (0..3).all(|i| (px[i] as i32 - want[i]).abs() <= 3)
        };
        assert!(
            close(t.background) || close(t.popover),
            "palette panel at ({x}, 300): got {:?}, want the background or popover token",
            &px[..3]
        );
    }

    pub fn run() {
        for mode in [Mode::Dark, Mode::Light] {
            println!("running shell_renders ({mode:?})");
            shell_renders(mode);
            println!("passed shell_renders ({mode:?})");
        }
        println!("render: 2 passed (real offscreen renderer)");
    }
}
