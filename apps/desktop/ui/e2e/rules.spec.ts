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

test("a rule turns on and off from its checkbox in the list, which the form's Save keeps", async ({ page }) => {
  await page.goto("/?rules");
  const editor = page.getByTestId("rules-editor");
  const row = editor.getByTestId("rule-row").first();
  const toggle = row.getByTestId("rule-enabled");
  await row.click();
  const form = editor.getByTestId("rule-form");
  await expect(form).toBeVisible();
  // On/off lives in the list only.
  await expect(form.getByRole("checkbox", { name: "Enabled" })).toHaveCount(0);

  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(row).toHaveAttribute("data-selected", "true");
  await expect(form.getByTestId("rule-disabled")).toBeVisible();
  await expect(form).not.toContainText("Unsaved");

  // Saving an edit doesn't turn it back on.
  await form.getByTestId("rule-name").fill("Renamed rule");
  await form.getByTestId("save-rule").click();
  await expect(row).toContainText("Renamed rule");
  await expect(toggle).not.toBeChecked();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(form.getByTestId("rule-disabled")).toHaveCount(0);
});

test("Map Local's file and directory come from the OS picker", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  await page.getByTestId("menu-map-local").click();
  const editor = page.getByTestId("rules-editor");
  const actions = editor.getByTestId("action-list");
  await actions.getByTestId("map-local-source").click();
  await page.getByRole("option", { name: "A file" }).click();
  // The mock backend answers the dialog with a plausible pick.
  await editor.getByTestId("map-local-file-choose").click();
  await expect(editor.getByTestId("map-local-file")).toHaveValue("/Users/you/mock.json");

  await actions.getByTestId("map-local-source").click();
  await page.getByRole("option", { name: "A directory" }).click();
  await editor.getByTestId("map-local-root-choose").click();
  await expect(editor.getByTestId("map-local-root")).toHaveValue("/Users/you/site");
});

test("a shaky press on a rule's checkbox toggles it, and rows still drag to reorder", async ({ page }) => {
  await page.goto("/");
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  await page.getByTestId("menu-map-local").click();
  const editor = page.getByTestId("rules-editor");
  await editor.getByTestId("save-rule").click();
  const rows = editor.getByTestId("rule-row");
  await expect(rows).toHaveCount(2);

  // WebKit drops the click if the press starts a row drag, even a cancelled one.
  const toggle = rows.first().getByTestId("rule-enabled");
  await expect(toggle).toBeChecked();
  const box = (await toggle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  for (const dx of [2, 3, 4, 5]) await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2 + 1);
  await page.mouse.up();
  await expect(toggle).not.toBeChecked();

  const first = (await rows.first().innerText()).split("\n")[0] ?? "";
  expect(first).not.toBe("");
  const from = (await rows.first().boundingBox())!;
  const to = (await rows.last().boundingBox())!;
  await page.mouse.move(from.x + 60, from.y + from.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(from.x + 60, from.y + from.height / 2 + ((to.y - from.y) * i) / 10);
  await page.mouse.up();
  await expect(rows.last()).toContainText(first);
  await expect(rows.first()).not.toContainText(first);
});
