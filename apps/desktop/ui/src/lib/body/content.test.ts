import { describe, expect, it } from "vitest";

import { kindFromContentType, looksBinary, sniff } from "./content";

const enc = (s: string) => new TextEncoder().encode(s);

describe("content detection", () => {
  it("maps content types", () => {
    expect(kindFromContentType("application/json; charset=utf-8")).toBe("json");
    expect(kindFromContentType("application/graphql")).toBe("graphql");
    expect(kindFromContentType("image/svg+xml")).toBe("svg");
    expect(kindFromContentType("multipart/form-data; boundary=x")).toBe("multipart");
    expect(kindFromContentType("font/woff2")).toBe("font");
    expect(kindFromContentType(null)).toBe("none");
  });

  it("sniffs magic bytes over the header", () => {
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
    expect(sniff(png, "application/json")).toEqual({ kind: "image", mime: "image/png" });
    expect(sniff(enc("RIFF\x24\0\0\0WEBPVP8 "), null)).toEqual({ kind: "image", mime: "image/webp" });
  });

  it("falls back to the text heuristic", () => {
    expect(sniff(enc(' {"a": 1}'), null).kind).toBe("json");
    expect(sniff(enc("<?xml version='1.0'?><a/>"), "text/plain").kind).toBe("xml");
    expect(sniff(enc("<!doctype html><p>"), "application/octet-stream").kind).toBe("html");
    expect(sniff(enc("hello"), "text/csv")).toEqual({ kind: "text", mime: "text/csv" });
    expect(sniff(new Uint8Array([0x80, 0x81, 0x00]), null).kind).toBe("binary");
    expect(sniff(enc("a=1"), "application/x-www-form-urlencoded").kind).toBe("formurlencoded");
  });

  it("treats NULs and invalid UTF-8 as binary", () => {
    expect(looksBinary(new Uint8Array([0x41, 0x00]))).toBe(true);
    expect(looksBinary(new Uint8Array([0xc3, 0x28, 0x41, 0x41, 0x41, 0x41]))).toBe(true);
    expect(looksBinary(enc("café").subarray(0, 4))).toBe(false);
  });
});
