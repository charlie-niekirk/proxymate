import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { expect, type Page } from "@playwright/test";

interface Fixture {
  name: string;
  request?: unknown;
}

const manifest = fileURLToPath(new URL("../../../../crates/pm-testkit/fixtures/zoo/zoo.json", import.meta.url));
export const FIXTURES: Fixture[] = (JSON.parse(readFileSync(manifest, "utf8")) as { fixtures: Fixture[] }).fixtures;

export function fixtureId(name: string): number {
  const id = FIXTURES.findIndex((f) => f.name === name);
  if (id < 0) throw new Error(`no zoo fixture ${name}`);
  return id;
}

/** Opens the app on a zoo flow, its request or response Body tab showing. */
export async function openFixture(page: Page, name: string, { theme = "dark", pane }: { theme?: "dark" | "light"; pane?: "request" | "response" } = {}) {
  const side = pane ?? (FIXTURES[fixtureId(name)]!.request ? "request" : "response");
  const params = new URLSearchParams({ zoo: "", select: String(fixtureId(name)), theme });
  if (side === "request") params.set("requestTab", "Body");
  await page.goto(`/?${params}`);
  // With the request tab on Body the request's viewer comes first; the response's is always last.
  const viewers = page.getByTestId("inspector").getByTestId("body-viewer");
  const body = side === "request" ? viewers.first() : viewers.last();
  await expect(body).toBeVisible();
  return body;
}

/** Picks a viewer from the body's dropdown. */
export async function chooseViewer(page: Page, body: ReturnType<Page["locator"]>, label: string) {
  await body.getByTestId("viewer-select").click();
  await page.getByRole("option", { name: label, exact: true }).click();
}
