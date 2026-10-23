import { describe, expect, it } from "vitest";

import { align, insert, nodeAt, parseDoc, remove, rename, setValue, type JNode, type Path } from "@/lib/json/doc";

import { diffForm, diffHeaders, diffJson, diffQuery, diffStatus, parsePairs, pinKey, uncovered, type KvRow } from "./diff";

/** The id path to a pointer-like list of keys and indices. */
function at(root: JNode, ...steps: (string | number)[]): Path {
  const path: number[] = [];
  let node = root;
  for (const s of steps) {
    const child = node.t === "obj" ? node.entries.find((e) => e.key === s) : node.t === "arr" ? node.items[s as number] : undefined;
    if (!child) throw new Error(`no ${s}`);
    path.push(child.id);
    node = child.v;
  }
  return path;
}

const str = (v: string): JNode => ({ t: "str", v });
const num = (raw: string): JNode => ({ t: "num", raw });

describe("diffJson", () => {
  const original = parseDoc('{"user":{"name":"Ana","id":12345678901234567890},"items":[1,2],"flag":true}');

  it("sets only a changed scalar, with its exact text", () => {
    const edited = setValue(original, at(original, "user", "id"), num("12345678901234567891"));
    expect(diffJson(original, edited, "response").actions).toEqual([
      { type: "JsonSet", phase: "response", pointer: "/user/id", value: null, raw: "12345678901234567891" },
    ]);
  });

  it("compares numbers by their text", () => {
    const edited = setValue(original, at(original, "items", 0), num("1.0"));
    expect(diffJson(original, edited, "response").actions.map((a) => a.type === "JsonSet" && a.raw)).toEqual(["1.0"]);
    expect(diffJson(original, original, "response").actions).toEqual([]);
  });

  it("removes, adds and renames members (removals first)", () => {
    let edited = remove(original, at(original, "flag"));
    edited = rename(edited, at(edited, "user", "name"), "first");
    edited = insert(edited, at(edited, "user"), null, { t: "obj", entries: [] }, "extra").root;
    expect(diffJson(original, edited, "request").actions).toEqual([
      { type: "JsonRemove", phase: "request", pointer: "/flag" },
      { type: "JsonRemove", phase: "request", pointer: "/user/name" },
      { type: "JsonSet", phase: "request", pointer: "/user/first", value: null, raw: '"Ana"' },
      { type: "JsonSet", phase: "request", pointer: "/user/extra", value: null, raw: "{}" },
    ]);
  });

  it("a swap of two keys survives", () => {
    const doc = parseDoc('{"a":1,"b":2}');
    let edited = rename(doc, at(doc, "a"), "b");
    edited = rename(edited, [doc.t === "obj" ? doc.entries[1]!.id : 0], "a");
    const pointers = diffJson(doc, edited, "response").actions.map((a) => `${a.type} ${"pointer" in a ? a.pointer : ""}`);
    expect(pointers).toEqual(["JsonRemove /a", "JsonRemove /b", "JsonSet /b", "JsonSet /a"]);
  });

  it("appends to arrays, and sets other array changes whole with a badge", () => {
    const appended = insert(original, at(original, "items"), null, num("3")).root;
    const d = diffJson(original, appended, "response");
    expect(d.actions).toEqual([{ type: "JsonSet", phase: "response", pointer: "/items/-", value: null, raw: "3" }]);
    expect(d.replaced.size).toBe(0);

    const items = at(original, "items");
    const inserted = insert(original, items, -1, num("0")).root;
    const whole = diffJson(original, inserted, "response");
    expect(whole.actions).toEqual([{ type: "JsonSet", phase: "response", pointer: "/items", value: null, raw: "[0,1,2]" }]);
    expect([...whole.replaced]).toEqual([items[0]]);

    const shorter = remove(original, at(original, "items", 1));
    expect(diffJson(original, shorter, "response").actions[0]).toMatchObject({ pointer: "/items", raw: "[1]" });
  });

  it("edits inside array items keep their index", () => {
    const doc = parseDoc('[{"a":1},{"a":2}]');
    const edited = setValue(doc, at(doc, 1, "a"), num("5"));
    expect(diffJson(doc, edited, "response").actions).toEqual([
      { type: "JsonSet", phase: "response", pointer: "/1/a", value: null, raw: "5" },
    ]);
  });

  it("a new root type is a whole-body change", () => {
    expect(diffJson(original, parseDoc("[]"), "response").wholeBody).toBe(true);
    expect(diffJson(parseDoc('"a"'), parseDoc('"b"'), "response").wholeBody).toBe(true);
  });

  it("escapes pointer tokens", () => {
    const doc = parseDoc('{"a/b":{"c~d":1}}');
    const edited = setValue(doc, at(doc, "a/b", "c~d"), num("2"));
    expect(diffJson(doc, edited, "response").actions[0]).toMatchObject({ pointer: "/a~1b/c~0d" });
  });

  it("works under a prefix (GraphQL variables)", () => {
    const vars = parseDoc('{"first":3}');
    const edited = setValue(vars, at(vars, "first"), num("10"));
    expect(diffJson(vars, edited, "request", undefined, "/variables").actions).toEqual([
      { type: "JsonSet", phase: "request", pointer: "/variables/first", value: null, raw: "10" },
    ]);
    expect(diffJson(parseDoc("null"), vars, "request", undefined, "/variables").actions).toEqual([
      { type: "JsonSet", phase: "request", pointer: "/variables", value: null, raw: '{"first":3}' },
    ]);
  });

  it("pins keep a saved set whose value equals the flow's", () => {
    const pins = new Set([pinKey.json("response", "/user/name")]);
    expect(diffJson(original, original, "response", pins).actions).toEqual([
      { type: "JsonSet", phase: "response", pointer: "/user/name", value: null, raw: '"Ana"' },
    ]);
  });

  it("reconstructs a saved rule from its dry run", () => {
    // The rule set /user/name, appended to /items and removed /flag.
    const result = align(original, parseDoc('{"user":{"name":"Bea","id":12345678901234567890},"items":[1,2,9]}'));
    const pins = new Set([pinKey.json("response", "/user/name"), pinKey.json("response", "/items/-")]);
    expect(diffJson(original, result, "response", pins).actions).toEqual([
      { type: "JsonRemove", phase: "response", pointer: "/flag" },
      { type: "JsonSet", phase: "response", pointer: "/user/name", value: null, raw: '"Bea"' },
      { type: "JsonSet", phase: "response", pointer: "/items/-", value: null, raw: "9" },
    ]);
    expect(nodeAt(result, at(result, "user", "name"))).toEqual(str("Bea"));
  });
});

