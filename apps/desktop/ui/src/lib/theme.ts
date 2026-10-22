import { backend } from "@/lib/backend";
import { usePrefs, type Appearance } from "@/store/prefs";

const darkQuery = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

/** The theme an appearance setting resolves to right now. */
export function resolveTheme(appearance: Appearance): "light" | "dark" {
  if (appearance !== "system") return appearance;
  return darkQuery?.matches === false ? "light" : "dark";
}

function paint(theme: "light" | "dark") {
  const root = document.documentElement;
  if (root.dataset.theme === theme) return;
  root.dataset.theme = theme;
  root.classList.toggle("dark", theme === "dark");
}

let forced: "light" | "dark" | null = null;

/**
 * Keeps `<html data-theme>` in step with the Appearance setting, and with
 * the OS while it is "System". The native window follows too, so the
 * sidebar's vibrancy and the traffic lights match the panes.
 * `force` pins a theme for this page (Playwright snapshots, `?theme=`).
 */
export function startTheme(force: "light" | "dark" | null = null) {
  forced = force;
  const apply = () => {
    const appearance = usePrefs.getState().appearance;
    paint(forced ?? resolveTheme(appearance));
  };
  apply();
  const syncWindow = (appearance: Appearance) => {
    if (forced) return;
    backend.setWindowTheme(appearance === "system" ? null : appearance).catch(() => {
      // Outside the app (a browser) there is no native window.
    });
  };
  syncWindow(usePrefs.getState().appearance);
  usePrefs.subscribe((s, prev) => {
    if (s.appearance === prev.appearance) return;
    syncWindow(s.appearance);
    apply();
  });
  darkQuery?.addEventListener("change", apply);
}

/** Dev only: switch the theme for this page without saving it. */
export function forceTheme(theme: "light" | "dark") {
  forced = theme;
  paint(theme);
}
