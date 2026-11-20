//! The perf probe (`dev/perfProbe.ts` for the native app), enabled with
//! `PROXYMATE_SYNTH_FLOWS=100000 PROXYMATE_DEV_PERF_PROBE=1` (debug builds,
//! or release ones with `--features dev-harness`; it waits for 100k rows,
//! which only synthetic seeding provides).
//!
//! Inside the real app it scrolls the flow table a set distance every
//! frame and measures the frame cadence (the window's display-link ticks,
//! so a slow frame shows as a long gap) and how many frames showed rows
//! still loading; then it runs the Tauri probe's nine filter changes and
//! times each from the change to the first painted frame with its rows.
//! Results print as `[perf-probe]` lines. The window must be on screen.

use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, point, px};
use pm_model::{ContentGroup, FilterSpec, ProtocolFilter};
use serde_json::{Value, json};

use crate::features::flow_table::{self, FlowTable};

/// Rows the probe waits for, as the Tauri probe does.
const ROWS: usize = 100_000;

pub fn init(cx: &mut App) {
    if !crate::dev_env::flag("PROXYMATE_DEV_PERF_PROBE") {
        return;
    }
    cx.spawn(async move |cx| {
        if let Err(err) = run(cx).await {
            eprintln!("[perf-probe] failed: {err}");
        }
    })
    .detach();
}

