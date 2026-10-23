import { expect, test } from "@playwright/test";

import { openFixture } from "./zoo";

/**
 * The M14 scripts UI against the mock backend: the Scripts window, and a
 * view script's output shown automatically in the inspector. The mock runs
 * view scripts as browser modules; QuickJS and the rest of the API are
 * tested in Rust (pm-core).
 */

for (const theme of ["dark", "light"] as const) {
  test(`a view script decodes a base64 JSON field on its own (${theme})`, async ({ page }) => {
    const body = await openFixture(page, "json-base64", { theme });
    // The script's view is picked without a click…
    await expect(body.getByTestId("viewer-select")).toHaveText(/session\.payload/);
    const view = body.getByTestId("script-view");
    await expect(view.locator(".cm-content")).toContainText('"name": "Ada Lovelace"');
    await expect(view.locator(".cm-line").nth(1)).toBeVisible();
    await expect(body).toHaveScreenshot(`script-view-${theme}.png`);

    // …and the built-in viewers are still in the menu.
    await body.getByTestId("viewer-select").click();
    await expect(page.getByTestId("script-views")).toContainText("Scripts");
    await page.getByRole("option", { name: "JSON", exact: true }).click();
    await expect(body.getByTestId("script-view")).toHaveCount(0);
    await expect(body.locator(".cm-content")).toContainText('"payload": "eyJ1c2Vy');
  });

  test(`the Scripts window lists scripts by kind and edits one (${theme})`, async ({ page }) => {
    await page.goto(`/?scripts=mock-view-base64&theme=${theme}`);
    const window = page.getByTestId("scripts-window");
    await expect(window).toBeVisible();
    await expect(window.getByTestId("script-group-view")).toContainText("Decode base64 JSON field");
    await expect(window.getByTestId("script-group-mock")).toContainText("Checkout stub");
    await expect(window.getByTestId("rule-agent-badge")).toHaveText("AI · claude-code");
    const editor = window.getByTestId("script-editor");
    await expect(editor.locator(".cm-content")).toContainText("export function onView(context, flow)");
    await expect(window.getByTestId("view-settings")).toBeVisible();
    await expect(window).toHaveScreenshot(`scripts-window-${theme}.png`);
  });
}

test("a template becomes a saved script, with completions for the API", async ({ page }) => {
  await page.goto("/?scripts");
  const window = page.getByTestId("scripts-window");
  await window.getByTestId("new-script").first().click();
  await page.getByTestId("template-decode-jwt").click();
  await expect(window.getByTestId("script-name")).toHaveValue("Decode JWT");

  const content = window.getByTestId("script-editor").locator(".cm-content");
  await content.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("\nProxymate.base64D");
  await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("base64DecodeJson");
  await page.keyboard.press("Escape");

  const rows = window.getByTestId("script-row");
  const before = await rows.count();
  await window.getByTestId("save-script").click();
  await expect(rows).toHaveCount(before + 1);
  await expect(rows.filter({ hasText: "Decode JWT" })).toHaveAttribute("data-selected", "true");
});

test("the flow menu starts a view script for that flow", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  await page.getByTestId("menu-script").click();
  await page.getByTestId("menu-script-view").click();
  const window = page.getByTestId("scripts-window");
  await expect(window.getByTestId("script-name")).toHaveValue(/^View · api\.github\.com\//);
  await expect(window.getByTestId("matcher-host")).toHaveValue("api.github.com");
});
