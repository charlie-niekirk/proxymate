import { beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => ({
  setRecording: vi.fn(),
  setProxySettings: vi.fn(),
  removeMockSource: vi.fn(),
  regenerateCa: vi.fn(),
  wsSend: vi.fn(),
}));
vi.mock("@/lib/backend", () => ({ backend }));

import { mutations, runMutation } from "@/lib/mutations";
import {
  caInfoKey,
  deviceAccessKey,
  mockSourcesKey,
  proxySettingsKey,
  proxyStatusKey,
  queryClient,
  streamKey,
  terminalEnvKey,
  trustStatusKey,
  wsPagesKey,
} from "@/lib/query";

const status = (recording: boolean) => ({ recording }) as never;

describe("mutations", () => {
  beforeEach(() => {
    queryClient.clear();
    for (const fn of Object.values(backend)) fn.mockReset();
  });

  it("a setter stores its answer under its key", async () => {
    backend.setRecording.mockResolvedValue(status(false));
    await runMutation(mutations.setRecording, false);
    expect(backend.setRecording).toHaveBeenCalledWith(false);
    expect(queryClient.getQueryData(proxyStatusKey)).toEqual(status(false));
  });

  it("a setter refetches what else it moved, and a failure leaves its key alone", async () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    queryClient.setQueryData(proxySettingsKey, { port: 9090 } as never);
    backend.setProxySettings.mockRejectedValue(new Error("port in use"));
    await expect(runMutation(mutations.setProxySettings, { port: 1 } as never)).rejects.toThrow("port in use");
    expect(queryClient.getQueryData(proxySettingsKey)).toEqual({ port: 9090 });
    // The port may have moved anyway: the listening state is refetched either way.
    for (const key of [proxyStatusKey, deviceAccessKey, terminalEnvKey]) {
      expect(invalidate).toHaveBeenCalledWith({ queryKey: key });
    }
    invalidate.mockRestore();
  });

  it("regenerating the CA refetches the CA and trust even when untrusting the old one fails", async () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    backend.regenerateCa.mockRejectedValue(new Error("keychain said no"));
    await expect(runMutation(mutations.regenerateCa, true)).rejects.toThrow();
    expect(invalidate).toHaveBeenCalledWith({ queryKey: caInfoKey });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: trustStatusKey });
    invalidate.mockRestore();
  });

  it("a write without an answer invalidates what it changed", async () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    backend.removeMockSource.mockResolvedValue(undefined);
    await runMutation(mutations.removeMockSource, "m1");
    expect(backend.removeMockSource).toHaveBeenCalledWith("m1");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: mockSourcesKey });
    invalidate.mockRestore();
  });

  it("a WebSocket send refetches that flow's message list", async () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    backend.wsSend.mockResolvedValue(undefined);
    const message = { direction: "sent", text: "hi" } as never;
    await runMutation(mutations.wsSend, { store: 1, id: 4, message });
    expect(backend.wsSend).toHaveBeenCalledWith(4, message);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: streamKey(1, 4) });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: wsPagesKey(1, 4) });
    invalidate.mockRestore();
  });
});
