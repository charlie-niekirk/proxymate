//! The dev harness (R4): everything a dev build sets up from its env vars
//! and URL parameters, in one place. `main.tsx` calls it only in dev, so
//! release builds drop it. Editors register their buttons for the remote
//! with `useBridge` (dev/bridge.ts).

import { useUi, type DeviceTab, type RequestTab, type ResponseTab, type SettingsPage } from "@/store/ui";

export function startDevHarness() {
  // start with Auto Select on, so new traffic is inspected without a
  // click (used to verify the real app from a shell).
  if (import.meta.env.VITE_PM_AUTO_SELECT) {
    useUi.getState().setAutoScroll(true);
  }

  // open the response pane on a given tab (e.g. `TLS`), for
  // screenshots of the real app.
  const responseTab = import.meta.env.VITE_PM_RESPONSE_TAB as ResponseTab | undefined;
  if (responseTab) {
    useUi.getState().setResponseTab(responseTab);
  }

  // `?select=<flow id>` and `?requestTab=Body` set up a
  // view for the Playwright viewer snapshots (with `?zoo`, see lib/backend.ts).
  {
    const params = new URLSearchParams(window.location.search);
    const select = params.get("select");
    if (select !== null) useUi.getState().select(Number(select));
    const requestTab = params.get("requestTab") as RequestTab | null;
    if (requestTab) useUi.getState().setRequestTab(requestTab);
    if (params.has("rules")) useUi.getState().openRulesEditor();
    const rewrite = params.get("rewrite");
    if (rewrite !== null) useUi.getState().openRewrite({ store: 0, flow: Number(rewrite) });
    if (params.has("scripts")) useUi.getState().openScripts({ scriptId: params.get("scripts") || null });
  }

  if (import.meta.env.VITE_PM_REMOTE) {
    void import("@/dev/remote").then((m) => m.runRemote());
  }

  // start with this search text (e.g. a host), for screenshots.
  const filterText = import.meta.env.VITE_PM_FILTER as string | undefined;
  if (filterText) {
    useUi.getState().setText(filterText);
  }

  // open the certificate wizard, the Rules Editor or the Breakpoint
  // Editor or device setup at launch, for screenshots
  // (`VITE_PM_OPEN=cert-wizard|rules|breakpoints|device-setup:<tab>|settings:mcp`).
  if (import.meta.env.VITE_PM_OPEN === "cert-wizard") {
    useUi.getState().setCertWizardOpen(true);
  }
  if (import.meta.env.VITE_PM_OPEN === "rules") {
    useUi.getState().openRulesEditor();
  }
  if (import.meta.env.VITE_PM_OPEN === "scripts") {
    useUi.getState().openScripts();
  }
  if (import.meta.env.VITE_PM_OPEN === "breakpoints") {
    useUi.getState().setBreakpointsOpen(true);
  }
  const openDevices = import.meta.env.VITE_PM_OPEN as string | undefined;
  if (openDevices?.startsWith("settings")) {
    useUi.getState().openSettings((openDevices.split(":")[1] as SettingsPage | undefined) ?? "mcp");
  }
  if (openDevices?.startsWith("device-setup")) {
    const tab = openDevices.split(":")[1] as DeviceTab | undefined;
    if (tab) useUi.getState().setDeviceTab(tab);
    useUi.getState().setDeviceSetupOpen(true);
  }

  if (import.meta.env.VITE_PM_PERF_PROBE) {
    void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
  }
}
