import { expect, test, type Page } from "@playwright/test";

import { chooseViewer, openFixture } from "./zoo";

/**
 * The zoo's page.html and logo.svg try to run scripts and load
 * `http://zoo.invalid/…` resources. Their previews must do neither.
 */

function watch(page: Page) {
  const requests: string[] = [];
  const messages: unknown[] = [];
  page.on("request", (r) => {
    if (!r.url().startsWith(`http://localhost:${process.env.PM_E2E_PORT ?? 1425}/`)) requests.push(r.url());
  });
  page.on("console", (m) => {
    if (/pwned/i.test(m.text())) messages.push(m.text());
  });
  return { requests, messages };
}

test("@security the HTML preview runs no scripts and loads nothing", async ({ page }) => {
  const seen = watch(page);
  await page.addInitScript(() => {
    (window as unknown as { __messages: unknown[] }).__messages = [];
    window.addEventListener("message", (e) => (window as unknown as { __messages: unknown[] }).__messages.push(e.data));
  });
  const body = await openFixture(page, "html");
  await chooseViewer(page, body, "Preview");
  const iframe = body.getByTestId("html-preview");
  await expect(iframe).toHaveAttribute("sandbox", "");
  const frame = iframe.contentFrame();
  await expect(frame.locator("h1")).toHaveText("Proxymate zoo page");
  // Give onload handlers, the meta refresh, remote images and frames time to fire.
  await page.waitForTimeout(1500);

  expect(seen.requests.filter((u) => !u.startsWith("blob:") && !u.startsWith("data:"))).toEqual([]);
  expect(await page.evaluate(() => (window as unknown as { __messages: unknown[] }).__messages)).toEqual([]);
  await expect(frame.locator("body")).not.toHaveAttribute("data-pwned", /.*/);
  await expect(frame.locator("html")).not.toHaveAttribute("data-pwned", /.*/);
  // Still on the original document: neither the meta refresh nor a link navigated it.
  await expect(frame.locator("h1")).toHaveText("Proxymate zoo page");
  expect(seen.messages).toEqual([]);
});

test("@security an SVG with scripts renders as an inert image", async ({ page }) => {
  const seen = watch(page);
  const body = await openFixture(page, "svg-script");
  const img = body.getByTestId("image");
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(120);
  await page.waitForTimeout(1000);
  expect(seen.requests.filter((u) => !u.startsWith("blob:") && !u.startsWith("data:"))).toEqual([]);
  // The SVG is never put in the DOM as markup.
  await expect(page.locator("svg[onload]")).toHaveCount(0);
  expect(seen.messages).toEqual([]);
});
