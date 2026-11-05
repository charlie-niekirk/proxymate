import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

/**
 * R4: axe scans the main window and every dialog against the mock backend,
 * in both themes, for WCAG 2.1 A and AA. Each dialog also has to take the
 * focus when it opens (`DialogContent`'s `initialFocus`).
 */

const mod = process.platform === "darwin" ? "Meta" : "Control";
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function scan(page: Page, include?: string) {
  // Let open/close animations settle so colours are final.
  await page.waitForTimeout(250);
  let builder = new AxeBuilder({ page }).withTags(TAGS);
  if (include) builder = builder.include(include);
  const { violations } = await builder.analyze();
  const report = violations.map((v) => ({
    id: v.id,
    help: v.help,
    nodes: v.nodes.slice(0, 8).map((n) => `${n.target.join(" ")}: ${n.failureSummary ?? ""}`),
  }));
  if (process.env.PM_A11Y_DUMP && report.length > 0) {
    const { appendFileSync } = await import("node:fs");
    appendFileSync(process.env.PM_A11Y_DUMP, `${JSON.stringify({ test: test.info().title, report })}\n`);
  }
  expect(report, JSON.stringify(report, null, 2)).toEqual([]);
}

async function runCommand(page: Page, id: string) {
  await page.keyboard.press(`${mod}+k`);
  await page.getByTestId(`palette-${id}`).click();
}

/** The open dialog has the focus (or something inside it does). */
async function expectDialogFocused(page: Page) {
  const dialog = page.locator('[role="dialog"]').last();
  await expect(dialog).toBeVisible();
  await expect
    .poll(() => dialog.evaluate((d) => d === document.activeElement || d.contains(document.activeElement)))
    .toBe(true);
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme} theme`, () => {
    test("the main window has no axe violations", async ({ page }) => {
      await page.goto(`/?theme=${theme}&select=0`);
      await expect(page.locator('[data-flow-id="0"]')).toHaveAttribute("data-selected", "true");
      await scan(page);
    });

    test("the zoo with a body open has no axe violations", async ({ page }) => {
      await page.goto(`/?zoo&select=0&theme=${theme}`);
      await expect(page.getByTestId("inspector")).toBeVisible();
      await scan(page);
    });

    for (const id of ["settings", "rules", "scripts", "devices", "certificate", "compose"]) {
      test(`the ${id} dialog takes the focus and has no axe violations`, async ({ page }) => {
        await page.goto(`/?theme=${theme}&select=0`);
        await runCommand(page, id);
        await expectDialogFocused(page);
        await scan(page, '[role="dialog"]');
      });
    }

    for (const id of ["rewrite", "comment", "repeat", "add-header-column"]) {
      test(`the ${id} dialog (on a flow) takes the focus and has no axe violations`, async ({ page }) => {
        await page.goto(`/?zoo&select=0&theme=${theme}`);
        await expect(page.getByTestId("inspector")).toBeVisible();
        await runCommand(page, id);
        await expectDialogFocused(page);
        await scan(page, '[role="dialog"]');
      });
    }

    test("the command palette takes the focus and has no axe violations", async ({ page }) => {
      await page.goto(`/?theme=${theme}`);
      await page.keyboard.press(`${mod}+k`);
      await expect(page.getByTestId("palette-input")).toBeFocused();
      await scan(page, '[role="dialog"]');
    });

    test("the Breakpoint Editor takes the focus and has no axe violations", async ({ page }) => {
      await page.goto(`/?breakpoint&theme=${theme}`);
      await expectDialogFocused(page);
      await scan(page, '[role="dialog"]');
    });

    test("the LAN allow prompt focuses its name field and has no axe violations", async ({ page }) => {
      await page.goto(`/?devicePrompt=192.168.1.42&theme=${theme}`);
      await expect(page.getByTestId("device-allow-prompt").locator("input")).toBeFocused();
      await scan(page, '[role="dialog"]');
    });
  });
}

test("the record button is a green Play when stopped and a red Stop when recording", async ({ page }) => {
  await page.goto("/?theme=dark");
  const button = page.getByTestId("record-button");
  await expect(button).toHaveAccessibleName("Stop recording");
  await expect(button).toHaveAttribute("data-recording", "true");
  await button.click();
  await expect(button).toHaveAccessibleName("Start recording");
  await expect(button).toHaveAttribute("data-recording", "false");
  await page.keyboard.press(`${mod}+e`);
  await expect(button).toHaveAccessibleName("Stop recording");
});

test("the header sorts from the keyboard and resizes with the arrow keys", async ({ page }) => {
  await page.goto("/?theme=dark");
  const header = page.getByTestId("flow-table-header");
  const status = header.locator('[data-column="status"]');
  await status.getByRole("button").focus();
  await page.keyboard.press("Enter");
  await expect(status).toHaveAttribute("aria-sort", "ascending");
  const handle = page.getByTestId("resize-status");
  const before = Number(await handle.getAttribute("aria-valuenow"));
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Shift+ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", String(before + 40));
  await page.keyboard.press("Enter");
  await expect(status).toHaveAttribute("aria-sort", "ascending");
  // Rows are gridcells under the grid; the grid names the focused row.
  await page.locator('[data-flow-id="2"]').click();
  await expect(page.getByTestId("flow-table")).toHaveAttribute("aria-activedescendant", "flow-row-2");
  await expect(page.locator('[data-flow-id="2"] > [role="gridcell"]').first()).toBeVisible();
});
