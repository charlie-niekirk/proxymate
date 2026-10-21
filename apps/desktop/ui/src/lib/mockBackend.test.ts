import { describe, expect, it } from "vitest";

import type { FilterSpec_Deserialize, SidebarSnapshot_Deserialize } from "@/bindings";

import { createMockBackend } from "./mockBackend";

const all: FilterSpec_Deserialize = { text: "", protocol: "all", content: [], host: null, client: null };

describe("mock backend", () => {
  it("pushes a sidebar that adds up", async () => {
    const backend = createMockBackend(1_000);
    const snapshot = await new Promise<SidebarSnapshot_Deserialize>((resolve) => void backend.subscribeSidebar(resolve));
    expect(snapshot.total).toBe(1_000);
    expect(snapshot.apps.reduce((n, a) => n + a.count, 0)).toBe(1_000);
    expect(snapshot.domains.reduce((n, d) => n + d.count, 0)).toBe(1_000);
    expect(snapshot.domains.find((d) => d.domain === "github.com")?.hosts).toEqual([{ host: "api.github.com", count: 167 }]);
  });

  it("filters by app and by domain like the engine", async () => {
    const backend = createMockBackend(1_000);
    const safari = await backend.createView({ ...all, client: "Safari" }, { key: "id", dir: "asc" });
    expect(safari.len).toBe(200);
    const github = await backend.createView({ ...all, host: "*.github.com" }, { key: "id", dir: "asc" });
    const exact = await backend.createView({ ...all, host: "api.github.com" }, { key: "id", dir: "asc" });
    expect(github.len).toBe(exact.len);
    expect(github.len).toBeGreaterThan(0);
  });

  it("excludes apps from decryption case-insensitively", async () => {
    const backend = createMockBackend(10);
    expect((await backend.setAppSslExcluded("com.apple.Safari", true)).exclude_apps).toEqual(["com.apple.Safari"]);
    expect((await backend.setAppSslExcluded("COM.APPLE.SAFARI", false)).exclude_apps).toEqual([]);
  });
});
