import { expect, test } from "@playwright/test";

/**
 * M19's WebSocket UI against the in-memory mock backend: the composer,
 * Resend, message flags and the original of an edited message, a held
 * message in the Breakpoint Editor, and the rule action. The engine side is
 * tested in Rust (`crates/pm-core/tests/ws_rules.rs`).
 */

test("the composer sends both ways and edited messages show their original", async ({ page }) => {
  // Flow 14 is an open mock WebSocket.
  await page.goto("/?select=13");
  const panel = page.getByTestId("websocket-panel");
  await expect(panel.getByTestId("message-count")).toContainText("40 messages");

  const composer = panel.getByTestId("ws-composer");
  await composer.getByTestId("ws-compose-input").fill('{"type":"ping"}');
  await composer.getByTestId("ws-compose-send").click();
  await expect(panel.getByTestId("message-count")).toContainText("41 messages");
  await expect(panel.getByTestId("ws-flag-injected").last()).toBeVisible();

  // Hex goes as binary, to the client; bad hex says why.
  await composer.getByTestId("ws-compose-to-client").click();
  await composer.getByTestId("ws-compose-hex").click();
  await composer.getByTestId("ws-compose-input").fill("zz");
  await composer.getByTestId("ws-compose-send").click();
  await expect(composer.getByTestId("ws-compose-error")).toContainText("isn't hex");
  await composer.getByTestId("ws-compose-input").fill("00 01 ff");
  await composer.getByTestId("ws-compose-send").click();
  await expect(panel.getByTestId("message-count")).toContainText("42 messages");

  // Resend from a row's menu.
  const last = panel.getByTestId("ws-row").last();
  await last.click({ button: "right" });
  await page.getByTestId("ws-resend").click();
  await expect(panel.getByTestId("message-count")).toContainText("43 messages");

  // Message 8 was edited by a rule: Original shows what arrived.
  const list = panel.getByTestId("ws-row").first().locator("xpath=ancestor::div[contains(@class,'overflow-auto')]");
  await list.evaluate((el) => (el.scrollTop = 0));
  await panel.getByTestId("ws-flag-edited").click();
  const detail = panel.getByTestId("ws-message-detail");
  await expect(detail).toContainText('"price": 107');
  await detail.getByTestId("ws-show-original").click();
  await expect(detail).toContainText('"price": 4.2');
  await expect(panel.getByTestId("ws-flag-dropped")).toBeVisible();

  await panel.getByTestId("ws-close").click();
  await expect(panel.getByTestId("ws-composer")).toBeHidden();
});

test("a held message is edited as hex and sent", async ({ page }) => {
  await page.goto("/?breakpoint=ws");
  const editor = page.getByTestId("breakpoint-editor");
  const form = editor.getByTestId("message-form");
  await expect(form).toContainText("From the server · Text");
  // JSON starts as a tree.
  await expect(form.getByTestId("bp-message-json")).toBeVisible();
  await form.getByTestId("bp-message-hex").click();
  await expect(form.getByTestId("bp-message-hex-input")).toHaveValue(/^7b 0a/);
  await form.getByTestId("bp-message-hex-input").fill("not hex");
  await form.getByTestId("bp-send").click();
  await expect(form).toContainText("isn't hex");
  await form.getByTestId("bp-message-hex-input").fill("7b 7d");
  await form.getByTestId("bp-send").click();
  await expect(editor).toBeHidden();
});

test("a rule gets a WebSocket message action", async ({ page }) => {
  await page.goto("/?rules");
  const editor = page.getByTestId("rules-editor");
  await editor.getByTestId("new-rule").click();
  await editor.getByTestId("add-action").click();
  await page.getByRole("button", { name: "WebSocket message" }).click();
  const action = editor.getByTestId("action").last();
  await action.getByTestId("ws-op").click();
  await page.getByRole("option", { name: "JSON set" }).click();
  await action.getByTestId("ws-pointer").fill("/price");
  await action.getByTestId("json-set-raw").fill("0");
  await editor.getByTestId("save-rule").click();
  await expect(editor.getByTestId("rule-row").filter({ hasText: "/price = 0" })).toBeVisible();
});
