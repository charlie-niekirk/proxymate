import { defineConfig, devices } from "@playwright/test";

/**
 * Viewer snapshots and preview-safety checks against the UI in a browser,
 * with the mock backend serving the zoo fixtures (`?zoo`). WebKit is the
 * engine the app runs on (WKWebView, WebKitGTK); Chromium also runs the
 * safety checks. Snapshot baselines are per platform: macOS ones are made
 * locally, Linux ones in CI's Playwright Docker image (a missing baseline
 * is written there and comes back in the playwright-report artifact).
 * PM_E2E_IGNORE_SNAPSHOTS turns the pixel diffs off.
 */
const port = Number(process.env.PM_E2E_PORT ?? 1425);

export default defineConfig({
  testDir: "e2e",
  snapshotPathTemplate: "e2e/__snapshots__/{arg}-{projectName}-{platform}{ext}",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  ignoreSnapshots: !!process.env.PM_E2E_IGNORE_SNAPSHOTS,
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.002, animations: "disabled", caret: "hide" } },
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: { width: 1440, height: 1100 },
    deviceScaleFactor: 1,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 1440, height: 1100 }, deviceScaleFactor: 1 } },
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 1100 }, deviceScaleFactor: 1 },
      grep: /@security/,
    },
  ],
  webServer: {
    command: `npm run dev -- --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
