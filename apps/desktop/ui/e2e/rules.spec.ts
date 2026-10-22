import { expect, test } from "@playwright/test";

/**
 * The M5 rules UI against the in-memory mock backend: presets from the flow
 * menu, the Rules Editor, the network conditions menu and the Breakpoint
 * Editor. The engine side is tested in Rust (pm-core).
 */

test("Map Local… from the flow menu opens a draft that saves and dry-runs", async ({ page }) => {
  await page.goto("/");
  const row = page.locator('[data-flow-id="0"]');
  await row.click({ button: "right" });
  await expect(page.getByTestId("flow-menu")).toBeVisible();
  await page.getByTestId("menu-map-local").click();

  const editor = page.getByTestId("rules-editor");
  await expect(editor).toBeVisible();
  await expect(editor.getByTestId("rule-name")).toHaveValue(/^Map Local · GET api\.github\.com/);
  await expect(editor.getByTestId("matcher-host")).toHaveValue("api.github.com");

  await editor.getByTestId("test-rule").click();
  await expect(editor.getByTestId("rule-test-result")).toContainText("The rule matches this flow");

  const rows = editor.getByTestId("rule-row");
  const before = await rows.count();
  await editor.getByTestId("save-rule").click();
  await expect(rows).toHaveCount(before + 1);
  await expect(editor.getByTestId("rule-row").filter({ hasText: "Map Local · GET api.github.com" })).toHaveAttribute("data-selected", "true");
});

test("agent rules carry the AI badge and can be removed together", async ({ page }) => {
  await page.goto("/?rules");
  const editor = page.getByTestId("rules-editor");
  await expect(editor.getByTestId("rule-agent-badge")).toHaveText("AI · claude-code");
  await editor.getByTestId("remove-agent-rules").click();
  await expect(editor.getByTestId("rule-agent-badge")).toHaveCount(0);
});

test("the throttle menu switches the global network conditions", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("throttle-menu").click();
  await page.getByRole("button", { name: /^3G/ }).click();
  await expect(page.getByTestId("throttle-menu")).toHaveText("3G");
  await page.getByTestId("throttle-menu").click();
  await page.getByRole("button", { name: /^Off/ }).click();
  await expect(page.getByTestId("throttle-menu")).toHaveText("");
});

test("a paused flow opens the Breakpoint Editor and Execute lets it go", async ({ page }) => {
  await page.goto("/?breakpoint");
  const editor = page.getByTestId("breakpoint-editor");
  await expect(editor).toBeVisible();
  await expect(page.getByTestId("breakpoint-badge")).toContainText("1");
  await expect(editor.getByTestId("bp-url")).toHaveValue("https://api.stripe.com/v1/charges?expand=customer");
  // The body is in CodeMirror, highlighted as its Content-Type.
  const body = editor.getByTestId("bp-body");
  await expect(body).toContainText('"amount": 4200');
  await expect(body).toHaveAttribute("data-language", "json");
  // Token spans appear once the lazily loaded JSON grammar is in.
  await expect(body.locator(".cm-line span").first()).toBeVisible();
  await body.locator(".cm-content").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" ");
  await editor.getByTestId("bp-respond").click();
  await expect(editor.getByTestId("bp-respond-body")).toHaveAttribute("data-language", "json");
  await editor.getByTestId("bp-execute").click();
  await expect(editor).toBeHidden();
  await expect(page.getByTestId("breakpoint-badge")).not.toContainText("1");
});
