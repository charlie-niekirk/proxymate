//! The main window's shell, rendered by gpui's real offscreen renderer
//! (Metal on macOS, WGPU on Linux; CI uses Mesa's lavapipe) in both themes,
//! from injected snapshots: no engine, no proxy. The pane pixels must be
//! their tokens, and each capture is written to
//! `target/native-snapshots/shell-<theme>.png` for review (CI uploads them).
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
    use pm_model::{ProxyStatus, StorageStatus};
    use pm_theme::{Mode, Rgba};
    use proxymate_native::bridge::stream::Latest;
    use proxymate_native::shell::{self, MainWindow};

    const SIZE: (f32, f32) = (1200., 760.);

    fn context() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(gpui_kit::assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            proxymate_native::prefs::init(None, cx);
        });
        cx
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
                    let view = cx.new(|cx| MainWindow::new(storage, proxy, window, cx));
                    cx.new(|cx| shell::root(view, window, cx))
                })
            })
            .expect("open the window");
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
        assert_pixel(
            &image,
            scale,
            (main_x, shell::TOOLBAR_HEIGHT + 40.),
            t.pane_table,
            "content",
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
