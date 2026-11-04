import { describe, expect, it, vi } from "vitest";

import type { FlowRow_Serialize, RowsPage_Serialize } from "@/bindings";

import { MAX_FETCH_PAGES, PAGE_SIZE, RowCache, type Schedule } from "./rowCache";

/** A fake backend over `ids`, recording every page request. */
function fakeView(ids: number[]) {
  const requests: number[] = [];
  const counts: number[] = [];
  const fetchPage = vi.fn(async (start: number, count: number): Promise<RowsPage_Serialize> => {
    requests.push(start);
    counts.push(count);
    const rows = ids.slice(start, start + count).map(
      (id): FlowRow_Serialize => [id, "completed", "https", "GET", "h", 443, "/", "", 200, "json", null, 0, 1, 0, 0, false, "HTTP/1.1", "intercepted", null, null, false, null, null, null, []],
    );
    return { view: 1, start, len: ids.length, rows };
  });
  return { requests, counts, fetchPage };
}

const settle = () => new Promise((r) => setTimeout(r, 0));
/** Runs scheduled fetches straight away (the app waits for the next frame). */
const now: Schedule = (run) => queueMicrotask(run);

describe("RowCache", () => {
  it("loads only the pages around the visible window", async () => {
    const ids = Array.from({ length: 100_000 }, (_, i) => i);
    const { requests, fetchPage } = fakeView(ids);
    const onUpdate = vi.fn();
    const cache = new RowCache(fetchPage, onUpdate, now);
    cache.reset(ids.length);
    cache.ensure(50_000, 50_040);
    await settle();
    // The visible page plus one page of read-ahead on each side, in one fetch.
    expect(requests).toEqual([49_792]);
    expect(fetchPage).toHaveBeenCalledWith(49_792, 3 * PAGE_SIZE);
    expect(cache.get(50_000)?.id).toBe(50_000);
    expect(cache.get(0)).toBeUndefined();
    expect(onUpdate).toHaveBeenCalled();
  });

  it("serves stale rows while a reset refetches, then replaces them", async () => {
    const ids = Array.from({ length: 300 }, (_, i) => i);
    const view = fakeView(ids);
    const cache = new RowCache(view.fetchPage, () => {}, now);
    cache.reset(ids.length);
    cache.ensure(0, 20);
    await settle();
    ids.reverse();
    cache.reset(ids.length);
    expect(cache.get(0)?.id).toBe(0); // stale, no blank flash
    await settle();
    expect(cache.get(0)?.id).toBe(299);
  });

  it("refetches the partial last page when rows are appended", async () => {
    const ids = Array.from({ length: 10 }, (_, i) => i);
    const view = fakeView(ids);
    const cache = new RowCache(view.fetchPage, () => {}, now);
    cache.reset(ids.length);
    cache.ensure(0, 40);
    await settle();
    expect(cache.get(10)).toBeUndefined();
    ids.push(10, 11);
    cache.append(ids.length);
    await settle();
    expect(cache.get(11)?.id).toBe(11);
  });

  it("refetches pages holding changed ids", async () => {
    const ids = Array.from({ length: PAGE_SIZE * 3 }, (_, i) => i);
    const view = fakeView(ids);
    const cache = new RowCache(view.fetchPage, () => {}, now);
    cache.reset(ids.length);
    cache.ensure(0, PAGE_SIZE * 3);
    await settle();
    view.requests.length = 0;
    cache.invalidateIds([PAGE_SIZE + 5]);
    await settle();
    expect(view.requests).toEqual([PAGE_SIZE]);
  });

  it("coalesces the asks of one frame into fetches for the latest window", async () => {
    const ids = Array.from({ length: 100_000 }, (_, i) => i);
    const view = fakeView(ids);
    const frames: (() => void)[] = [];
    const cache = new RowCache(view.fetchPage, () => {}, (run) => frames.push(run));
    cache.reset(ids.length);
    // A fast scroll: many windows before the frame, only the last one counts.
    for (let first = 0; first < 20_000; first += 500) cache.ensure(first, first + 40);
    const at = 470 * PAGE_SIZE;
    cache.ensure(at, at + PAGE_SIZE * 8 - 1);
    expect(view.requests).toEqual([]);
    expect(frames).toHaveLength(1);
    frames.shift()?.();
    await settle();
    // 10 pages wanted (8 visible and 2 read-ahead): runs of at most MAX_FETCH_PAGES.
    expect(view.counts).toEqual([4, 4, 2].map((n) => n * PAGE_SIZE));
    expect(view.requests[0]).toBe(at - PAGE_SIZE);
    expect(Math.max(...view.counts)).toBeLessThanOrEqual(MAX_FETCH_PAGES * PAGE_SIZE);
    expect(cache.get(at)?.id).toBe(at);
    expect(cache.get(at + PAGE_SIZE * 9 - 1)?.id).toBe(at + PAGE_SIZE * 9 - 1);
  });
});
