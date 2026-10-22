import { expect, test } from "@playwright/test";

/**
 * The M7 MCP UI against the in-memory mock backend (one connected agent):
 * the toolbar indicator, Settings › MCP and its audit log. The server
 * itself is tested in Rust (crates/pm-mcp/tests).
 */

test("the toolbar counts connected agents and opens Settings › MCP", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("mcp-client-count")).toHaveText("1");
  await page.getByTestId("mcp-indicator").click();

  const settings = page.getByTestId("settings-mcp");
  await expect(settings.getByTestId("mcp-serving")).toContainText("Serving on http://127.0.0.1:9095/mcp");
  await expect(settings.getByTestId("mcp-clients")).toContainText("claude-code");
  await expect(settings.getByTestId("mcp-audit")).toContainText("create_map_local");
  // The token is masked on screen.
  await expect(settings).toContainText("Bearer ••••••••");
});

test("read-only and the enable switch take effect", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("settings-button").click();
  await page.getByTestId("settings-page-mcp").click();
  const settings = page.getByTestId("settings-mcp");

  await settings.getByTestId("mcp-read-only").click();
  await expect(settings.getByTestId("mcp-serving")).toContainText("read-only");

  await settings.getByTestId("mcp-enabled").click();
  await expect(settings.getByTestId("mcp-serving")).toContainText("Off");
  await expect(page.getByTestId("mcp-client-count")).toHaveCount(0);
});
