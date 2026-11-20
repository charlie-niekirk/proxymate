//! G0 spikes (a) and (e), measured with gpui's real offscreen renderer and
//! the platform text system (no window on screen):
//!
//! - (a) the gpui-kit code editor, read-only, on a ~20 MB JSON body: open,
//!   first frame, scroll, jump to the end, search, select all and copy.
//! - (e) `uniform_list` over 100,000 rows of flow-table-like cells, fed by a
//!   RowCache-shaped page cache (128-row pages), scrolled a page per frame.
//!
//! `cargo run --release -p proxymate-native --example spike_bench [a|e]`.
//! Results are written up in `docs/PLAN.md` (Native track, G0 spikes).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, HeadlessAppContext, IntoElement,
    ParentElement as _, Render, ScrollDelta, Styled as _, Window, WindowBounds, WindowOptions, div, point, px, size,
    uniform_list,
};

fn context() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        pm_theme::kit::apply(pm_theme::Mode::Dark, cx);
    });
    cx
}

fn options(w: f32, h: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(w), px(h)),
        })),
        focus: true,
        show: false,
        ..Default::default()
    }
}

/// Times `f` and prints it.
fn time<R>(label: &str, f: impl FnOnce() -> R) -> R {
    let start = Instant::now();
    let r = f();
    println!("  {label:<40} {:>9.1} ms", start.elapsed().as_secs_f64() * 1e3);
    r
}

fn stats(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let p = |q: f64| ms(samples[((samples.len() - 1) as f64 * q) as usize]);
    let over = samples.iter().filter(|d| ms(**d) > 16.7).count();
    println!(
        "  {label:<40} p50 {:>6.2} ms  p95 {:>6.2} ms  p99 {:>6.2} ms  max {:>6.2} ms  >16.7 ms: {over}/{}",
        p(0.5),
        p(0.95),
        p(0.99),
        p(1.0),
        samples.len()
    );
}

fn frame(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// A pretty-printed JSON array of about `target` bytes.
fn big_json(target: usize) -> String {
    let mut out = String::with_capacity(target + 4096);
    out.push_str("[\n");
    let mut i = 0usize;
    while out.len() < target {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!(
            "  {{\n    \"id\": {i},\n    \"name\": \"item-{i}\",\n    \"active\": {},\n    \"score\": {}.{},\n    \
             \"tags\": [\"alpha\", \"beta\", \"gamma\"],\n    \"owner\": {{ \"login\": \"user{}\", \"url\": \
             \"https://api.example.com/users/{}\" }}\n  }}",
            i.is_multiple_of(2),
            i % 1000,
            i % 97,
            i % 5000,
            i % 5000
        ));
        i += 1;
    }
    out.push_str("\n]\n");
    out
}

struct EditorView {
    state: Entity<EditorState>,
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(Editor::new(&self.state).readonly(true).h(px(860.)))
    }
}

fn spike_a() {
    println!("(a) gpui-kit editor, read-only JSON");
    let json = big_json(20 << 20);
    let lines = json.lines().count();
    println!("  body: {:.1} MB, {lines} lines", json.len() as f64 / (1 << 20) as f64);
    let mut cx = context();
    let (handle, state) = time("open window + EditorState::new", || {
        let mut state = None;
        let handle = cx
            .update(|cx| {
                cx.open_window(options(1200., 900.), |window, cx| {
                    let editor = cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language("json")
                            .line_number(true)
                            .searchable(true)
                    });
                    state = Some(editor.clone());
                    let view = cx.new(|_| EditorView { state: editor });
                    cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
                })
            })
            .unwrap();
        (handle.into(), state.unwrap())
    });
    time("set_value(20 MB)", || {
        cx.update_window(handle, |_, window, cx| {
            state.update(cx, |s, cx| s.set_value(json.clone(), window, cx));
        })
        .unwrap();
    });
    time("first frame", || frame(&mut cx, handle));
    time("first frame + GPU render + readback", || {
        cx.capture_screenshot(handle).unwrap()
    });
    time("settle (highlighting tasks)", || cx.run_until_parked());
    time("frame after settle", || frame(&mut cx, handle));

    // Focus the editor so keys reach it.
    cx.update_window(handle, |_, window, cx| {
        let focus = state.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
    })
    .unwrap();
    let mut scroll = Vec::new();
    for _ in 0..120 {
        let start = Instant::now();
        cx.update_window(handle, |_, window, cx| window.press("pagedown", cx))
            .unwrap();
        scroll.push(start.elapsed());
    }
    stats("page down (key + frame) x120", scroll);
    let mut wheel = Vec::new();
    let editor_id = cx
        .update_window(handle, |_, window, _| window.try_find("input").map(|_| "input"))
        .unwrap();
    if let Some(id) = editor_id {
        for _ in 0..120 {
            let start = Instant::now();
            cx.update_window(handle, |_, window, cx| {
                window.scroll(id, ScrollDelta::Pixels(point(px(0.), px(-240.))), cx)
            })
            .unwrap();
            wheel.push(start.elapsed());
        }
        stats("wheel 240 px (event + frame) x120", wheel);
    }
    time("cmd-down (jump to end) + frame", || {
        cx.update_window(handle, |_, window, cx| window.press("cmd-down", cx))
            .unwrap()
    });
    time("cmd-up (back to start) + frame", || {
        cx.update_window(handle, |_, window, cx| window.press("cmd-up", cx))
            .unwrap()
    });
    time("cmd-f, type a needle, enter", || {
        cx.update_window(handle, |_, window, cx| {
            window.press("cmd-f", cx);
            window.input("user4999", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
    });
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    time("cmd-a + cmd-c (copy 20 MB)", || {
        cx.update_window(handle, |_, window, cx| {
            window.press("cmd-a", cx);
            window.press("cmd-c", cx);
        })
        .unwrap()
    });
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()).map(|t| t.len()));
    println!("  clipboard after copy: {copied:?} bytes (body {})", json.len());
}

