import { describe, expect, it } from "vitest";

import { formatJson } from "./jsonFormat";

describe("formatJson", () => {
  it("indents like JSON.stringify for ordinary documents", () => {
    const doc = { a: [1, true, null], b: { c: "x:y,{z}" }, d: [], e: {} };
    expect(formatJson(JSON.stringify(doc))).toBe(JSON.stringify(doc, null, 2));
  });

  it("keeps number literals and escapes exactly", () => {
    expect(formatJson('{"price":1.50,"id":12345678901234567890,"s":"a\\"b"}')).toBe(
      '{\n  "price": 1.50,\n  "id": 12345678901234567890,\n  "s": "a\\"b"\n}',
    );
  });

  it("formats loosely when the JSON is broken or cut short", () => {
    expect(formatJson('{"a": [1, 2')).toBe('{\n  "a": [\n    1,\n    2');
    expect(formatJson("hello")).toBeNull();
  });
});
