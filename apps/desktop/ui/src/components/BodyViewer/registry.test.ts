import { describe, expect, it } from "vitest";

import type { ContentKind } from "@/bindings";

import { pickViewer, VIEWERS, viewersFor } from "./registry";

const KINDS: ContentKind[] = [
  "none", "json", "xml", "html", "formurlencoded", "multipart", "graphql", "javascript", "css", "text",
  "image", "svg", "video", "audio", "font", "pdf", "binary", "grpc", "protobuf", "messagepack", "cbor",
];

describe("viewer registry", () => {
  it("offers hex for every kind and a known viewer first", () => {
    for (const kind of KINDS) {
      expect(viewersFor(kind)).toContain("hex");
      expect(VIEWERS[viewersFor(kind)[0]!]).toBeDefined();
    }
  });

  it("picks the best viewer unless the user chose another that fits", () => {
    expect(pickViewer("json", undefined)).toBe("json");
    expect(pickViewer("json", "json-tree")).toBe("json-tree");
    expect(pickViewer("image", "json-tree")).toBe("image");
    expect(pickViewer("svg", undefined)).toBe("image");
    expect(pickViewer("binary", undefined)).toBe("hex");
    expect(pickViewer("grpc", undefined)).toBe("grpc");
    expect(pickViewer("cbor", "json-tree")).toBe("decoded");
  });
});
