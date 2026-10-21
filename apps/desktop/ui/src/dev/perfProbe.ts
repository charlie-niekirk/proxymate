/**
 * Dev-only performance probe, enabled with `VITE_PM_PERF_PROBE=1 npm run tauri dev`.
 *
 * Inside the real app it scrolls the flow table and cycles filters, then
 * POSTs the numbers to the Vite dev server, which prints them to the
 * terminal (see `perfProbeSink` in vite.config.ts). Nothing here ships.
 */
import { useUi } from "@/store/ui";

const nextFrame = () => new Promise<number>((r) => requestAnimationFrame(r));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function percentile(xs: number[], p: number): number {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] ?? 0;
}

async function waitFor(cond: () => boolean, timeoutMs = 30_000) {
  const start = performance.now();
  while (!cond()) {
    if (performance.now() - start > timeoutMs) throw new Error("perf probe: timed out");
    await sleep(50);
  }
}

/** Scrolls `pxPerFrame` every frame for `frames` frames; reports frame times and unloaded rows. */
async function scrollRun(el: HTMLElement, pxPerFrame: number, frames: number) {
  const deltas: number[] = [];
  let placeholderFrames = 0;
  let last = await nextFrame();
  for (let i = 0; i < frames; i++) {
    el.scrollTop += pxPerFrame;
    const now = await nextFrame();
    deltas.push(now - last);
    last = now;
    if (el.querySelector("[data-placeholder]")) placeholderFrames++;
  }
  const avg = deltas.reduce((a, b) => a + b, 0) / deltas.length;
  return {
    pxPerFrame,
    frames,
    avgFps: Math.round(1000 / avg),
    p95FrameMs: +percentile(deltas, 95).toFixed(1),
    maxFrameMs: +Math.max(...deltas).toFixed(1),
    framesOver20ms: deltas.filter((d) => d > 20).length,
    framesWithUnloadedRows: placeholderFrames,
  };
}

async function filterRun(apply: () => void, label: string) {
  const before = useUi.getState().filterTiming;
  apply();
  await waitFor(() => useUi.getState().filterTiming !== before, 5_000);
  const t = useUi.getState().filterTiming!;
  await sleep(150);
  return { label, rows: useUi.getState().stats.len, engineMs: +t.engineMs.toFixed(2), endToEndMs: +t.endToEndMs.toFixed(1) };
}

export async function runPerfProbe() {
  await waitFor(() => useUi.getState().stats.len >= 100_000);
  await sleep(1_000);
  const el = document.querySelector<HTMLElement>("[data-testid=flow-table]");
  if (!el) throw new Error("perf probe: no flow table");

  const scroll = [
    await scrollRun(el, 22 * 3, 240), // steady trackpad-like scroll
    await scrollRun(el, 22 * 40, 240), // fast flick
  ];
  el.scrollTop = el.scrollHeight / 2;
  await sleep(300);
  scroll.push(await scrollRun(el, -22 * 200, 120)); // very fast fling upwards

  const ui = useUi.getState();
  const filters = [];
  for (const [label, apply] of [
    ["text api", () => ui.setText("api")],
    ["text stripe.com/v1", () => ui.setText("stripe.com/v1")],
    ["https", () => ui.setProtocol("https")],
    ["+json", () => ui.toggleContent("json")],
    ["+media", () => ui.toggleContent("media")],
    ["no match", () => ui.setText("zzzz-none")],
    ["clear text", () => ui.setText("")],
    ["all content", () => ui.clearContent()],
    ["all protocols", () => ui.setProtocol("all")],
  ] as const) {
    filters.push(await filterRun(apply, label));
  }
  const report = { rows: useUi.getState().stats.total, scroll, filters };
  await fetch("/__pm_perf", { method: "POST", body: JSON.stringify(report, null, 2) });
  // Leave a representative screen: top of the table with a flow inspected.
  el.scrollTop = 0;
  useUi.getState().select(3);
}
