import { describe, expect, it } from "vitest";

import {
  align,
  convert,
  duplicate,
  indexDoc,
  insert,
  isJsonNumber,
  nodeAt,
  parseDoc,
  remove,
  removedCount,
  rename,
  ROOT_ID,
  sameJson,
  setValue,
  statusOf,
  toJson,
  type Entry,
  type JNode,
} from "./doc";

const obj = (n: JNode) => {
  if (n.t !== "obj") throw new Error(`not an object: ${n.t}`);
  return n.entries;
};
const keyId = (n: JNode, key: string) => obj(n).find((e) => e.key === key)!.id;

describe("parseDoc", () => {
  it("keeps member order, integer-like keys and repeated keys", () => {
    const doc = parseDoc('{"b":1,"2":2,"a":3,"1":4,"a":5}');
    expect(obj(doc).map((e) => e.key)).toEqual(["b", "2", "a", "1", "a"]);
    expect(toJson(doc)).toBe('{"b":1,"2":2,"a":3,"1":4,"a":5}');
  });

  it("keeps number text exactly", () => {
    const text = '[12345678901234567890,1.50,-0,1e400,0.1]';
    expect(toJson(parseDoc(text))).toBe(text);
  });

  it("gives every member and item its own id", () => {
    const doc = parseDoc('{"a":[1,2],"b":{"c":null}}');
    const ids = [...indexDoc(doc).byId.keys()];
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toHaveLength(5);
  });

  it("pretty-prints like JSON.stringify", () => {
    const text = '{"a":[1,{"b":"x"}],"c":{},"d":[]}';
    expect(toJson(parseDoc(text), 2)).toBe(JSON.stringify(JSON.parse(text), null, 2));
  });
});

describe("edits", () => {
  const base = parseDoc('{"name":"Ana","tags":["x","y"],"n":1}');

  it("are immutable and share untouched subtrees", () => {
    const id = keyId(base, "name");
    const next = setValue(base, [id], { t: "str", v: "Bea" });
    expect(toJson(base)).toBe('{"name":"Ana","tags":["x","y"],"n":1}');
    expect(toJson(next)).toBe('{"name":"Bea","tags":["x","y"],"n":1}');
    expect(obj(next)[1]).toBe(obj(base)[1]);
  });

  it("rename, remove, insert and duplicate keep ids stable", () => {
    const nameId = keyId(base, "name");
    let doc = rename(base, [nameId], "first");
    expect(keyId(doc, "first")).toBe(nameId);
    doc = remove(doc, [keyId(doc, "n")]);
    const added = insert(doc, [], nameId, { t: "bool", v: true }, "flag");
    expect(toJson(added.root)).toBe('{"first":"Ana","flag":true,"tags":["x","y"]}');
    const tagsId = keyId(added.root, "tags");
    const copy = duplicate(added.root, [tagsId]);
    expect(toJson(copy.root)).toBe('{"first":"Ana","flag":true,"tags":["x","y"],"tags_copy":["x","y"]}');
    // The copy's items are new values, not the original's.
    const original = indexDoc(base);
    const copied = nodeAt(copy.root, copy.path)!;
    expect(copied.t === "arr" && copied.items.every((i) => !original.byId.has(i.id))).toBe(true);
  });

  it("inserts pick a free key", () => {
    const one = insert(base, [], null, { t: "null" }, "name");
    expect(obj(one.root).at(-1)!.key).toBe("name2");
  });

  it("convert keeps what carries over", () => {
    expect(convert({ t: "str", v: "42" }, "num")).toEqual({ t: "num", raw: "42" });
    expect(convert({ t: "str", v: "x" }, "num")).toEqual({ t: "num", raw: "0" });
    expect(convert({ t: "num", raw: "1.50" }, "str")).toEqual({ t: "str", v: "1.50" });
    expect(toJson(convert(parseDoc("[1,2]"), "obj"))).toBe('{"0":1,"1":2}');
    expect(toJson(convert(parseDoc('{"a":1}'), "str"))).toBe('"{\\"a\\":1}"');
  });

  it("validates number text against the JSON grammar", () => {
    for (const ok of ["0", "-1", "1.50", "1e400", "12345678901234567890", "-0.0E+3"]) expect(isJsonNumber(ok)).toBe(true);
    for (const bad of ["01", "1.", ".5", "+1", "NaN", "1e", "0x10", ""]) expect(isJsonNumber(bad)).toBe(false);
  });
});

describe("comparing with the original", () => {
  it("marks added, changed and renamed rows and counts removals", () => {
    const original = parseDoc('{"a":1,"b":2,"c":3}');
    const index = indexDoc(original);
    let doc = setValue(original, [keyId(original, "a")], { t: "num", raw: "1.0" });
    doc = rename(doc, [keyId(doc, "b")], "bb");
    doc = remove(doc, [keyId(doc, "c")]);
    doc = insert(doc, [], null, { t: "null" }, "d").root;
    expect(obj(doc).map((e: Entry) => statusOf(e, index))).toEqual(["changed", "renamed", "added"]);
    expect(removedCount(doc, ROOT_ID, index, original)).toBe(1);
  });

  it("align gives a dry run's result the original's ids", () => {
    const original = parseDoc('{"a":{"x":1},"items":[1,2],"gone":true}');
    const result = align(original, parseDoc('{"a":{"x":2},"items":[1,2,3],"new":0}'));
    const index = indexDoc(original);
    expect(keyId(result, "a")).toBe(keyId(original, "a"));
    expect(obj(result).map((e) => statusOf(e, index))).toEqual(["same", "same", "added"]);
    expect(sameJson(result, parseDoc('{"a":{"x":2},"items":[1,2,3],"new":0}'))).toBe(true);
  });
});
