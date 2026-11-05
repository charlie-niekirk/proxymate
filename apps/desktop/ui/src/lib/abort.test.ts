import { describe, expect, it, vi } from "vitest";

import { SIGNAL_ARG, abortable, withSignals } from "@/lib/abort";
import type { Backend } from "@/lib/backend";
import { createMockBackend } from "@/lib/mockBackend";

describe("abortable", () => {
  it("runs the read without a signal", async () => {
    await expect(abortable(undefined, () => Promise.resolve(1))).resolves.toBe(1);
  });

  it("never starts a read whose signal has already aborted", async () => {
    const read = vi.fn(() => Promise.resolve(1));
    const controller = new AbortController();
    controller.abort(new Error("gone"));
    await expect(abortable(controller.signal, read)).rejects.toThrow("gone");
    expect(read).not.toHaveBeenCalled();
  });

  it("rejects as soon as the signal aborts, dropping the late answer", async () => {
    let answer!: (n: number) => void;
    const controller = new AbortController();
    const pending = abortable(controller.signal, () => new Promise<number>((r) => (answer = r)));
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    answer(1);
  });

  it("passes the read's own answer and error through", async () => {
    const signal = new AbortController().signal;
    await expect(abortable(signal, () => Promise.resolve("ok"))).resolves.toBe("ok");
    await expect(abortable(signal, () => Promise.reject(new Error("engine")))).rejects.toThrow("engine");
  });
});

describe("withSignals", () => {
  it("names each read's signal at its last parameter", () => {
    const backend = createMockBackend(0);
    for (const [name, at] of Object.entries(SIGNAL_ARG)) {
      // `length` counts parameters before the first default: the signal sits past every one.
      expect(at, name).toBeGreaterThanOrEqual((backend[name as keyof Backend] as (...a: unknown[]) => unknown).length);
    }
  });

  it("calls the read without the signal, and not at all once aborted", async () => {
    const getFlow = vi.fn(() => Promise.resolve(null));
    const wrapped = withSignals({ getFlow } as unknown as Backend);
    const controller = new AbortController();
    await wrapped.getFlow(0, 3, controller.signal);
    expect(getFlow).toHaveBeenCalledWith(0, 3);
    controller.abort();
    await expect(wrapped.getFlow(0, 4, controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(getFlow).toHaveBeenCalledTimes(1);
  });

  it("leaves other methods as they are", () => {
    const getProxyStatus = vi.fn();
    const wrapped = withSignals({ getProxyStatus } as unknown as Backend);
    expect(wrapped.getProxyStatus).toBe(getProxyStatus);
  });
});
