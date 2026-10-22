import { expect, test } from "@playwright/test";

/**
 * Settings › Updates (M11) against the mock backend. The updater itself
 * (feed, signature check, install) runs in Rust; see docs/RELEASING.md.
 */
test("the launch check can be turned off, and stays off", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("settings-button").click();
  await page.getByTestId("settings-page-updates").click();

  const updates = page.getByTestId("settings-updates");
  await expect(updates).toContainText("Proxymate 0.1.0");
  const launch = updates.getByTestId("update-check-at-launch");
  await expect(launch).toHaveAttribute("aria-checked", "true");
  await launch.click();
  await expect(launch).toHaveAttribute("aria-checked", "false");

  // Leaving and coming back reads the saved setting.
  await page.getByTestId("settings-page-mcp").click();
  await page.getByTestId("settings-page-updates").click();
  await expect(updates.getByTestId("update-check-at-launch")).toHaveAttribute("aria-checked", "false");
});
