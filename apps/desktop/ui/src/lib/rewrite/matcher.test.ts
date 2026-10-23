import { describe, expect, it } from "vitest";

import { lineDiff, prettyForDiff } from "./lines";
import { isIdLike, matcherFor, modeOf, suggestGlob, type FlowTarget } from "./matcher";

const target: FlowTarget = {
  method: "GET",
  scheme: "https",
  host: "api.example.com",
  port: 443,
  path: "/v1/users/42/orders/3f2c9a1e-5b7d-4c11-9a2b-0d1e2f3a4b5c",
  query: [["page", "2"]],
};

describe("matcher chips", () => {
  it("suggests * for id-like segments", () => {
    expect(suggestGlob(target.path)).toBe("/v1/users/*/orders/*");
    expect(isIdLike("v1")).toBe(false);
    expect(isIdLike("deadbeefcafe")).toBe(true);
    expect(isIdLike("checkout")).toBe(false);
  });

  it("builds each mode and recognises it again", () => {
    expect(matcherFor(target, "path", false)).toMatchObject({ methods: ["GET"], port: null, query: [], path: { kind: "exact" } });
    expect(matcherFor(target, "exact", true).query).toEqual([
      { name: "page", value: { kind: "exact", value: "2", case_insensitive: false }, negate: false },
    ]);
    expect(matcherFor({ ...target, port: 8443 }, "glob", false).port).toBe(8443);
    for (const mode of ["exact", "path", "glob"] as const) {
      expect(modeOf(target, matcherFor(target, mode, true))).toEqual({ mode, anyMethod: true });
    }
    expect(modeOf(target, { ...matcherFor(target, "path", false), client_app: { kind: "exact", value: "Safari", case_insensitive: false } })).toBeNull();
  });
});

describe("lineDiff", () => {
  it("marks changed lines and keeps the rest", () => {
    const before = prettyForDiff('{"a":1,"b":{"c":2},"d":3}');
    const after = prettyForDiff('{"a":1,"b":{"c":5},"d":3,"e":true}');
    const changed = lineDiff(before, after).filter((l) => l.kind !== "same");
    expect(changed).toEqual([
      { kind: "removed", text: '    "c": 2' },
      { kind: "added", text: '    "c": 5' },
      { kind: "removed", text: '  "d": 3' },
      { kind: "added", text: '  "d": 3,' },
      { kind: "added", text: '  "e": true' },
    ]);
  });

  it("keeps key order and number text when pretty-printing", () => {
    expect(prettyForDiff('{"2":1.50,"1":12345678901234567890}')).toBe('{\n  "2": 1.50,\n  "1": 12345678901234567890\n}');
    expect(prettyForDiff("not json")).toBe("not json");
  });
});
