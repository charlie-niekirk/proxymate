import { isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import { NoEngine } from "@/components/NoEngine";
import { startDevHarness } from "@/dev/harness";
import { startTheme } from "@/lib/theme";
import "@/styles/index.css";

// Lets tokens.css switch the sidebar to vibrancy in the macOS app (a plain
// browser has no vibrancy behind the page, so it keeps the solid colour).
const platform = isTauri() && navigator.userAgent.includes("Mac") ? "macos" : "other";
document.documentElement.dataset.platform = platform;

// Dev only: `?theme=light` pins a theme for the Playwright snapshots.
const pinnedTheme = import.meta.env.DEV ? new URLSearchParams(window.location.search).get("theme") : null;
// Before the first render, so the first paint already has the right theme.
startTheme(pinnedTheme === "light" || pinnedTheme === "dark" ? pinnedTheme : null);

// A release build outside the app has no engine (lib/backend.ts): say so
// instead of rendering an app whose every call fails.
const hasEngine = isTauri() || import.meta.env.DEV;

createRoot(document.getElementById("root")!).render(
  <StrictMode>{hasEngine ? <App /> : <NoEngine />}</StrictMode>,
);

// Dev only: env vars and URL parameters that set up a screen, the remote
// and the perf probe (dev/harness.ts). Dropped from release builds.
if (import.meta.env.DEV) startDevHarness();
