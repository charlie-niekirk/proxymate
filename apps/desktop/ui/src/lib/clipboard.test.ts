import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast }));

import { copy, copyBodyText, copyLater } from "@/lib/clipboard";

describe("clipboard", () => {
  const writeText = vi.fn<(text: string) => Promise<void>>();

  beforeEach(() => {
    writeText.mockReset().mockResolvedValue(undefined);
    toast.success.mockReset();
    toast.error.mockReset();
    vi.stubGlobal("navigator", { clipboard: { writeText } });
  });
  afterEach(() => vi.unstubAllGlobals());

  it("copies and confirms", async () => {
    expect(await copy("abc", "header value")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("abc");
    expect(toast.success).toHaveBeenCalledWith("Copied header value", expect.objectContaining({ duration: 1200 }));
  });

  it("stays quiet when asked, for buttons with their own tick", async () => {
    expect(await copy("abc", "x", { silent: true })).toBe(true);
    expect(toast.success).not.toHaveBeenCalled();
  });

  it("explains a refused write", async () => {
    writeText.mockRejectedValue(new Error("Write permission denied."));
    expect(await copy("abc")).toBe(false);
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.error).toHaveBeenCalledWith("Couldn't copy", expect.objectContaining({ description: "Write permission denied." }));
  });

  it("copies fetched text, and explains a failed fetch", async () => {
    expect(await copyLater(async () => "curl …", "cURL command")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("curl …");

    expect(await copyLater(() => Promise.reject(new Error("no detail")))).toBe(false);
    expect(toast.error).toHaveBeenCalledWith("Couldn't copy", expect.objectContaining({ description: "no detail" }));

    writeText.mockClear();
    expect(await copyLater(async () => null)).toBe(false);
    expect(writeText).not.toHaveBeenCalled();
  });

  it("copies text bodies, and says why not for binary ones", async () => {
    expect(await copyBodyText(new TextEncoder().encode('{"a":1}'), "body")).toBe(true);
    expect(writeText).toHaveBeenCalledWith('{"a":1}');

    expect(await copyBodyText(new Uint8Array([0x89, 0x50, 0x00, 0x01]), "payload")).toBe(false);
    expect(toast.error).toHaveBeenCalledWith(
      "Couldn't copy",
      expect.objectContaining({ description: "The payload isn't UTF-8 text: copy it as Base64 instead" }),
    );
  });
});
