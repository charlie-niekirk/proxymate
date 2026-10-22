import { describe, expect, it, vi } from "vitest";

import type { FlowRow_Serialize, RowsPage_Serialize } from "@/bindings";

import { PAGE_SIZE, RowCache } from "./rowCache";

/** A fake backend over `ids`, recording every page request. */
function fakeView(ids: number[]) {
  const requests: number[] = [];
  const fetchPage = vi.fn(async (start: number, count: number): Promise<RowsPage_Serialize> => {
    requests.push(start);
    const rows = ids.slice(start, start + count).map(
      (id): FlowRow_Serialize => [id, "completed", "https", "GET", "h", 443, "/", "", 200, "json", null, 0, 1, 0, 0, false, "HTTP/1.1", "intercepted", null, null, false],
    );
    return { view: 1, start, len: ids.length, rows };
  });
  return { requests, fetchPage };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("RowCache", () => {
  it("loads only the pages around the visible window", async () => {
    const ids = Array.from({ length: 100_000 }, (_, i) => i);
    const { requests, fetchPage } = fakeView(ids);
    const onUpdate = vi.fn();
    const cache = new RowCache(fetchPage, onUpdate);
    cache.reset(ids.length);
    cache.ensure(50_000, 50_040);
    await settle();
    // The visible page plus one page of read-ahead on each side.
    expect(requests.sort((a, b) => a - b)).toEqual([49_792, 49_920, 50_048]);
    expect(cache.get(50_000)?.id).toBe(50_000);
    expect(cache.get(0)).toBeUndefined();
    expect(onUpdate).toHaveBeenCalled();
  });

  it("serves stale rows while a reset refetches, then replaces them", async () => {
    const ids = Array.from({ length: 300 }, (_, i) => i);
    const view = fakeView(ids);
    const cache = new RowCache(view.fetchPage, () => {});
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
    const cache = new RowCache(view.fetchPage, () => {});
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
    const cache = new RowCache(view.fetchPage, () => {});
    cache.reset(ids.length);
    cache.ensure(0, PAGE_SIZE * 3);
    await settle();
    view.requests.length = 0;
    cache.invalidateIds([PAGE_SIZE + 5]);
    await settle();
    expect(view.requests).toEqual([PAGE_SIZE]);
  });
});
