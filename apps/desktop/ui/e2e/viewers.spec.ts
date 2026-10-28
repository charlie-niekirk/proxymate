import { expect, test, type Locator, type Page } from "@playwright/test";

import { chooseViewer, fixtureId, openFixture } from "./zoo";

/**
 * One snapshot per viewer, in dark and light, of the inspector showing a
 * zoo fixture. Each case waits until its viewer has really rendered.
 */

type Ready = (body: Locator, page: Page) => Promise<void>;

const code: Ready = async (body) => {
  await expect(body.locator(".cm-content")).toBeVisible();
  await expect(body.locator(".cm-line").nth(1)).toBeVisible();
};

const image: Ready = async (body) => {
  await expect.poll(() => body.getByTestId("image").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
  await expect(body.getByTestId("media-info")).toContainText("px");
};

const media: Ready = async (body) => {
  // Duration is shown once the metadata has loaded.
  await expect(body.getByTestId("media-info")).toContainText(/\d:\d\d\.\d/);
};

const decodedTree: Ready = async (body) => {
  await expect(body.getByTestId("decoded-tree").locator("[data-row]").nth(3)).toBeVisible();
  await expect(body.getByTestId("tree-badge").first()).toBeVisible();
};

interface Case {
  fixture: string;
  snap: string;
  viewer?: string;
  /** The request's body instead of the fixture's default side. */
  pane?: "request" | "response";
  ready: Ready;
  /** Parts that legitimately change between runs (animation, native media chrome). */
  mask?: (body: Locator) => Locator[];
}

const CASES: Case[] = [
  { fixture: "json", snap: "json-pretty", ready: code },
  {
    fixture: "json",
    snap: "json-tree",
    viewer: "JSON Tree",
    ready: async (body) => expect(body.getByTestId("json-tree").locator("[data-row]").nth(3)).toBeVisible(),
  },
  { fixture: "json-gzip", snap: "json-gzip", ready: async (body) => (await code(body, undefined!), expect(body.getByTestId("body-note").first()).toContainText("Decoded from gzip")) },
  {
    fixture: "json-10mb",
    snap: "json-10mb",
    ready: async (body) => {
      await expect(body.getByTestId("highlight-note")).toBeVisible({ timeout: 30_000 });
      await code(body, undefined!);
    },
  },
  { fixture: "xml", snap: "xml", ready: code },
  { fixture: "html", snap: "html", ready: code },
  {
    fixture: "html",
    snap: "html-preview",
    viewer: "Preview",
    ready: async (body) => {
      const frame = body.getByTestId("html-preview").contentFrame();
      await expect(frame.locator("h1")).toHaveText("Proxymate zoo page");
    },
  },
  { fixture: "form", snap: "form", ready: async (body) => expect(body.getByTestId("form-table").locator("tbody tr")).toHaveCount(6) },
  {
    fixture: "multipart",
    snap: "multipart",
    ready: async (body) => {
      await expect(body.getByTestId("multipart-part")).toHaveCount(4);
      await image(body.getByTestId("multipart-part").nth(2), undefined!);
    },
  },
  {
    fixture: "graphql",
    snap: "graphql",
    ready: async (body) => {
      await expect(body.getByTestId("graphql-view")).toContainText("GetRepository");
      await expect(body.locator(".cm-content").nth(1)).toContainText("charlie-niekirk");
    },
  },
  {
    fixture: "js",
    snap: "javascript",
    ready: async (body) => {
      await expect(body.locator(".cm-content")).toContainText("constructor(");
      await expect(body.locator(".cm-line").nth(5)).toBeVisible();
    },
  },
  { fixture: "css", snap: "css", ready: async (body) => expect(body.locator(".cm-line").nth(5)).toBeVisible() },
  { fixture: "text", snap: "text", ready: code },
  { fixture: "png", snap: "image-png", ready: image },
  { fixture: "jpeg-exif", snap: "image-jpeg-exif", ready: async (body) => (await image(body, undefined!), expect(body.getByTestId("media-info")).toContainText("EXIF")) },
  { fixture: "gif-animated", snap: "image-gif", ready: async (body) => (await image(body, undefined!), expect(body.getByTestId("media-info")).toContainText("animated")), mask: (b) => [b.getByTestId("image")] },
  { fixture: "webp", snap: "image-webp", ready: image },
  { fixture: "avif", snap: "image-avif", ready: image },
  { fixture: "ico", snap: "image-ico", ready: image },
  { fixture: "bmp", snap: "image-bmp", ready: image },
  { fixture: "svg-script", snap: "image-svg", ready: image },
  { fixture: "mp4", snap: "video", ready: media, mask: (b) => [b.locator("video")] },
  { fixture: "mp3", snap: "audio", ready: media, mask: (b) => [b.locator("audio")] },
  { fixture: "pdf", snap: "pdf", ready: async (body) => expect(body.getByTestId("pdf-view").locator("iframe")).toBeVisible(), mask: (b) => [b.locator("iframe")] },
  {
    fixture: "font",
    snap: "font",
    ready: async (body, page) => {
      await expect(body.getByTestId("font-view")).toContainText("The quick brown fox");
      await page.evaluate(() => document.fonts.ready);
    },
  },
  { fixture: "binary", snap: "hex", ready: async (body) => expect(body.getByTestId("hex-view")).toContainText("00000010") },
  // M16: decoded in Rust; the mock serves the engine's output for each fixture.
  {
    fixture: "grpc-web",
    snap: "grpc",
    pane: "response",
    ready: async (body) => {
      await decodedTree(body, undefined!);
      await expect(body.getByTestId("decoded-messages").getByTestId("decoded-message")).toHaveCount(1);
      await expect(body.getByTestId("grpc-status")).toContainText("OK (0)");
    },
  },
  {
    fixture: "grpc-web",
    snap: "grpc-request",
    pane: "request",
    ready: async (body) => expect(body.getByTestId("decoded-tree")).toContainText('"o-1001"'),
  },
  { fixture: "protobuf", snap: "protobuf", ready: decodedTree },
  { fixture: "msgpack", snap: "msgpack", ready: decodedTree },
  { fixture: "cbor", snap: "cbor", ready: decodedTree },
];

for (const theme of ["dark", "light"] as const) {
  test.describe(`viewers (${theme})`, () => {
    for (const c of CASES) {
      test(`${c.snap}`, async ({ page }) => {
        const body = await openFixture(page, c.fixture, { theme, pane: c.pane });
        if (c.viewer) await chooseViewer(page, body, c.viewer);
        await c.ready(body, page);
        await expect(page.getByTestId("inspector")).toHaveScreenshot(`${c.snap}-${theme}.png`, { mask: c.mask?.(body) });
      });
    }
  });
}

test("the viewer choice sticks per kind and offers the alternatives", async ({ page }) => {
  const body = await openFixture(page, "json");
  await body.getByTestId("viewer-select").click();
  await expect(page.getByRole("option")).toHaveText(["JSON", "JSON Tree", "Text", "Hex"]);
  await page.getByRole("option", { name: "JSON Tree" }).click();
  await expect(body).toHaveAttribute("data-viewer", "json-tree");
  // Another JSON flow opens in the tree too.
  await page.getByText("/zoo/json-headerless").click();
  await expect(page.getByTestId("inspector").getByTestId("body-viewer").last()).toHaveAttribute("data-viewer", "json-tree");
});

test("the JSON tree filters by JSONPath and pointer", async ({ page }) => {
  const body = await openFixture(page, "json");
  await chooseViewer(page, body, "JSON Tree");
  const filter = body.getByTestId("json-filter");
  await filter.fill("$..login");
  await expect(body.getByTestId("json-tree")).toContainText("1 match");
  await expect(body.getByTestId("json-tree").locator("[data-row]").first()).toContainText('"charlie-niekirk"');
  await filter.fill("/data/repository/topics/1");
  await expect(body.getByTestId("json-tree").locator("[data-row]").first()).toContainText('"proxy"');
  await filter.fill("$.data[");
  await expect(body.getByTestId("json-tree")).toContainText("Unsupported bracket expression");
});

test("GraphQL operations are named in the URL column", async ({ page }) => {
  await openFixture(page, "graphql");
  await expect(page.getByTestId("graphql-op").first()).toHaveText("GetRepository");
});

test("a gRPC call's status pill reads its gRPC status", async ({ page }) => {
  await openFixture(page, "grpc-web", { pane: "response" });
  await expect(page.getByTestId("url-bar")).toContainText("OK");
  const body = page.getByTestId("inspector").getByTestId("body-viewer").last();
  await body.getByTestId("viewer-select").click();
  await expect(page.getByRole("option")).toHaveText(["gRPC", "Hex"]);
});

test("Decode as lists the imported message types", async ({ page }) => {
  await page.goto("/?zoo&protos&select=" + String(fixtureId("protobuf")));
  const body = page.getByTestId("inspector").getByTestId("body-viewer").last();
  await body.getByTestId("decode-as").click();
  await expect(page.getByRole("option", { name: "shop.v1.Order" })).toBeVisible();
});