let nextRow = 1;
const rows = (...pairs: [string, string][]): KvRow[] => pairs.map(([name, value]) => ({ id: nextRow++, name, value }));

describe("diffHeaders", () => {
  const original = rows(["Accept", "*/*"], ["X-Tag", "a"], ["X-Tag", "b"], ["Content-Length", "10"]);

  it("compares names case-insensitively", () => {
    expect(diffHeaders(original, rows(["accept", "*/*"], ["x-tag", "a"], ["x-tag", "b"]), "request")).toEqual([]);
  });

  it("sets, removes, and re-adds repeated headers", () => {
    const edited = rows(["Accept", "application/json"], ["X-Tag", "a"], ["X-Tag", "c"], ["X-New", "1"]);
    expect(diffHeaders(original, edited, "request")).toEqual([
      { type: "SetHeader", phase: "request", name: "Accept", value: "application/json" },
      { type: "RemoveHeader", phase: "request", name: "X-Tag" },
      { type: "AddHeader", phase: "request", name: "X-Tag", value: "a" },
      { type: "AddHeader", phase: "request", name: "X-Tag", value: "c" },
      { type: "SetHeader", phase: "request", name: "X-New", value: "1" },
    ]);
    expect(diffHeaders(original, rows(["Accept", "*/*"]), "response")).toEqual([
      { type: "RemoveHeader", phase: "response", name: "X-Tag" },
    ]);
  });

  it("leaves the framing headers to the proxy", () => {
    expect(diffHeaders(original, [...original.slice(0, 3), ...rows(["Content-Length", "99"])], "request")).toEqual([]);
  });

  it("keeps a pinned header", () => {
    const pins = new Set([pinKey.header("request", "ACCEPT")]);
    expect(diffHeaders(original, original, "request", pins)).toEqual([
      { type: "SetHeader", phase: "request", name: "Accept", value: "*/*" },
    ]);
  });
});

describe("query, form and status", () => {
  const ids = () => nextRow++;

  it("parses pairs like the engine", () => {
    expect(parsePairs("a=1&b=x+y%21&c&&d=", ids).map((r) => [r.name, r.value])).toEqual([
      ["a", "1"],
      ["b", "x y!"],
      ["c", ""],
      ["d", ""],
    ]);
  });

  it("sets and removes fields; a repeated one can only be removed", () => {
    const original = parsePairs("q=a&page=1&tag=x&tag=y", ids);
    const edited = [...original.filter((r) => r.name !== "page").map((r) => (r.name === "q" ? { ...r, value: "b" } : r)), { id: ids(), name: "new", value: "1" }];
    expect(diffQuery(original, edited)).toEqual({
      actions: [
        { type: "RemoveQuery", name: "page" },
        { type: "SetQuery", name: "q", value: "b" },
        { type: "SetQuery", name: "new", value: "1" },
      ],
      problems: [],
    });
    const tagless = original.filter((r) => r.name !== "tag");
    expect(diffForm(original, tagless, "request").actions).toEqual([{ type: "FormRemove", phase: "request", name: "tag" }]);
    const retagged = original.map((r) => (r.name === "tag" ? { ...r, value: "z" } : r));
    expect(diffForm(original, retagged, "request").problems).toEqual(["tag is repeated: it can be removed, not edited"]);
  });

  it("status", () => {
    expect(diffStatus(200, 200)).toEqual([]);
    expect(diffStatus(200, 404)).toEqual([{ type: "SetStatus", status: 404 }]);
    expect(diffStatus(200, 200, new Set([pinKey.status()]))).toEqual([{ type: "SetStatus", status: 200 }]);
  });
});

describe("uncovered", () => {
  it("keeps saved actions the rebuilt edits don't account for", () => {
    const saved = [
      { type: "JsonSet", phase: "response", pointer: "/missing/x", value: null, raw: "1" },
      { type: "JsonRemove", phase: "response", pointer: "/items/1" },
      { type: "SetHeader", phase: "response", name: "X-A", value: "1" },
      { type: "Delay", phase: "response", ms: 10 },
    ] as const;
    const rebuilt = [
      { type: "JsonSet", phase: "response", pointer: "/items", value: null, raw: "[1]" },
      { type: "SetHeader", phase: "response", name: "x-a", value: "1" },
    ] as const;
    expect(uncovered([...saved], [...rebuilt])).toEqual([saved[0]]);
  });
});
