import { expect, test } from "@playwright/test";

/**
 * The M8 UI against the in-memory mock backend: Edit & Replay and Compose,
 * and documents opening in their own tabs. Sending, cURL, HAR and sessions
 * themselves are tested in Rust (pm-core).
 */

test("Edit & Replay opens Compose pre-filled, and Send selects the new flow", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  await page.getByTestId("menu-edit-replay").click();

  const compose = page.getByTestId("compose");
  await expect(compose).toBeVisible();
  await expect(compose.getByTestId("compose-method")).toHaveValue("GET");
  await expect(compose.getByTestId("compose-url")).toHaveValue(/^https:\/\/api\.github\.com\/v1\/items\/0/);
  // Host and Content-Length are derived when sending, so they aren't offered.
  await expect(compose.getByTestId("compose-headers").locator("input").first()).toHaveValue("user-agent");

  await compose.getByTestId("compose-method").fill("POST");
  await compose.getByTestId("compose-send").click();
  await expect(compose).toBeHidden();
  const inspector = page.getByTestId("inspector");
  await expect(inspector.getByTestId("sent-badge")).toHaveText("Replay");
  await expect(inspector).toContainText("POST");
});

test("Compose sends a new request with a body", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("compose-button").click();
  const compose = page.getByTestId("compose");
  await compose.getByTestId("compose-url").fill("https://example.test/notes");
  await compose.getByTestId("compose-method").fill("put");
  await compose.getByTestId("compose-type").click();
  await page.getByRole("option", { name: "JSON" }).click();
  await expect(compose.getByTestId("compose-headers").locator("input").last()).toHaveValue("application/json");
  await compose.locator(".cm-content").click();
  await page.keyboard.type('{"note": 1}');
  await page.keyboard.press("ControlOrMeta+Enter");
  await expect(compose).toBeHidden();
  await expect(page.getByTestId("inspector")).toContainText("example.test/notes");
});

test("an opened document gets a tab and a Saved entry, and closes", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("document-tabs")).toHaveCount(0);
  await page.getByText("Open session or HAR…").click();

  const tabs = page.getByTestId("document-tabs");
  await expect(tabs).toBeVisible();
  await expect(page.getByTestId("tab-1")).toHaveAttribute("data-active", "true");
  await expect(page.getByTestId("saved-1")).toContainText("mock.proxymate");
  await expect(page.getByTestId("row-count")).toContainText("50 of");

  await page.getByTestId("tab-live").click();
  await expect(page.getByTestId("row-count")).toContainText("100,000 of");
  await page.getByTestId("close-1").click();
  await expect(page.getByTestId("document-tabs")).toHaveCount(0);
});
