import { isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import { startTheme } from "@/lib/theme";
import { useUi, type DeviceTab, type RequestTab, type ResponseTab, type SettingsPage } from "@/store/ui";
import "@/styles/index.css";

// Lets tokens.css switch the sidebar to vibrancy in the macOS app (a plain
// browser has no vibrancy behind the page, so it keeps the solid colour).
const platform = isTauri() && navigator.userAgent.includes("Mac") ? "macos" : "other";
document.documentElement.dataset.platform = platform;

// Dev only: `?theme=light` pins a theme for the Playwright snapshots.
const pinnedTheme = import.meta.env.DEV ? new URLSearchParams(window.location.search).get("theme") : null;
// Before the first render, so the first paint already has the right theme.
startTheme(pinnedTheme === "light" || pinnedTheme === "dark" ? pinnedTheme : null);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Dev only: start with Auto Select on, so new traffic is inspected without a
// click (used to verify the real app from a shell).
if (import.meta.env.DEV && import.meta.env.VITE_PM_AUTO_SELECT) {
  useUi.getState().setAutoScroll(true);
}

// Dev only: open the response pane on a given tab (e.g. `TLS`), for
// screenshots of the real app.
const responseTab = import.meta.env.DEV ? (import.meta.env.VITE_PM_RESPONSE_TAB as ResponseTab | undefined) : undefined;
if (responseTab) {
  useUi.getState().setResponseTab(responseTab);
}

// Dev only: `?select=<flow id>` and `?requestTab=Body` set up a
// view for the Playwright viewer snapshots (with `?zoo`, see lib/backend.ts).
if (import.meta.env.DEV) {
  const params = new URLSearchParams(window.location.search);
  const select = params.get("select");
  if (select !== null) useUi.getState().select(Number(select));
  const requestTab = params.get("requestTab") as RequestTab | null;
  if (requestTab) useUi.getState().setRequestTab(requestTab);
  if (params.has("rules")) useUi.getState().openRulesEditor();
  const rewrite = params.get("rewrite");
  if (rewrite !== null) useUi.getState().openRewrite({ store: 0, flow: Number(rewrite) });
}

if (import.meta.env.DEV && import.meta.env.VITE_PM_REMOTE) {
  void import("@/dev/remote").then((m) => m.runRemote());
}

// Dev only: start with this search text (e.g. a host), for screenshots.
const filterText = import.meta.env.DEV ? (import.meta.env.VITE_PM_FILTER as string | undefined) : undefined;
if (filterText) {
  useUi.getState().setText(filterText);
}

// Dev only: open the certificate wizard, the Rules Editor or the Breakpoint
// Editor or device setup at launch, for screenshots
// (`VITE_PM_OPEN=cert-wizard|rules|breakpoints|device-setup:<tab>|settings:mcp`).
if (import.meta.env.DEV && import.meta.env.VITE_PM_OPEN === "cert-wizard") {
  useUi.getState().setCertWizardOpen(true);
}
if (import.meta.env.DEV && import.meta.env.VITE_PM_OPEN === "rules") {
  useUi.getState().openRulesEditor();
}
if (import.meta.env.DEV && import.meta.env.VITE_PM_OPEN === "breakpoints") {
  useUi.getState().setBreakpointsOpen(true);
}
const openDevices = import.meta.env.DEV ? (import.meta.env.VITE_PM_OPEN as string | undefined) : undefined;
if (openDevices?.startsWith("settings")) {
  useUi.getState().openSettings((openDevices.split(":")[1] as SettingsPage | undefined) ?? "mcp");
}
if (openDevices?.startsWith("device-setup")) {
  const tab = openDevices.split(":")[1] as DeviceTab | undefined;
  if (tab) useUi.getState().setDeviceTab(tab);
  useUi.getState().setDeviceSetupOpen(true);
}

if (import.meta.env.DEV && import.meta.env.VITE_PM_PERF_PROBE) {
  void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
}
