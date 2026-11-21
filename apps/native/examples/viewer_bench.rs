//! G4's large-body budget: a 20 MB JSON body ("Load all"'s limit) shown the
//! way the Body tab shows it above the 2 MB editor limit: re-indented
//! without parsing (`pm_view::body::json_format`), cut into rows on the
//! background executor, and drawn by the CodeView (`uniform_list`, only the
//! rows on screen highlighted), with gpui's real offscreen renderer.
//!
//! `cargo run --release -p proxymate-native --features spikes --example viewer_bench`

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, HeadlessAppContext, IntoElement, ParentElement as _, Render,
    ScrollStrategy, Styled as _, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use pm_view::body::highlight::Lang;
use pm_widgets::code_view::{CodeView, Highlight};

fn context() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(pm_widgets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        pm_theme::kit::apply(pm_theme::Mode::Dark, cx);
        pm_widgets::code_view::init(cx);
    });
    cx
}

struct Host {
    view: Entity<CodeView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

fn stats(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let p = |q: f64| ms(samples[((samples.len() - 1) as f64 * q) as usize]);
    let over = samples.iter().filter(|d| ms(**d) > 16.7).count();
    println!(
        "  {label:<44} p50 {:>6.2} ms  p95 {:>6.2} ms  p99 {:>6.2} ms  max {:>6.2} ms  >16.7 ms: {over}/{}",
        p(0.5),
        p(0.95),
        p(0.99),
        p(1.0),
        samples.len()
    );
}

fn frame(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
}

fn main() {
    let raw = pm_testkit::zoo::big_json(20 << 20);
    println!("viewer_bench: {:.1} MB of minified JSON", raw.len() as f64 / (1 << 20) as f64);
    let start = Instant::now();
    let text = pm_view::body::text_of(&raw).expect("text");
    let pretty = pm_view::body::json_format::format_json(text, "  ").expect("JSON");
    println!(
        "  format_json (re-indent, no parse)            {:>8.1} ms  ({:.1} MB, {} lines)",
        start.elapsed().as_secs_f64() * 1e3,
        pretty.len() as f64 / (1 << 20) as f64,
        pretty.bytes().filter(|b| *b == b'\n').count() + 1
    );
    for (label, highlight, wrap) in [
        ("plain", None, false),
        ("highlighted anyway", Some(Highlight::Lang(Lang::Json)), false),
        ("wrapped", None, true),
    ] {
        println!("{label}:");
        let mut cx = context();
        let text: gpui_kit::SharedString = pretty.clone().into();
        let mut view = None;
        let start = Instant::now();
        let handle: AnyWindowHandle = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(900.), px(700.)),
                        })),
                        focus: true,
                        show: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        let v = cx.new(|cx| {
                            CodeView::new("bench", "Bench", text, cx)
                                .highlight(highlight)
                                .wrap(wrap, cx)
                        });
                        view = Some(v.clone());
                        let host = cx.new(|_| Host { view: v });
                        cx.new(|cx| gpui_kit::base::Root::new(host, window, cx))
                    },
                )
            })
            .unwrap()
            .into();
        let view = view.unwrap();
        frame(&mut cx, handle);
        println!(
            "  first frame (rows cutting in the background) {:>8.1} ms",
            start.elapsed().as_secs_f64() * 1e3
        );
        while cx.update(|cx| view.read(cx).is_indexing()) {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(1));
        }
        frame(&mut cx, handle);
        cx.run_until_parked();
        frame(&mut cx, handle);
        let rows = cx.update(|cx| view.read(cx).row_count());
        println!(
            "  rows ready + frame                           {:>8.1} ms  ({rows} rows)",
            start.elapsed().as_secs_f64() * 1e3
        );
        // A page (about 38 rows) per frame, then far jumps.
        let mut page = Vec::new();
        for i in 0..240 {
            let begin = Instant::now();
            cx.update(|cx| view.read(cx).scroll_handle().scroll_to_item(i * 38 + 60, ScrollStrategy::Top));
            frame(&mut cx, handle);
            page.push(begin.elapsed());
        }
        stats("scroll a page per frame x240", page);
        let mut jumps = Vec::new();
        for i in 0..120 {
            let begin = Instant::now();
            let target = (i * 7919 * 13) % rows.max(1);
            cx.update(|cx| view.read(cx).scroll_handle().scroll_to_item(target, ScrollStrategy::Top));
            frame(&mut cx, handle);
            jumps.push(begin.elapsed());
        }
        stats("jump anywhere x120", jumps);
        let begin = Instant::now();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, cx| v.open_find(window, cx));
            window.input("user4999", cx);
        })
        .unwrap();
        while cx.update(|cx| view.read(cx).match_count()) == 0 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(1));
        }
        frame(&mut cx, handle);
        println!(
            "  ⌘F \"user4999\": matches found + frame          {:>8.1} ms  ({} matches)",
            begin.elapsed().as_secs_f64() * 1e3,
            cx.update(|cx| view.read(cx).match_count())
        );
        // The last frame, to see what was drawn.
        if let Ok(image) = cx.capture_screenshot(handle) {
            let path = std::env::temp_dir().join(format!("viewer_bench-{}.png", label.replace(' ', "-")));
            if image.save(&path).is_ok() {
                println!("  last frame: {}", path.display());
            }
        }
    }
}
