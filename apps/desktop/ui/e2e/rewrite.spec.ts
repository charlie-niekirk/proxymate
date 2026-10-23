import { expect, test, type Page } from "@playwright/test";

import { fixtureId } from "./zoo";

/**
 * The M13 visual rewrite against the mock backend: the Rewrite dialog on
 * the zoo JSON flow (edit, rename, add; the actions it makes; the preview)
 * and the Breakpoint Editor's JSON tree, in dark and light. Turning edits
 * into actions is table-tested in `lib/rewrite/diff.test.ts`; the engine
 * side in Rust.
 */

async function openRewrite(page: Page, theme: "dark" | "light") {
  const id = fixtureId("json");
  await page.goto(`/?zoo&select=${id}&rewrite=${id}&theme=${theme}`);
  const dialog = page.getByTestId("rewrite-dialog");
  await expect(dialog.getByTestId("rewrite-tree-rows")).toBeVisible();
  return dialog;
}

/** A tree row by its key. */
const row = (dialog: ReturnType<Page["getByTestId"]>, key: string) =>
  dialog.getByTestId("json-editor-row").filter({ hasText: new RegExp(`^"${key}"`) });

for (const theme of ["dark", "light"] as const) {
  test(`edits in the Rewrite dialog become exact actions (${theme})`, async ({ page }) => {
    const dialog = await openRewrite(page, theme);
    await expect(dialog.getByTestId("match-path")).toHaveAttribute("data-state", "on");

    // Edit a value.
    await row(dialog, "durationMs").dblclick();
    await dialog.getByTestId("json-value-input").fill("99.50");
    await dialog.getByTestId("json-value-input").press("Enter");
    // Rename a key (F2).
    await row(dialog, "cached").click();
    await page.keyboard.press("F2");
    await dialog.getByTestId("json-key-input").fill("fromCache");
    await dialog.getByTestId("json-key-input").press("Enter");
    // Add a key after it (⌘↩), then make it a number.
    await page.keyboard.press("ControlOrMeta+Enter");
    await dialog.getByTestId("json-key-input").fill("added");
    await dialog.getByTestId("json-key-input").press("Enter");
    await row(dialog, "added").click({ button: "right" });
    await page.getByTestId("json-menu-type").click();
    await page.getByTestId("json-type-num").click();

    await expect(row(dialog, "durationMs")).toHaveAttribute("data-status", "changed");
    await expect(row(dialog, "fromCache")).toHaveAttribute("data-status", "renamed");
    await expect(dialog.getByTestId("rewrite-footer")).toContainText("4 changes");
    await expect(dialog.getByTestId("preview-body")).toContainText('"durationMs": 99.50');

    await dialog.getByTestId("rewrite-panel-actions").click();
    await expect(dialog.getByTestId("rewrite-action")).toHaveText([
      /JSON remove\s*response remove \/meta\/cached/,
      /JSON set\s*response \/meta\/durationMs = 99.50/,
      /JSON set\s*response \/meta\/fromCache = true/,
      /JSON set\s*response \/meta\/added = 0/,
    ]);
    await dialog.getByTestId("rewrite-panel-preview").click();
    await row(dialog, "added").click();
    await expect(dialog).toHaveScreenshot(`rewrite-dialog-${theme}.png`);

    // Undo takes the type change back.
    await page.keyboard.press("ControlOrMeta+z");
    await expect(row(dialog, "added")).toContainText('""');
  });
}

test("the whole-body toggle, the request side and saving", async ({ page }) => {
  const dialog = await openRewrite(page, "dark");
  await row(dialog, "warnings").click();
  await page.keyboard.press("Backspace");
  await dialog.getByTestId("rewrite-whole-body").click();
  await dialog.getByTestId("rewrite-panel-actions").click();
  await expect(dialog.getByTestId("rewrite-action")).toHaveText([/Set body\s*response body/]);

  await dialog.getByTestId("rewrite-side-request").click();
  await dialog.getByTestId("rewrite-tab-headers").click();
  await dialog.getByTestId("rewrite-headers-request-add").click();
  await dialog.getByTestId("kv-name").last().fill("X-Debug");
  await dialog.getByTestId("kv-value").last().fill("1");
  await expect(dialog.getByTestId("rewrite-action")).toHaveText([/Set header\s*request X-Debug: 1/, /Set body/]);

  await dialog.getByTestId("match-any-method").click();
  await expect(dialog.getByTestId("matcher-summary")).not.toContainText("GET");
  await dialog.getByTestId("rewrite-save").click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText(/^Saved “Rewrite · GET zoo\.proxymate\.test\/zoo\/json”/)).toBeVisible();
});

for (const theme of ["dark", "light"] as const) {
  test(`the Breakpoint Editor edits a JSON body as a tree (${theme})`, async ({ page }) => {
    await page.goto(`/?breakpoint&theme=${theme}`);
    const editor = page.getByTestId("breakpoint-editor");
    await expect(editor.getByTestId("bp-body")).toContainText('"amount": 4200');
    await editor.getByTestId("bp-body-tree").click();
    const tree = editor.getByTestId("bp-body-json-rows");
    await tree.getByTestId("json-editor-row").filter({ hasText: /^"amount"/ }).dblclick();
    await editor.getByTestId("json-value-input").fill("5000");
    await editor.getByTestId("json-value-input").press("Enter");
    await expect(tree.getByTestId("json-editor-row").filter({ hasText: /^"amount"/ })).toHaveAttribute("data-status", "changed");
    await expect(editor.getByTestId("bp-body-json")).toHaveScreenshot(`breakpoint-tree-${theme}.png`);
    await editor.getByTestId("bp-body-text").click();
    await expect(editor.getByTestId("bp-body")).toContainText('"amount": 5000');
  });
}
