import { describe, expect, it } from "vitest";

import { boundaryOf, parseMultipart } from "./multipart";

const enc = (s: string) => new TextEncoder().encode(s);
const dec = (b: Uint8Array) => new TextDecoder().decode(b);

describe("multipart", () => {
  it("reads the boundary parameter", () => {
    expect(boundaryOf("multipart/form-data; boundary=----abc")).toBe("----abc");
    expect(boundaryOf('multipart/mixed; charset=utf-8; boundary="a b"')).toBe("a b");
    expect(boundaryOf("application/json")).toBeNull();
  });

  it("splits parts with their headers", () => {
    const body = enc(
      "preamble\r\n--XX\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n" +
        '--XX\r\nContent-Disposition: form-data; name="file"; filename="a \\"b\\".txt"\r\nContent-Type: text/plain\r\n\r\nline1\r\nline2\r\n' +
        "--XX--\r\nepilogue",
    );
    const parts = parseMultipart(body, "XX");
    expect(parts).toHaveLength(2);
    expect(parts[0]!.name).toBe("title");
    expect(dec(parts[0]!.body)).toBe("Hello");
    expect(parts[1]!.filename).toBe('a "b".txt');
    expect(parts[1]!.contentType).toBe("text/plain");
    expect(dec(parts[1]!.body)).toBe("line1\r\nline2");
  });

  it("keeps a truncated last part and accepts LF line ends", () => {
    const parts = parseMultipart(enc("--b\nContent-Type: text/plain\n\nfirst\n--b\n\nsecond, cut"), "b");
    expect(parts.map((p) => dec(p.body))).toEqual(["first", "second, cut"]);
  });
});