/// A RowCache-shaped cache: 128-row pages, filled on demand from a source.
struct RowCache {
    pages: HashMap<usize, Rc<Vec<Row>>>,
    fetched: usize,
}

#[derive(Clone)]
struct Row {
    id: u32,
    method: &'static str,
    url: String,
    status: u16,
    size: String,
    time: String,
}

const PAGE: usize = 128;

impl RowCache {
    fn row(&mut self, ix: usize) -> Row {
        let page = ix / PAGE;
        let rows = self.pages.entry(page).or_insert_with(|| {
            self.fetched += 1;
            Rc::new(
                (page * PAGE..(page + 1) * PAGE)
                    .map(|i| Row {
                        id: i as u32 + 1,
                        method: ["GET", "POST", "PUT", "DELETE"][i % 4],
                        url: format!(
                            "https://api{}.example.com/v1/items/{i}?page={}&sort=desc",
                            i % 7,
                            i % 50
                        ),
                        status: [200, 201, 304, 404, 500][i % 5],
                        size: format!("{:.1} KB", (i % 900) as f32 / 3.0),
                        time: format!("12:{:02}:{:02}.{:03}", (i / 60) % 60, i % 60, i % 1000),
                    })
                    .collect(),
            )
        });
        let row = rows[ix % PAGE].clone();
        // Evict pages more than 16 pages away, as lib/rowCache.ts does.
        self.pages.retain(|p, _| p.abs_diff(page) <= 16);
        row
    }
}

struct ListView {
    count: usize,
    cache: Rc<RefCell<RowCache>>,
    scroll: gpui_kit::UniformListScrollHandle,
}

impl Render for ListView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *pm_theme::kit::TokensExt::tokens(&**cx);
        let cache = self.cache.clone();
        uniform_list("flows", self.count, move |range, _, _| {
            let mut cache = cache.borrow_mut();
            range
                .map(|ix| {
                    let row = cache.row(ix);
                    let (fg, bg) = match row.status / 100 {
                        2 => (t.s_2xx, t.s_2xx_bg),
                        3 => (t.s_3xx, t.s_3xx_bg),
                        4 => (t.s_4xx, t.s_4xx_bg),
                        _ => (t.s_5xx, t.s_5xx_bg),
                    };
                    div()
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .text_size(px(12.))
                        .when(ix % 2 == 1, |d| d.bg(t.pane_row_alt.hsla()))
                        .child(
                            div()
                                .w(px(56.))
                                .text_color(t.muted_foreground.hsla())
                                .child(row.id.to_string()),
                        )
                        .child(pm_widgets::Pill::new(row.method, t.m_get, t.m_get_bg))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_color(t.url_path.hsla())
                                .child(row.url),
                        )
                        .child(pm_widgets::Pill::new(row.status.to_string(), fg, bg))
                        .child(div().w(px(70.)).child(row.size))
                        .child(div().w(px(90.)).child(row.time))
                })
                .collect()
        })
        .track_scroll(&self.scroll)
        .size_full()
        .bg(t.pane_table.hsla())
        .text_color(t.foreground.hsla())
    }
}

use gpui_kit::prelude::FluentBuilder as _;

fn spike_e() {
    println!("(e) uniform_list, 100,000 rows of 22 px, RowCache pages of {PAGE}");
    let mut cx = context();
    let cache = Rc::new(RefCell::new(RowCache {
        pages: HashMap::new(),
        fetched: 0,
    }));
    let scroll = gpui_kit::UniformListScrollHandle::new();
    let handle: AnyWindowHandle = cx
        .update(|cx| {
            let cache = cache.clone();
            let scroll = scroll.clone();
            cx.open_window(options(1200., 800.), move |window, cx| {
                let view = cx.new(|_| ListView {
                    count: 100_000,
                    cache,
                    scroll,
                });
                cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
            })
        })
        .unwrap()
        .into();
    time("first frame", || frame(&mut cx, handle));
    // Scroll a page (36 rows) per frame through the first 10,000 rows, then
    // jump around: the worst case for the cache, every frame a new page.
    let mut frames = Vec::new();
    let mut rendered = Vec::new();
    for step in 0..300usize {
        let target = if step < 280 { step * 36 } else { (step * 7919) % 100_000 };
        let start = Instant::now();
        scroll.scroll_to_item(target, gpui_kit::ScrollStrategy::Top);
        frame(&mut cx, handle);
        frames.push(start.elapsed());
        if step % 10 == 0 {
            let start = Instant::now();
            cx.capture_screenshot(handle).unwrap();
            rendered.push(start.elapsed());
        }
    }
    stats("scroll a page + frame (CPU) x300", frames);
    stats("+ GPU render + readback x30", rendered);
    let mut small = Vec::new();
    for step in 0..300usize {
        let start = Instant::now();
        scroll.scroll_to_item(50_000 + step, gpui_kit::ScrollStrategy::Top);
        frame(&mut cx, handle);
        small.push(start.elapsed());
    }
    stats("scroll a row + frame (CPU) x300", small);
    println!("  pages fetched: {}", cache.borrow().fetched);
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "ae".into());
    if which.contains('e') {
        spike_e();
    }
    if which.contains('a') {
        spike_a();
    }
}
