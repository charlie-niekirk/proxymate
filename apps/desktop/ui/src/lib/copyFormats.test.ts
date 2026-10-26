import { describe, expect, it } from "vitest";

import {
  authToken,
  base64,
  dslTerm,
  formPair,
  headerLine,
  headersBlock,
  hexBytes,
  hexDump,
  hexRow,
  pairsJson,
  parseCookie,
  parseSetCookie,
  plainBlock,
  queryString,
  sseWire,
  tableText,
  withTerm,
} from "@/lib/copyFormats";

describe("copy formats", () => {
  it("writes header lines and blocks", () => {
    expect(headerLine("Content-Type", "text/html; charset=utf-8")).toBe("Content-Type: text/html; charset=utf-8");
    expect(
      headersBlock([
        ["Accept", "*/*"],
        ["Set-Cookie", "a=1"],
        ["Set-Cookie", "b=2"],
      ]),
    ).toBe("Accept: */*\nSet-Cookie: a=1\nSet-Cookie: b=2");
    expect(plainBlock([["Issuer", "R3"]])).toBe("Issuer: R3");
  });

  it("re-encodes query and form pairs", () => {
    expect(formPair("q", "two words&more")).toBe("q=two+words%26more");
    expect(
      queryString([
        ["x", "1"],
        ["y", "é"],
        ["x", "3"],
      ]),
    ).toBe("x=1&y=%C3%A9&x=3");
  });

  it("turns pairs into JSON, repeats as arrays", () => {
    expect(
      JSON.parse(
        pairsJson([
          ["a", "1"],
          ["b", "2"],
          ["a", "3"],
          ["a", "4"],
        ]),
      ),
    ).toEqual({ a: ["1", "3", "4"], b: "2" });
  });

  it("reads cookies", () => {
    expect(parseCookie("session=abc; theme=dark;  empty=; flag")).toEqual([
      { name: "session", value: "abc" },
      { name: "theme", value: "dark" },
      { name: "empty", value: "" },
      { name: "flag", value: "" },
    ]);
    expect(parseCookie("")).toEqual([]);
    expect(parseSetCookie("id=a3fWa=; Expires=Thu, 21 Oct 2021 07:28:00 GMT; Secure")).toEqual({ name: "id", value: "a3fWa=" });
    expect(parseSetCookie("; Secure")).toBeNull();
  });

  it("takes the scheme off an Authorization value", () => {
    expect(authToken("Bearer eyJhbGciOi.payload.sig")).toBe("eyJhbGciOi.payload.sig");
    expect(authToken("Basic dXNlcjpwYXNz")).toBe("dXNlcjpwYXNz");
    expect(authToken("opaque-token")).toBeNull();
  });

  it("dumps hex like the Hex viewer", () => {
    const bytes = new Uint8Array([...new TextEncoder().encode("Hello, hex view!"), 0x00, 0xff]);
    expect(hexBytes(bytes.subarray(0, 4))).toBe("48 65 6c 6c");
    expect(hexRow(bytes, 0)).toEqual({
      offset: "00000000",
      hex: "48 65 6c 6c 6f 2c 20 68  65 78 20 76 69 65 77 21",
      ascii: "Hello, hex view!",
    });
    expect(hexRow(bytes, 1)).toEqual({ offset: "00000010", hex: "00 ff", ascii: ".." });
    expect(hexDump(bytes).split("\n")).toEqual([
      "00000000  48 65 6c 6c 6f 2c 20 68  65 78 20 76 69 65 77 21  Hello, hex view!",
      `00000010  ${"00 ff".padEnd(48)}  ..`,
    ]);
  });

  it("encodes Base64 of any size", () => {
    expect(base64(new TextEncoder().encode("hi"))).toBe("aGk=");
    const big = new Uint8Array(100_000).fill(0x41);
    expect(atob(base64(big)).length).toBe(100_000);
  });

  it("writes SSE events as they travel", () => {
    expect(sseWire({ event: "update", id: "7", data: "line 1\nline 2", retry: null })).toBe(
      "event: update\nid: 7\ndata: line 1\ndata: line 2",
    );
    expect(sseWire({ event: null, id: null, data: "{}", retry: 3000 })).toBe("retry: 3000\ndata: {}");
  });

  it("writes tab-separated tables", () => {
    expect(
      tableText([
        ["DNS", "3 ms"],
        ["Total", "40 ms"],
      ]),
    ).toBe("DNS\t3 ms\nTotal\t40 ms");
  });

  it("builds filter terms the DSL reads", () => {
    expect(dslTerm("host", "example.com")).toBe("host:example.com");
    expect(dslTerm("app", "Google Chrome")).toBe('app:"Google Chrome"');
    expect(dslTerm("reqheader", 'etag: W/"abc"')).toBe('reqheader:"etag: W/"');
    expect(dslTerm("app", "")).toBe('app:""');
  });

  it("narrows the search text without repeating a term", () => {
    expect(withTerm("", "status:404")).toBe("status:404");
    expect(withTerm("  host:a.com ", "status:404")).toBe("host:a.com status:404");
    expect(withTerm("host:a.com status:404", "status:404")).toBe("host:a.com status:404");
  });
});
