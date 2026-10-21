import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import "@/styles/index.css";

// Lets tokens.css switch the sidebar to vibrancy on macOS.
const platform = navigator.userAgent.includes("Mac") ? "macos" : "other";
document.documentElement.dataset.platform = platform;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

if (import.meta.env.DEV && import.meta.env.VITE_PM_PERF_PROBE) {
  void import("@/dev/perfProbe").then((m) => m.runPerfProbe());
}
