import { describe, expect, it } from "vitest";

import { evaluatePath, toJsonPath, toPointer } from "./jsonPath";

const doc = {
  items: [
    { id: 1, name: "a", tags: ["x"] },
    { id: 2, name: "b", owner: { id: 9 } },
  ],
  "a/b": { "~": true },
  "with space": 3,
};

const values = (expr: string) => {
  const r = evaluatePath(doc, expr);
  if ("error" in r) throw new Error(r.error);
  return r.matches.map((m) => m.value);
};

describe("evaluatePath", () => {
  it("follows JSON pointers", () => {
    expect(values("/items/1/name")).toEqual(["b"]);
    expect(values("/a~1b/~0")).toEqual([true]);
    expect(values("/items/5")).toEqual([]);
  });

  it("evaluates a JSONPath subset", () => {
    expect(values("$.items[*].name")).toEqual(["a", "b"]);
    expect(values("$..id")).toEqual([1, 2, 9]);
    expect(values("$.items[-1].id")).toEqual([2]);
    expect(values("$.items[0:1].id")).toEqual([1]);
    expect(values("$['with space']")).toEqual([3]);
    expect(values("$.items[0].*")).toEqual([1, "a", ["x"]]);
    expect(values("$")).toEqual([doc]);
  });

  it("reports errors", () => {
    expect(evaluatePath(doc, "items")).toHaveProperty("error");
    expect(evaluatePath(doc, "$.items[?(@.id)]")).toHaveProperty("error");
  });

  it("builds pointers and paths", () => {
    expect(toPointer(["items", 0, "a/b"])).toBe("/items/0/a~1b");
    expect(toJsonPath(["items", 0, "with space"])).toBe("$.items[0]['with space']");
  });
});
