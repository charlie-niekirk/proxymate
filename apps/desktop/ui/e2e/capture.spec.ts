import { expect, test } from "@playwright/test";

/**
 * M21's capture targets and noise against the in-memory mock backend: the
 * Intercept menu (nothing really starts), the No Caching and Block Cookies
 * toggles, "Never record this host", the footer's count and Settings ›
 * Storage. Launch command lines, the recording lists and both rules are
 * tested in Rust (`pm-platform` launcher tests, `crates/pm-core/tests/capture.rs`).
 */

test("the Intercept menu lists installed apps and shows what's running", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("intercept-menu").click();
  const menu = page.getByTestId("intercept-popover");
  await expect(menu.getByTestId("intercept-terminal")).toBeEnabled();
  await expect(menu.getByTestId("intercept-edge")).toBeDisabled();
  await expect(menu.getByTestId("intercept-edge")).toContainText("not installed");
  await menu.getByTestId("intercept-chrome").click();
  await expect(page.getByText("Started Google Chrome")).toBeVisible();
  await expect(page.getByTestId("intercept-menu")).toContainText("1");
  await page.getByTestId("intercept-menu").click();
  await expect(page.getByTestId("intercept-running")).toContainText("Google Chrome (Intercepted)");
});

test("No Caching and Block Cookies turn their built-in rules on and off", async ({ page }) => {
  await page.goto("/");
  const noCache = page.getByTestId("no-cache-toggle");
  const noCookies = page.getByTestId("no-cookies-toggle");
  await expect(noCache).toHaveAttribute("aria-pressed", "false");
  await noCache.click();
  await expect(noCache).toHaveAttribute("aria-pressed", "true");
  await expect(noCache).toContainText("No Caching");
  await noCookies.click();
  await expect(noCookies).toHaveAttribute("aria-pressed", "true");
  await noCache.click();
  await expect(noCache).toHaveAttribute("aria-pressed", "false");
  await expect(noCookies).toHaveAttribute("aria-pressed", "true");

  // Like the throttle, they're managed from the toolbar and palette, not listed in the Rules Editor.
  await page.getByTestId("rules-button").click();
  await expect(page.getByTestId("rules-editor").getByTestId("rule-row").filter({ hasText: "Block Cookies" })).toHaveCount(0);
});

test("Never record this host adds it to Settings › Storage and the footer counts", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("unrecorded-count")).toHaveCount(0);
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  const item = page.getByTestId("menu-never-record");
  await expect(item).toContainText("Never record api.github.com");
  await item.click();
  await expect(page.getByText("Not recording api.github.com")).toBeVisible();
  await page.getByTestId("unrecorded-count").click();
  const storage = page.getByTestId("settings-storage");
  await expect(storage.getByTestId("recording-never")).toContainText("api.github.com");
  await expect(storage.getByTestId("storage-unrecorded")).toContainText("1 flows");

  // Pasted URLs keep only their host.
  const only = storage.getByTestId("recording-only");
  await only.getByRole("textbox").fill("https://API.Example.com:8443/v1");
  await only.getByRole("button", { name: "Add" }).click();
  await expect(only).toContainText("api.example.com");
});

test("auto-save is set up in Settings › Storage and saves on demand", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByTestId("settings-page-storage").click();
  const storage = page.getByTestId("settings-storage");
  await storage.getByTestId("autosave-enabled").click();
  await expect(storage.getByTestId("settings-error")).toContainText("choose a folder");
  await storage.getByRole("button", { name: "Choose…" }).click();
  await expect(storage.getByTestId("autosave-folder")).toHaveValue("/Users/you/Documents/Proxymate");
  await storage.getByTestId("autosave-keep").fill("3");
  await storage.getByTestId("autosave-save").click();
  await storage.getByTestId("autosave-enabled").click();
  await expect(storage.getByTestId("autosave-enabled")).toBeChecked();
  await storage.getByTestId("autosave-now").click();
  await expect(storage.getByTestId("autosave-status")).toContainText("Proxymate Autosave");
});
