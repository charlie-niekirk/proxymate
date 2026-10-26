import { expect, test, type Locator, type Page } from "@playwright/test";

import { chooseViewer, openFixture } from "./zoo";

/**
 * M15's context menus against the mock backend: what each copies, where
 * the right-click lands. Clipboard writes are captured in the page
 * (WebKit's clipboard needs a permission Playwright can't grant); the
 * formats themselves are unit-tested in lib/copyFormats.test.ts.
 */

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __copied: string[] };
    w.__copied = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => void w.__copied.push(text),
        write: async () => {},
      },
    });
  });
});

/** The last text copied. */
const copied = (page: Page) => page.evaluate(() => (window as unknown as { __copied: string[] }).__copied.at(-1) ?? null);

async function menuItem(page: Page, target: Locator, item: string) {
  await target.click({ button: "right" });
  await page.getByTestId(item).click();
}

test("a header row copies its value and line, and filters by it", async ({ page }) => {
  await page.goto("/?zoo&select=0&theme=dark");
  const row = page.getByTestId("request-headers").locator("tr", { hasText: "User-Agent" });

  await menuItem(page, row, "kv-copy-value");
  await expect.poll(() => copied(page)).toBe("proxymate-zoo/1");
  await menuItem(page, row, "kv-copy-line");
  await expect.poll(() => copied(page)).toBe("User-Agent: proxymate-zoo/1");
  await menuItem(page, row, "kv-copy-all");
  await expect.poll(() => copied(page)).toMatch(/^:authority: zoo\.proxymate\.test\nUser-Agent: proxymate-zoo\/1/);

  await menuItem(page, row, "kv-filter");
  await expect(page.getByTestId("flow-search")).toHaveValue('reqheader:"User-Agent: proxymate-zoo/1"');
});

test("the URL bar copies the URL and status", async ({ page }) => {
  await page.goto("/?zoo&select=0&theme=dark");
  const bar = page.getByTestId("url-bar");
  await menuItem(page, bar, "url-copy-url");
  await expect.poll(() => copied(page)).toBe("https://zoo.proxymate.test/zoo/json");
  await menuItem(page, bar, "url-copy-status");
  await expect.poll(() => copied(page)).toMatch(/^200\b/);
});

test("code views copy what they show; every body menu copies the body", async ({ page }) => {
  const body = await openFixture(page, "json");
  await chooseViewer(page, body, "JSON");
  await menuItem(page, body.getByTestId("code-view"), "code-copy-all");
  await expect.poll(() => copied(page)).toContain('\n  "data": {');

  await menuItem(page, body.getByTestId("code-view"), "menu-copy-body");
  const raw = await copied(page);
  expect(JSON.parse(raw!)).toHaveProperty("data.repository.name", "proxymate");

  await chooseViewer(page, body, "JSON Tree");
  const line = body.getByTestId("json-tree").locator("[data-row]").first();
  await line.click({ button: "right" });
  await expect(page.getByTestId("json-tree-menu")).toContainText("Copy value");
  await page.getByTestId("menu-copy-body-base64").click();
  await expect.poll(async () => Buffer.from((await copied(page)) ?? "", "base64").toString("utf8")).toBe(raw);
});

test("the Body tab's Copy button copies the decoded body", async ({ page }) => {
  const body = await openFixture(page, "json-gzip");
  await body.getByTestId("copy-body").click();
  await expect.poll(async () => JSON.parse((await copied(page)) ?? "null")).toHaveProperty("data.repository.name", "proxymate");
});

test("hex rows copy as hex", async ({ page }) => {
  const body = await openFixture(page, "binary");
  await chooseViewer(page, body, "Hex");
  await menuItem(page, body.getByTestId("hex-view").locator('[data-menu-key="0"]'), "hex-copy-row");
  await expect.poll(() => copied(page)).toMatch(/^([0-9a-f]{2} {1,2}){15}[0-9a-f]{2}$/);
});

test("the flow menu copies the cell under the cursor and opens from the row's …", async ({ page }) => {
  await page.goto("/?theme=dark");
  const status = page.getByTestId("flow-table-header").locator('[data-column="status"]');
  const row = page.locator('[data-flow-id="2"]');
  const [col, line] = [await status.boundingBox(), await row.boundingBox()];
  await page.mouse.click(col!.x + col!.width / 2, line!.y + line!.height / 2, { button: "right" });
  const cell = page.getByTestId("menu-copy-cell");
  await expect(cell).toContainText("Copy Status");
  await cell.click();
  await expect.poll(() => copied(page)).toMatch(/^\d{3}$/);

  await page.locator('[data-flow-id="4"]').hover();
  await page.locator('[data-flow-id="4"]').getByTestId("row-tools").click();
  await expect(page.getByTestId("flow-menu")).toBeVisible();
  await expect(page.locator('[data-flow-id="4"]')).toHaveAttribute("data-selected", "true");
});

test("submenu items line up with the others (the flow menu's Script)", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.locator('[data-flow-id="0"]').click({ button: "right" });
  // From the icon's right edge to the label's first glyph.
  const gap = (testId: string) =>
    page.getByTestId(testId).evaluate((el) => {
      const icon = el.querySelector("svg")!.getBoundingClientRect();
      const text = [...el.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && n.textContent!.trim())!;
      const range = document.createRange();
      range.selectNodeContents(text);
      return range.getBoundingClientRect().left - icon.right;
    });
  expect(Math.abs((await gap("menu-script")) - (await gap("menu-replay")))).toBeLessThan(1);
});

test("WebSocket messages copy their whole payload", async ({ page }) => {
  await page.goto("/?select=13&theme=dark");
  const row = page.getByTestId("ws-row").first();
  await menuItem(page, row, "ws-copy-base64");
  await expect.poll(() => copied(page)).toMatch(/^[A-Za-z0-9+/]+=*$/);
});

test("Settings › MCP copies the bearer token, which stays masked on screen", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("mcp-indicator").click();
  const settings = page.getByTestId("settings-mcp");
  await settings.getByTestId("mcp-copy-json").click();
  const config = await copied(page);
  await menuItem(page, settings.getByTestId("mcp-copy-claude").locator(".."), "mcp-copy-token");
  const token = await copied(page);
  expect(token).not.toContain("•");
  expect(config).toContain(`Bearer ${token}`);
  await expect(settings).toContainText("Bearer ••••••••");
});

test("a rule row duplicates and deletes from its menu", async ({ page }) => {
  await page.goto("/?rules");
  const editor = page.getByTestId("rules-editor");
  const rows = editor.getByTestId("rule-row");
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  const name = (await rows.first().locator(".font-medium").first().textContent())!;

  await menuItem(page, rows.first(), "rule-menu-duplicate");
  await expect(rows).toHaveCount(before + 1);
  const copy = rows.filter({ hasText: `${name} copy` });
  await expect(copy).toHaveAttribute("data-selected", "true");

  await menuItem(page, copy, "rule-menu-delete");
  await expect(rows).toHaveCount(before);
});

test("labels and buttons have no menu", async ({ page }) => {
  await page.goto("/?select=0&theme=dark");
  await page.getByTestId("inspector").getByText("Request", { exact: true }).click({ button: "right" });
  await page.getByLabel("Clear").click({ button: "right" });
  await expect(page.getByRole("menu")).toHaveCount(0);
});
