import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import { useUi, type RequestTab, type ResponseTab } from "@/store/ui";
import "@/styles/index.css";

// Lets tokens.css switch the sidebar to vibrancy on macOS.
const platform = navigator.userAgent.includes("Mac") ? "macos" : "other";
document.documentElement.dataset.platform = platform;

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

// Dev only: `?theme=light`, `?select=<flow id>` and `?requestTab=Body` set up a
// view for the Playwright viewer snapshots (with `?zoo`, see lib/backend.ts).
if (import.meta.env.DEV) {
  const params = new URLSearchParams(window.location.search);
  const theme = params.get("theme");
  if (theme === "light" || theme === "dark") {
    document.documentElement.dataset.theme = theme;
    document.documentElement.classList.toggle("dark", theme === "dark");
  }
  const select = params.get("select");
  if (select !== null) useUi.getState().select(Number(select));
  const requestTab = params.get("requestTab") as RequestTab | null;
  if (requestTab) useUi.getState().setRequestTab(requestTab);
}

if (import.meta.env.DEV && import.meta.env.VITE_PM_REMOTE) {
  void import("@/dev/remote").then((m) => m.runRemote());
}

if (import.meta.env.DEV && import.meta.env.VITE_PM_PERF_PROBE) {
  void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
}
