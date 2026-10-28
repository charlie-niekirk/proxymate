import { expect, test, type Page } from "@playwright/test";

import { chooseViewer, openFixture } from "./zoo";

/**
 * M17 against the mock backend: multi-select and bulk actions, labels and
 * comments, deleting, header columns, Compare, Repeat and search
 * highlighting in bodies. The engine side of each is tested in Rust
 * (`crates/pm-core/tests/workflow.rs`).
 */

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __copied: string[] };
    w.__copied = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => void w.__copied.push(text),
        write: async (items: ClipboardItem[]) => {
          const item = items[0]!;
          if (item.types.includes("text/plain")) w.__copied.push(await (await item.getType("text/plain")).text());
        },
      },
    });
  });
});

const copied = (page: Page) => page.evaluate(() => (window as unknown as { __copied: string[] }).__copied.at(-1) ?? null);
const row = (page: Page, id: number) => page.locator(`[data-testid=flow-table] [role=row][data-flow-id="${id}"]`);
const mod = process.platform === "darwin" ? "Meta" : "Control";

async function selectRows(page: Page, ids: number[]) {
  await row(page, ids[0]!).click();
  for (const id of ids.slice(1)) await row(page, id).click({ modifiers: [mod] });
}

test("⇧-click selects a range, ⌘-click toggles, and bulk actions take the selection", async ({ page }) => {
  await page.goto("/?theme=dark");
  await row(page, 1).click();
  await row(page, 4).click({ modifiers: ["Shift"] });
  await expect(page.getByTestId("selection-count")).toContainText("4 selected");
  await row(page, 2).click({ modifiers: [mod] });
  await expect(page.getByTestId("selection-count")).toContainText("3 selected");
  await expect(row(page, 2)).toHaveAttribute("data-selected", "false");
  await expect(row(page, 3)).toHaveAttribute("data-selected", "true");

  await row(page, 3).click({ button: "right" });
  await page.getByTestId("bulk-copy-urls").click();
  await expect.poll(async () => (await copied(page))?.split("\n").length).toBe(3);

  // Escape drops back to the focused flow; ⌘A selects the whole view.
  await page.getByTestId("flow-table").press("Escape");
  await expect(page.getByTestId("selection-count")).toHaveCount(0);
  await page.getByTestId("flow-table").press(`${mod}+a`);
  await expect(page.getByTestId("selection-count")).toContainText("100,000 selected");
});

test("labels and comments show in the Notes column and filter", async ({ page }) => {
  await page.goto("/?theme=dark");
  await selectRows(page, [5, 7]);
  await row(page, 5).click({ button: "right" });
  await page.getByTestId("menu-label").click();
  await page.getByTestId("menu-label-red").click();
  await expect(row(page, 5).locator("[data-label=red]")).toBeVisible();
  await expect(row(page, 7).locator("[data-label=red]")).toBeVisible();

  await row(page, 9).click({ button: "right" });
  await page.getByTestId("menu-comment").click();
  await page.getByTestId("comment-text").fill("Slow on purpose?");
  await page.getByTestId("comment-save").click();
  await expect(row(page, 9).getByLabel("Commented")).toBeVisible();

  await page.getByTestId("flow-search").fill("label:red");
  await expect(page.getByTestId("row-count")).toContainText("2 of 100,000 rows");
  await page.getByTestId("flow-search").fill("is:commented");
  await expect(page.getByTestId("row-count")).toContainText("1 of 100,000 rows");
});

test("⌫ deletes the selected flows", async ({ page }) => {
  await page.goto("/?theme=dark");
  await selectRows(page, [2, 3]);
  await page.getByTestId("flow-table").press("Backspace");
  await expect(row(page, 2)).toHaveCount(0);
  await expect(row(page, 3)).toHaveCount(0);
  await expect(page.getByTestId("row-count")).toContainText("99,998 of 100,000 rows");
});

test("a header column shows, sorts and filters by a header", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.getByTestId("flow-table-header").click({ button: "right" });
  await page.getByTestId("add-header-column").click();
  await page.getByTestId("add-column-name").fill("User-Agent");
  await page.getByTestId("add-column-save").click();
  const header = page.locator('[data-column="hdr:request:user-agent"]');
  await expect(header).toHaveText("user-agent");
  await expect(row(page, 0)).toContainText("curl/8.7.1");

  await header.click();
  await expect(header).toHaveAttribute("aria-sort", "ascending");

  // "Filter by" on the cell.
  await row(page, 0).locator("div", { hasText: /^curl\/8\.7\.1$/ }).first().click({ button: "right" });
  await page.getByTestId("menu-filter-header").click();
  await expect(page.getByTestId("flow-search")).toHaveValue('reqheader:"user-agent: curl/8.7.1"');

  await header.click({ button: "right" });
  await page.getByTestId("remove-header-column").click();
  await expect(header).toHaveCount(0);
});

test("Compare shows two flows side by side", async ({ page }) => {
  await page.goto("/?theme=dark");
  await selectRows(page, [0, 6]);
  await row(page, 0).click({ button: "right" });
  await page.getByTestId("bulk-compare").click();
  const compare = page.getByTestId("compare");
  await expect(compare.getByTestId("compare-a")).toContainText("api.github.com");
  await expect(compare.getByTestId("compare-b")).toContainText("api.github.com");
  await compare.getByTestId("compare-tab-request-headers").click();
  await expect(compare.getByTestId("compare-headers").locator("tr")).toHaveCount(3);
  await compare.getByTestId("compare-tab-timing").click();
  await expect(compare.getByTestId("compare-timing")).toContainText("Total");
});

test("Repeat runs and reports its counts", async ({ page }) => {
  await page.goto("/?theme=dark");
  await row(page, 0).click({ button: "right" });
  await page.getByTestId("menu-repeat").click();
  await page.getByTestId("repeat-iterations").fill("50");
  await page.getByTestId("repeat-start").click();
  await expect(page.getByTestId("repeat-done")).toHaveText("50 / 50");
  await expect(page.getByTestId("repeat-statuses")).toContainText("200 × 50");
});

test("the filter's body terms are highlighted in code views and the JSON tree", async ({ page }) => {
  const body = await openFixture(page, "json");
  await page.getByTestId("flow-search").fill("body:proxymate");
  await chooseViewer(page, body, "JSON");
  await expect(body.getByTestId("match-count")).toHaveText(/\d+ matches/);
  await expect(body.locator(".cm-pm-filter-match").first()).toBeVisible();
  await body.getByTestId("match-next").click();
  await expect(body.getByTestId("match-count")).toHaveText(/^1 of \d+$/);

  await chooseViewer(page, body, "JSON Tree");
  await expect(body.getByTestId("match-count")).toHaveText(/\d+ matches/);
  await body.getByTestId("match-next").click();
  await expect(body.getByTestId("match-count")).toHaveText(/^1 of \d+$/);
  await expect(body.locator("[data-current-match] [data-testid=body-match]").first()).toBeVisible();
});
