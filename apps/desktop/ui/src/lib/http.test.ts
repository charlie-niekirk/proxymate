import { describe, expect, it } from "vitest";

import { bodyText, displayHeaderName, headerValue, queryPairs, rawMessage } from "@/lib/http";

describe("http helpers", () => {
  it("title-cases lower-case header names only", () => {
    expect(displayHeaderName("content-type")).toBe("Content-Type");
    expect(displayHeaderName("x-request-id")).toBe("X-Request-Id");
    expect(displayHeaderName("X-API-Key")).toBe("X-API-Key");
  });

  it("finds header values case-insensitively", () => {
    expect(headerValue([{ name: "content-type", value: "text/plain" }], "Content-Type")).toBe("text/plain");
    expect(headerValue([], "host")).toBeUndefined();
  });

  it("parses query pairs in order with repeats", () => {
    expect(queryPairs("http://a/b?x=1&y=two%20words&x=3#frag")).toEqual([
      ["x", "1"],
      ["y", "two words"],
      ["x", "3"],
    ]);
    expect(queryPairs("http://a/b")).toEqual([]);
  });

  it("tells text from binary", () => {
    expect(bodyText(new TextEncoder().encode("héllo"))).toBe("héllo");
    expect(bodyText(new Uint8Array([0x1f, 0x8b, 0x08, 0x00]))).toBeNull();
    expect(bodyText(new Uint8Array([0xff, 0xfe, 0x41]))).toBeNull();
  });

  it("rebuilds a raw message", () => {
    expect(rawMessage("GET / HTTP/1.1", [{ name: "host", value: "a" }], null)).toBe("GET / HTTP/1.1\nHost: a\n\n");
  });
});
