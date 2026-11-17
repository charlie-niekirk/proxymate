import { beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => {
  // jsdom has no matchMedia; theme.ts reads it at import. The OS is dark.
  window.matchMedia = ((query: string) => ({
    matches: true,
    media: query,
    addEventListener() {},
    removeEventListener() {},
  })) as unknown as typeof window.matchMedia;
  return { setWindowTheme: vi.fn<(theme: "light" | "dark" | null) => Promise<void>>(() => Promise.resolve()) };
});
vi.mock("@/lib/backend", () => ({ backend }));

import { forceTheme, startTheme } from "@/lib/theme";
import { usePrefs } from "@/store/prefs";

const page = () => document.documentElement.dataset.theme;
const windowThemes = () => backend.setWindowTheme.mock.calls.map(([theme]) => theme);

describe("theme", () => {
  beforeEach(() => backend.setWindowTheme.mockClear());

  it("keeps the native window in step with Appearance and a forced theme", () => {
    usePrefs.getState().setAppearance("system");
    startTheme();
    // System: the window follows macOS (null); the page follows it too.
    expect(page()).toBe("dark");
    expect(windowThemes()).toEqual([null]);

    usePrefs.getState().setAppearance("light");
    expect(page()).toBe("light");
    usePrefs.getState().setAppearance("dark");
    expect(page()).toBe("dark");
    usePrefs.getState().setAppearance("system");
    expect(windowThemes()).toEqual([null, "light", "dark", null]);

    // The dev-only pin paints the window as well as the page, so the
    // sidebar's vibrancy matches on a Mac whose appearance differs.
    forceTheme("light");
    expect(page()).toBe("light");
    expect(windowThemes()).toEqual([null, "light", "dark", null, "light"]);
    forceTheme("light");
    expect(windowThemes()).toHaveLength(5);
  });
});