async fn sleep(cx: &AsyncApp, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

/// The start of the window's next frame.
async fn next_frame(window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<Instant, String> {
    let (tx, rx) = async_channel::bounded(1);
    window
        .update(cx, |_, window, _| {
            window.on_next_frame(move |_, _| {
                let _ = tx.try_send(Instant::now());
            });
            window.refresh();
        })
        .map_err(|e| e.to_string())?;
    rx.recv().await.map_err(|e| e.to_string())
}

fn percentile(xs: &[f64], p: f64) -> f64 {
    let mut sorted = xs.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((p / 100.0 * sorted.len() as f64) as usize).min(sorted.len() - 1)]
}

/// Moves the table's scroll offset by `dy` pixels (down for positive).
fn scroll_by(table: &Entity<FlowTable>, dy: f32, cx: &mut App) {
    let grid = table.read(cx).grid().clone();
    grid.update(cx, |g, cx| {
        let len = g.len();
        let handle = g.scroll_handle().0.borrow();
        let base = &handle.base_handle;
        let viewport = f32::from(base.bounds().size.height);
        let max = (len as f32 * pm_view::columns::ROW_HEIGHT - viewport).max(0.0);
        let mut offset = base.offset();
        offset.y = px((f32::from(offset.y) - dy).clamp(-max, 0.0));
        base.set_offset(point(offset.x, offset.y));
        drop(handle);
        cx.notify();
    });
}

/// Scrolls `px_per_frame` every frame for `frames` frames.
async fn scroll_run(
    window: AnyWindowHandle,
    table: &Entity<FlowTable>,
    px_per_frame: f32,
    frames: usize,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let mut deltas = Vec::with_capacity(frames);
    let mut unloaded = 0;
    let mut last = next_frame(window, cx).await?;
    for _ in 0..frames {
        cx.update(|cx| scroll_by(table, px_per_frame, cx));
        let now = next_frame(window, cx).await?;
        deltas.push(now.duration_since(last).as_secs_f64() * 1000.0);
        last = now;
        // The frame just drawn: did it show rows still loading?
        if !cx.update(|cx| table.read(cx).grid().read(cx).rendered_loaded()) {
            unloaded += 1;
        }
    }
    let avg = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let report = json!({
        "pxPerFrame": px_per_frame,
        "frames": frames,
        "avgFps": (1000.0 / avg).round(),
        "p95FrameMs": (percentile(&deltas, 95.0) * 10.0).round() / 10.0,
        "maxFrameMs": (deltas.iter().copied().fold(0.0, f64::max) * 10.0).round() / 10.0,
        "framesOver20ms": deltas.iter().filter(|d| **d > 20.0).count(),
        "framesWithUnloadedRows": unloaded,
    });
    eprintln!("[perf-probe] scroll {report}");
    Ok(report)
}

async fn filter_run(
    table: &Entity<FlowTable>,
    label: &str,
    filter: FilterSpec,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let before = cx.update(|cx| table.read(cx).filter_timing());
    cx.update(|cx| table.update(cx, |t, cx| t.set_filter(filter, cx)));
    let started = Instant::now();
    let timing = loop {
        let now = cx.update(|cx| table.read(cx).filter_timing());
        if now != before
            && let Some(t) = now
        {
            break t;
        }
        if started.elapsed() > Duration::from_secs(5) {
            return Err(format!("filter {label:?} timed out"));
        }
        sleep(cx, 5).await;
    };
    sleep(cx, 150).await;
    let report = json!({
        "label": label,
        "rows": timing.rows,
        "engineMs": (timing.engine_ms * 100.0).round() / 100.0,
        "endToEndMs": (timing.end_to_end_ms * 10.0).round() / 10.0,
    });
    eprintln!("[perf-probe] filter {report}");
    Ok(report)
}

async fn run(cx: &mut AsyncApp) -> Result<(), String> {
    let started = Instant::now();
    let table = loop {
        let found = cx.update(|cx| flow_table::first(cx).filter(|t| t.read(cx).grid().read(cx).len() >= ROWS));
        if let Some(table) = found {
            break table;
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err(format!("no table with {ROWS} rows (set PROXYMATE_SYNTH_FLOWS={ROWS})"));
        }
        sleep(cx, 50).await;
    };
    sleep(cx, 1000).await;
    let window = cx.update(|cx| cx.windows().first().copied()).ok_or("no window")?;
    let _ = window.update(cx, |_, window, cx| {
        cx.activate(true);
        window.activate_window();
    });
    sleep(cx, 300).await;
    eprintln!(
        "[perf-probe] start: {} rows",
        cx.update(|cx| table.read(cx).grid().read(cx).len())
    );

    let row = pm_view::columns::ROW_HEIGHT;
    let mut scroll = vec![
        // A steady trackpad-like scroll, a fast flick.
        scroll_run(window, &table, row * 3.0, 240, cx).await?,
        scroll_run(window, &table, row * 40.0, 240, cx).await?,
    ];
    // From the middle, a very fast fling upwards.
    cx.update(|cx| {
        let len = table.read(cx).grid().read(cx).len();
        let grid = table.read(cx).grid().clone();
        grid.update(cx, |g, cx| g.scroll_to(len / 2, cx));
    });
    sleep(cx, 300).await;
    scroll.push(scroll_run(window, &table, -row * 200.0, 120, cx).await?);

    // The Tauri probe's filter changes, applied in turn as the filter row would.
    let mut spec = FilterSpec::default();
    let mut filters = Vec::new();
    let steps: [(&str, fn(&mut FilterSpec)); 9] = [
        ("text api", |f| f.text = "api".into()),
        ("text stripe.com/v1", |f| f.text = "stripe.com/v1".into()),
        ("https", |f| f.protocol = ProtocolFilter::Https),
        ("+json", |f| f.content.push(ContentGroup::Json)),
        ("+media", |f| f.content.push(ContentGroup::Media)),
        ("no match", |f| f.text = "zzzz-none".into()),
        ("clear text", |f| f.text.clear()),
        ("all content", |f| f.content.clear()),
        ("all protocols", |f| f.protocol = ProtocolFilter::All),
    ];
    for (label, change) in steps {
        change(&mut spec);
        filters.push(filter_run(&table, label, spec.clone(), cx).await?);
    }
    let rows = cx.update(|cx| table.read(cx).total());
    let report = json!({ "rows": rows, "scroll": scroll, "filters": filters });
    eprintln!(
        "[perf-probe] report {}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    // Leave a representative screen: the top of the table, a flow selected.
    cx.update(|cx| {
        let grid = table.read(cx).grid().clone();
        grid.update(cx, |g, cx| {
            g.scroll_to(0, cx);
            g.select_index(3, cx);
        });
    });
    Ok(())
}
