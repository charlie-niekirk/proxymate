import { describe, expect, it } from "vitest";

import type { FlowRow_Serialize } from "@/bindings";

import { decodeRow, formatCount, formatDuration, formatMs, formatSize, methodToken, statusLabel, statusToken, urlParts, versionLabel } from "./flow";

const tuple: FlowRow_Serialize = [
  7, "completed", "https", "GET", "api.example.com", 443, "/v1/users", "page=2", 404, "json", "Safari", 1.5, 42, 0, 1024, true,
  "HTTP/2", "intercepted",
];

describe("decodeRow", () => {
  it("maps the positional tuple to named fields", () => {
    const row = decodeRow(tuple);
    expect(row).toMatchObject({ id: 7, method: "GET", host: "api.example.com", status: 404, contentKind: "json", edited: true });
    expect(row).toMatchObject({ httpVersion: "HTTP/2", tls: "intercepted" });
  });
});

describe("urlParts", () => {
  it("omits default ports and prefixes the query", () => {
    expect(urlParts(decodeRow(tuple))).toEqual({
      scheme: "https://",
      host: "api.example.com",
      port: "",
      path: "/v1/users",
      query: "?page=2",
    });
  });

  it("keeps non-default ports", () => {
    expect(urlParts({ scheme: "http", host: "localhost", port: 3000, path: "/", query: "" }).port).toBe(":3000");
  });
});

describe("colour tokens", () => {
  it("maps methods to --m-* tokens", () => {
    expect(methodToken("POST")).toBe("post");
    expect(methodToken("PROPFIND")).toBe("other");
  });

  it("maps status classes and flow states to --s-* tokens", () => {
    expect(statusToken(204, "completed")).toBe("2xx");
    expect(statusToken(302, "completed")).toBe("3xx");
    expect(statusToken(429, "completed")).toBe("4xx");
    expect(statusToken(503, "completed")).toBe("5xx");
    expect(statusToken(null, "active")).toBe("active");
    expect(statusToken(null, "failed")).toBe("failed");
    expect(statusLabel(null, "tunnel")).toBe("Tunnel");
  });
});

describe("formatters", () => {
  it("formats sizes and durations compactly", () => {
    expect(formatSize(0)).toBe("—");
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1536)).toBe("1.5 KB");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(42)).toBe("42 ms");
    expect(formatDuration(1500)).toBe("1.50 s");
    expect(formatMs(0.25)).toBe("0.25 ms");
    expect(formatMs(12.34)).toBe("12.3 ms");
    expect(formatMs(250.6)).toBe("251 ms");
  });

  it("labels HTTP versions compactly", () => {
    expect(versionLabel("HTTP/2")).toBe("h2");
    expect(versionLabel("HTTP/1.1")).toBe("1.1");
  });
});

describe("formatCount", () => {
  it("keeps sidebar counts narrow", () => {
    expect([0, 999, 1000, 1234, 12_345, 999_999, 1_250_000].map(formatCount)).toEqual([
      "0",
      "999",
      "1.0k",
      "1.2k",
      "12k",
      "1000k",
      "1.3M",
    ]);
  });
});
