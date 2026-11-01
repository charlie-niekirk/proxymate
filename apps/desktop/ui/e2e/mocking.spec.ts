import { expect, test } from "@playwright/test";

/**
 * M20's offline mocking UI against the in-memory mock backend: "Mock from
 * session…" opens a Serve from rule, the mock library and OpenAPI specs in
 * Settings, generated mocks, and the inspector's operation line. Serving,
 * Charles import and the Petstore spec are tested in Rust
 * (`crates/pm-core/tests/mocking.rs`).
 */

test("mock from a session tab opens a Serve from rule", async ({ page }) => {
  await page.goto("/");
  await page.getByText("Open session or HAR…").click();
  await expect(page.getByTestId("tab-1")).toHaveAttribute("data-active", "true");
  await page.getByTestId("tab-1").click({ button: "right" });
  await page.getByTestId("tab-mock-from").click();

  const editor = page.getByTestId("rules-editor");
  await expect(editor.getByTestId("rule-name")).toHaveValue("Serve from mock");
  await expect(editor.getByTestId("serve-source")).toContainText("mock · 40 flows");
  await expect(editor.getByTestId("serve-key-query")).toBeChecked();
  await expect(editor.getByTestId("serve-key-body")).not.toBeChecked();
  await editor.getByTestId("serve-miss").click();
  await page.getByRole("option", { name: "Answer 404" }).click();
  await editor.getByTestId("save-rule").click();
  await expect(editor.getByTestId("rule-row").filter({ hasText: "Serve from mock" })).toContainText("serve from mock, else 404");
});

test("the library and specs live in Settings, and Generate mocks adds rules turned off", async ({ page }) => {
  await page.goto("/?mocks&openapi");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByTestId("settings-page-mocking").click();
  const settings = page.getByTestId("settings-mocking");
  await expect(settings.getByTestId("mock-source")).toContainText("checkout-recording");
  await expect(settings.getByTestId("openapi-spec")).toContainText("Items API");

  // A recording a rule serves from can't be removed.
  await settings.getByTestId("mock-from-file").click();
  const editor = page.getByTestId("rules-editor");
  await editor.getByTestId("save-rule").click();
  await editor.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByTestId("settings-page-mocking").click();
  await settings.getByRole("button", { name: "Remove recording" }).click();
  await expect(settings).toContainText("is used by");

  await settings.getByTestId("generate-mocks").click();
  await expect(editor).toBeVisible();
  const rows = editor.getByTestId("rule-row").filter({ hasText: "Items API" });
  await expect(rows).toHaveCount(2);
  for (const row of await rows.all()) await expect(row.getByRole("checkbox")).not.toBeChecked();
  await expect(editor.getByTestId("rule-name")).toHaveValue("getItem · Items API");
});

test("a flow that matches an imported operation shows it", async ({ page }) => {
  await page.goto("/?openapi&select=0");
  const op = page.getByTestId("flow-operation");
  await expect(op).toContainText("getItem");
  await expect(op).toContainText("Fetch an item");
  await expect(op).toContainText("/v1/items/{itemId} · Items API");
});
