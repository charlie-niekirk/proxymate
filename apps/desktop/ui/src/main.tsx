import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import { useUi, type ResponseTab } from "@/store/ui";
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

if (import.meta.env.DEV && import.meta.env.VITE_PM_PERF_PROBE) {
  void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
}
