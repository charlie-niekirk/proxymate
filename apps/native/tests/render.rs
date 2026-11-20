//! The main window's shell with the flow table (G1), rendered by gpui's real
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
        ContentKind, FlowLabel, FlowRow, FlowState, HttpVersion, ProxyStatus, Scheme, SentBy, StorageStatus, TlsStatus,
    };
    use pm_theme::{Mode, Rgba};
    use pm_view::columns::{HEADER_HEIGHT, ROW_HEIGHT};
    use proxymate_native::bridge::stream::Latest;
    use proxymate_native::features::flow_table::FlowTable;
    use proxymate_native::shell::{self, MainWindow};

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
                        disk_bytes: 5 << 20,
                        ..Default::default()
                    }),
                });
                let proxy = ProxyStatus {
                    listening: Some("127.0.0.1:9090".into()),
                    recording: true,
                    ..Default::default()
                };
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
                    let view = cx.new(|cx| MainWindow::new(storage, proxy, table, window, cx));
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
        let top = shell::TOOLBAR_HEIGHT;
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
        assert_pixel(&image, scale, (main_x + 300., SIZE.1 - 4.), t.pane_toolbar, "footer");
        if !cfg!(target_os = "macos") {
            // macOS leaves the sidebar clear for the vibrancy behind the window.
            assert_pixel(&image, scale, (8., SIZE.1 - 20.), t.sidebar, "sidebar");
        }
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
