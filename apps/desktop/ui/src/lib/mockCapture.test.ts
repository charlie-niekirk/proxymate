import { describe, expect, it } from "vitest";

import { createMockCapture, normalizeHost } from "@/lib/mockCapture";

describe("normalizeHost", () => {
  // The same cases as `capture::tests::hosts_are_normalized_before_they_are_stored`.
  it("keeps only the host, lower-cased", () => {
    expect(normalizeHost(" https://Example.COM:8443/path?q ")).toBe("example.com");
    expect(normalizeHost("user:pw@api.example.com")).toBe("api.example.com");
    expect(normalizeHost("*.Apple.com")).toBe("*.apple.com");
    expect(normalizeHost("[::1]:9090")).toBe("::1");
    expect(normalizeHost("::1")).toBe("::1");
    expect(normalizeHost("   ")).toBe("");
  });
});

describe("mock capture", () => {
  it("adds never-recorded hosts once and refuses spaces", async () => {
    let changes = 0;
    const capture = createMockCapture(() => changes++);
    await capture.methods.neverRecordHost("A.com");
    const lists = await capture.methods.neverRecordHost("a.com");
    expect(lists.never).toEqual(["a.com"]);
    expect(changes).toBe(2);
    await expect(capture.methods.setRecordingSettings({ only: ["a b.com"], never: [] })).rejects.toThrow("no spaces");
  });

  it("starts only installed apps", async () => {
    const capture = createMockCapture(() => {});
    const session = await capture.methods.intercept("chrome");
    expect(session.name).toBe("Google Chrome");
    await expect(capture.methods.intercept("edge")).rejects.toThrow("isn't installed");
    expect(await capture.methods.listIntercepts()).toHaveLength(1);
  });
});
