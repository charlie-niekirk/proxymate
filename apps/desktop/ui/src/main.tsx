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

// Dev only: start with this search text (e.g. a host), for screenshots.
const filterText = import.meta.env.DEV ? (import.meta.env.VITE_PM_FILTER as string | undefined) : undefined;
if (filterText) {
  useUi.getState().setText(filterText);
}

// Dev only: open the certificate wizard at launch, for screenshots.
if (import.meta.env.DEV && import.meta.env.VITE_PM_OPEN === "cert-wizard") {
  useUi.getState().setCertWizardOpen(true);
}

if (import.meta.env.DEV && import.meta.env.VITE_PM_PERF_PROBE) {
  void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
}
