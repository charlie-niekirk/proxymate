import { act, createElement, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ViewInfo_Serialize } from "@/bindings";

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
import { useUi } from "@/store/ui";

// React only flushes effects inside `act` when it knows it's a test.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const info = (len: number): ViewInfo_Serialize => ({ view: 7, total: len, len, elapsed_us: 0, deep: null });

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
});
