import { act, createElement, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ViewDelta_Serialize, ViewInfo_Serialize } from "@/bindings";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast }));

const backend = vi.hoisted(() => ({
  createView: vi.fn(),
  subscribe: vi.fn(),
  setSidebarStore: vi.fn(),
  dropView: vi.fn(),
  getRows: vi.fn(),
  setFilter: vi.fn(),
  setSort: vi.fn(),
}));
vi.mock("@/lib/backend", () => ({ backend }));

import { FILTER_DEBOUNCE_MS, useFlowView } from "@/hooks/useFlowView";
import { flowKey, queryClient } from "@/lib/query";
import { useUi } from "@/store/ui";

// React only flushes effects inside `act` when it knows it's a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const info = (len: number, view = 7): ViewInfo_Serialize => ({ view, total: len, len, elapsed_us: 0, deep: null });

const delta = (d: Partial<ViewDelta_Serialize>): ViewDelta_Serialize => ({
  view: 7,
  seq: 0,
  total: 10,
  len: 10,
  appended: 0,
  changed: [],
  structural: false,
  deep: null,
  ...d,
});

const initialUi = useUi.getState();

let result: ReturnType<typeof useFlowView>;
function Probe() {
  const current = useFlowView();
  useEffect(() => {
    result = current;
  });
  return null;
}

/** A promise whose settling the test controls. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe("useFlowView", () => {
  let root: Root;

  beforeEach(() => {
    for (const fn of Object.values(backend)) fn.mockReset();
    backend.subscribe.mockResolvedValue(undefined);
    backend.setSidebarStore.mockResolvedValue(undefined);
    backend.dropView.mockResolvedValue(undefined);
    backend.getRows.mockResolvedValue({ start: 0, rows: [] });
    toast.error.mockReset();
    useUi.setState(initialUi, true);
    queryClient.clear();
    root = createRoot(document.createElement("div"));
  });
  afterEach(() => act(() => root.unmount()));

  it("reports a view it couldn't open, and opens it on retry", async () => {
    backend.createView.mockRejectedValueOnce(new Error("engine unavailable"));
    await act(async () => root.render(createElement(Probe)));
    expect(result.error).toBe("engine unavailable");
    expect(result.cache).toBeNull();
    expect(toast.error).toHaveBeenCalledWith("Couldn't open the flow list", { description: "engine unavailable" });

    backend.createView.mockResolvedValueOnce(info(3));
    await act(async () => result.retry());
    expect(result.error).toBeNull();
    expect(result.cache?.len).toBe(3);
    expect(backend.createView).toHaveBeenCalledTimes(2);
  });

  it("lets only the latest sort's reply reset the rows", async () => {
    backend.createView.mockResolvedValueOnce(info(10));
    await act(async () => root.render(createElement(Probe)));
    const first = deferred<ViewInfo_Serialize>();
    const second = deferred<ViewInfo_Serialize>();
    backend.setSort.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    await act(async () => useUi.getState().setSort({ key: "url", dir: "asc" }));
    await act(async () => useUi.getState().setSort({ key: "status", dir: "desc" }));
    // The newer reply lands first; the stale one arrives after it.
    await act(async () => second.resolve(info(20)));
    await act(async () => first.resolve(info(99)));
    expect(result.cache?.len).toBe(20);
  });

  it("debounces typing in the search box, but applies chips at once", async () => {
    backend.createView.mockResolvedValueOnce(info(10));
    await act(async () => root.render(createElement(Probe)));
    backend.setFilter.mockResolvedValue(info(5));
    await act(async () => useUi.getState().setText("ap"));
    await act(async () => useUi.getState().setText("api"));
    expect(backend.setFilter).not.toHaveBeenCalled();
    await act(() => new Promise((r) => setTimeout(r, FILTER_DEBOUNCE_MS + 50)));
    expect(backend.setFilter).toHaveBeenCalledTimes(1);
    expect(backend.setFilter).toHaveBeenLastCalledWith(7, expect.objectContaining({ text: "api" }));

    await act(async () => useUi.getState().setProtocol("https"));
    expect(backend.setFilter).toHaveBeenCalledTimes(2);
    // Clearing the box is immediate too.
    await act(async () => useUi.getState().setText(""));
    expect(backend.setFilter).toHaveBeenCalledTimes(3);
    await act(async () => useUi.getState().setProtocol("all"));
  });

  /** Renders the hook over a view of `len` rows; resolves to the delta sink. */
  async function open(len: number) {
    let push!: (d: ViewDelta_Serialize) => void;
    backend.subscribe.mockImplementationOnce((_view: number, onDelta: (d: ViewDelta_Serialize) => void) => {
      push = onDelta;
      return Promise.resolve();
    });
    backend.createView.mockResolvedValueOnce(info(len));
    await act(async () => root.render(createElement(Probe)));
    return (d: Partial<ViewDelta_Serialize>) => act(async () => push(delta(d)));
  }

  it("follows deltas: appends grow the rows, a structural one resets them", async () => {
    const push = await open(10);
    const before = result.version;
    await push({ len: 12, total: 12, appended: 2 });
    expect(result.cache?.len).toBe(12);
    expect(useUi.getState().stats).toEqual({ len: 12, total: 12 });
    expect(result.version).toBeGreaterThan(before);
    await push({ len: 4, total: 12, structural: true });
    expect(result.cache?.len).toBe(4);
    expect(useUi.getState().stats).toEqual({ len: 4, total: 12 });
  });

  it("refetches the inspected flow when a delta changes it, and drops the selection on a clear", async () => {
    const push = await open(10);
    useUi.getState().select(3);
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    await push({ changed: [5] });
    expect(invalidate).not.toHaveBeenCalled();
    await push({ changed: [3] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: flowKey(0, 3) });
    // A selection made before its flow arrived (cached as gone) is retried as rows land.
    invalidate.mockClear();
    queryClient.setQueryData(flowKey(0, 3), null);
    await push({ len: 11, total: 11, appended: 1 });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: flowKey(0, 3) });
    await push({ len: 0, total: 0, structural: true });
    expect(useUi.getState().selectedId).toBeNull();
    invalidate.mockRestore();
  });

  it("opens a new view for another store and drops the old one", async () => {
    await open(10);
    expect(backend.createView).toHaveBeenLastCalledWith(0, expect.anything(), expect.anything());
    backend.createView.mockResolvedValueOnce(info(3, 8));
    await act(async () => useUi.getState().setStore(2));
    expect(backend.dropView).toHaveBeenCalledWith(7);
    expect(backend.createView).toHaveBeenLastCalledWith(2, expect.anything(), expect.anything());
    expect(backend.setSidebarStore).toHaveBeenLastCalledWith(2);
    expect(result.view).toBe(8);
    expect(result.cache?.len).toBe(3);
  });

  it("drops a view that arrives after the table has gone", async () => {
    const slow = deferred<ViewInfo_Serialize>();
    backend.createView.mockReturnValueOnce(slow.promise);
    await act(async () => root.render(createElement(Probe)));
    await act(async () => root.render(null));
    await act(async () => slow.resolve(info(5, 9)));
    expect(backend.dropView).toHaveBeenCalledWith(9);
    expect(backend.subscribe).not.toHaveBeenCalled();
  });
});
