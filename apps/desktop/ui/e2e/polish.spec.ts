import { expect, test, type Page } from "@playwright/test";

/**
 * M10 against the mock backend: keyboard navigation, the ⌘K palette,
 * persisted columns, Appearance, DSL autocomplete and the new Settings
 * pages. Eviction and deep search are tested in Rust (crates/pm-core/tests/limits.rs).
 */

const mod = process.platform === "darwin" ? "Meta" : "Control";

async function selectedId(page: Page) {
  return page.locator('[data-testid="flow-table"] [data-selected="true"]').getAttribute("data-flow-id");
}

test("the table moves the selection with the keyboard and Enter focuses the inspector", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.locator('[data-flow-id="3"]').click();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  expect(await selectedId(page)).toBe("5");
  await page.keyboard.press("ArrowUp");
  expect(await selectedId(page)).toBe("4");
  await page.keyboard.press("Home");
  expect(await selectedId(page)).toBe("0");
  await page.keyboard.press("PageDown");
  expect(Number(await selectedId(page))).toBeGreaterThan(10);
  await page.keyboard.press("End");
  await expect(page.locator('[data-flow-id="99999"]')).toHaveAttribute("data-selected", "true");
  await page.keyboard.press("Enter");
  await expect(page.locator('[data-testid="inspector"] [role="tab"][data-state="active"]').first()).toBeFocused();
  // Escape goes back to the table.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("flow-table")).toBeFocused();
});

test("⌘K runs commands; ⌘F and ⌘L focus the search boxes", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.keyboard.press(`${mod}+k`);
  await page.getByTestId("palette-input").fill("settings storage");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("settings-storage")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.keyboard.press(`${mod}+f`);
  await expect(page.getByTestId("flow-search")).toBeFocused();
  await page.keyboard.press(`${mod}+l`);
  await expect(page.locator("#sidebar-filter")).toBeFocused();
});

test("hidden columns and sort survive a reload", async ({ page }) => {
  await page.goto("/?theme=dark");
  const header = page.getByTestId("flow-table-header");
  await expect(header.locator('[data-column="client"]')).toBeVisible();
  await header.click({ button: "right" });
  await page.getByTestId("column-toggle-client").click();
  await page.keyboard.press("Escape");
  await expect(header.locator('[data-column="client"]')).toHaveCount(0);
  await header.locator('[data-column="status"]').click();
  await page.reload();
  await expect(page.getByTestId("flow-table-header").locator('[data-column="client"]')).toHaveCount(0);
  await expect(page.getByTestId("flow-table-header").locator('[data-column="status"]')).toHaveAttribute("aria-sort", "ascending");
});

test("Appearance switches the theme and remembers it", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("settings-button").click();
  await page.getByTestId("settings-page-appearance").click();
  await page.getByTestId("appearance-light").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByTestId("appearance-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("the search box completes keys and known hosts", async ({ page }) => {
  await page.goto("/?theme=dark");
  const search = page.getByTestId("flow-search");
  await search.click();
  await search.pressSequentially("ho");
  await expect(page.getByTestId("flow-search-suggestions")).toContainText("host:");
  await page.keyboard.press("Tab");
  await expect(search).toHaveValue("host:");
  await search.pressSequentially("sla");
  await expect(page.getByTestId("flow-search-suggestions")).toContainText("slack.com");
  await page.keyboard.press("Enter");
  await expect(search).toHaveValue("host:slack.com ");
  await expect(page.getByTestId("row-count")).toContainText("of 100,000 rows");
});

test("storage limits are validated and the disk-full state shows in the footer", async ({ page }) => {
  await page.goto("/?theme=dark&diskFull");
  await expect(page.getByTestId("storage-error")).toContainText("Not recording bodies");
  await page.getByTestId("storage-error").click();
  const storage = page.getByTestId("settings-storage");
  await storage.getByTestId("storage-max-flows").fill("10");
  await storage.getByTestId("storage-save").click();
  await expect(storage.getByTestId("settings-error").last()).toContainText("at least 100 flows");
});

test("a port in use is explained and the old one kept", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.getByTestId("settings-button").click();
  const proxy = page.getByTestId("settings-proxy");
  await proxy.getByTestId("settings-port").fill("8080");
  await proxy.getByRole("button", { name: "Apply" }).click();
  await expect(proxy.getByTestId("settings-error")).toContainText("in use");
});

test("pinning a flow lists it under Pinned", async ({ page }) => {
  await page.goto("/?theme=dark");
  await page.locator('[data-flow-id="2"]').click({ button: "right" });
  await page.getByTestId("menu-pin").click();
  await page.getByTestId("sidebar-pinned").click();
  await expect(page.getByTestId("flow-search")).toHaveValue("is:pinned");
  await expect(page.getByTestId("row-count")).toContainText("1 of 100,000 rows");
});
