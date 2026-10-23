/**
 * An editable JSON document for the visual rewrite (the JSON editor and
 * `lib/rewrite/diff.ts`). Unlike plain JS values it keeps what an edit must
 * not lose:
 * - object members in their order, integer-like keys included (a JS
 *   object would move `"2"` before `"a"`), and repeated keys;
 * - every number's exact text (`1.50`, `12345678901234567890`);
 * - a stable id per member and item, so the expanded rows and the diff
 *   follow a member through renames and an item through array edits.
 *
 * Nodes are immutable: every edit returns a new root sharing the
 * untouched subtrees, which makes undo a stack of roots.
 */

import { parseWith, type JsonBuilder } from "@/lib/body/losslessJson";

export interface Entry {
  id: number;
  key: string;
  v: JNode;
}

export interface Item {
  id: number;
  v: JNode;
}

export type JNode =
  | { t: "obj"; entries: readonly Entry[] }
  | { t: "arr"; items: readonly Item[] }
  | { t: "str"; v: string }
  | { t: "num"; raw: string }
  | { t: "bool"; v: boolean }
  | { t: "null" };

export type JType = JNode["t"];

/** The root's id; members and items get ids from {@link newId}. */
export const ROOT_ID = 0;

let nextId = 1;

export function newId(): number {
  return nextId++;
}

/** Makes sure new ids stay above those of a document parsed elsewhere (in a worker). */
export function reserveIds(next: number) {
  if (next > nextId) nextId = next;
}

const DOC_BUILDER: JsonBuilder<JNode, Entry[], Item[]> = {
  object: () => [],
  member: (entries, key, v) => void entries.push({ id: nextId++, key, v }),
  endObject: (entries) => ({ t: "obj", entries }),
  array: () => [],
  item: (items, v) => void items.push({ id: nextId++, v }),
  endArray: (items) => ({ t: "arr", items }),
  string: (v) => ({ t: "str", v }),
  number: (raw) => ({ t: "num", raw }),
  literal: (v) => (v === null ? { t: "null" } : { t: "bool", v }),
};

/** Parses JSON text into a document; throws a `JsonSyntaxError` if it isn't JSON. */
export function parseDoc(text: string): JNode {
  return parseWith(text, DOC_BUILDER);
}

/** {@link parseDoc} for a worker: the root and the id to continue from. */
export function parseDocFrom(text: string, firstId: number): { root: JNode; nextId: number } {
  nextId = Math.max(nextId, firstId);
  const root = parseDoc(text);
  return { root, nextId };
}

/** A plain JS value (from `JSON.parse`) as a document; numbers as JS prints them. */
export function fromValue(value: unknown): JNode {
  if (value === null || value === undefined) return { t: "null" };
  if (typeof value === "string") return { t: "str", v: value };
  if (typeof value === "number") return { t: "num", raw: Number.isFinite(value) ? String(value) : "0" };
  if (typeof value === "boolean") return { t: "bool", v: value };
  if (Array.isArray(value)) return { t: "arr", items: value.map((v) => ({ id: newId(), v: fromValue(v) })) };
  if (typeof value === "object") {
    return { t: "obj", entries: Object.entries(value).map(([key, v]) => ({ id: newId(), key, v: fromValue(v) })) };
  }
  return { t: "null" };
}

/**
 * JSON text for a node, numbers exactly as held. `indent` 0 is compact;
 * otherwise members go on their own lines like `JSON.stringify(v, null, indent)`.
 */
export function toJson(node: JNode, indent = 0): string {
  const out: string[] = [];
  const pad = " ".repeat(indent);
  const write = (n: JNode, prefix: string) => {
    switch (n.t) {
      case "str":
        out.push(JSON.stringify(n.v));
        return;
      case "num":
        out.push(n.raw);
        return;
      case "bool":
        out.push(n.v ? "true" : "false");
        return;
      case "null":
        out.push("null");
        return;
      case "arr": {
        if (n.items.length === 0) return void out.push("[]");
        const inner = prefix + pad;
        out.push("[");
        n.items.forEach((item, i) => {
          if (i > 0) out.push(",");
          if (indent) out.push("\n", inner);
          write(item.v, inner);
        });
        if (indent) out.push("\n", prefix);
        out.push("]");
        return;
      }
      case "obj": {
        if (n.entries.length === 0) return void out.push("{}");
        const inner = prefix + pad;
        out.push("{");
        n.entries.forEach((e, i) => {
          if (i > 0) out.push(",");
          if (indent) out.push("\n", inner);
          out.push(JSON.stringify(e.key), indent ? ": " : ":");
          write(e.v, inner);
        });
        if (indent) out.push("\n", prefix);
        out.push("}");
      }
    }
  };
  write(node, "");
  return out.join("");
}

/** True if both nodes hold the same JSON (member order and number text count; ids don't). */
export function sameJson(a: JNode, b: JNode): boolean {
  if (a === b) return true;
  if (a.t !== b.t) return false;
  switch (a.t) {
    case "str":
      return a.v === (b as typeof a).v;
    case "num":
      return a.raw === (b as typeof a).raw;
    case "bool":
      return a.v === (b as typeof a).v;
    case "null":
      return true;
    case "arr": {
      const items = (b as typeof a).items;
      return a.items.length === items.length && a.items.every((item, i) => sameJson(item.v, items[i]!.v));
    }
    case "obj": {
      const entries = (b as typeof a).entries;
      return (
        a.entries.length === entries.length &&
        a.entries.every((e, i) => e.key === entries[i]!.key && sameJson(e.v, entries[i]!.v))
      );
    }
  }
}

export function isContainer(n: JNode): n is Extract<JNode, { t: "obj" | "arr" }> {
  return n.t === "obj" || n.t === "arr";
}

/** The members of a container as `{id, key?, v}` (no key in an array). */
export function children(n: JNode): readonly (Entry | Item)[] {
  return n.t === "obj" ? n.entries : n.t === "arr" ? n.items : [];
}

/** Escapes one pointer token (RFC 6901). */
export function escapeToken(token: string): string {
  return token.replace(/~/g, "~0").replace(/\//g, "~1");
}

/** `/a/0/b` from tokens. */
export function pointerOf(tokens: readonly (string | number)[]): string {
  return tokens.map((t) => `/${escapeToken(String(t))}`).join("");
}

// ---- Addressing ----

/** A node's address: the ids of the members and items from the root down. */
export type Path = readonly number[];

/** The node at `path`, or undefined if an id is gone. */
export function nodeAt(root: JNode, path: Path): JNode | undefined {
  let node: JNode | undefined = root;
  for (const id of path) {
    node = node && children(node).find((c) => c.id === id)?.v;
  }
  return node;
}

/** The member (with its key) or item at `path`; undefined for the root. */
export function memberAt(root: JNode, path: Path): (Entry | Item) | undefined {
  if (path.length === 0) return undefined;
  const parent = nodeAt(root, path.slice(0, -1));
  return parent && children(parent).find((c) => c.id === path[path.length - 1]);
}

/** Replaces the children of the container at `path`. */
function withChildren(root: JNode, path: Path, change: (kids: (Entry | Item)[], parent: JNode) => (Entry | Item)[]): JNode {
  return update(root, path, (node) => {
    if (node.t === "obj") return { t: "obj", entries: change([...node.entries], node) as Entry[] };
    if (node.t === "arr") return { t: "arr", items: change([...node.items], node) as Item[] };
    return node;
  });
}

/** Applies `change` to the node at `path`, copying the way down. */
export function update(root: JNode, path: Path, change: (node: JNode) => JNode): JNode {
  if (path.length === 0) return change(root);
  const [id, ...rest] = path;
  const swap = <C extends Entry | Item>(kids: readonly C[]): C[] =>
    kids.map((c) => (c.id === id ? { ...c, v: update(c.v, rest, change) } : c));
  if (root.t === "obj") return { t: "obj", entries: swap(root.entries) };
  if (root.t === "arr") return { t: "arr", items: swap(root.items) };
  return root;
}

// ---- Edits ----

/** Sets the value at `path` (the root with `[]`). */
export function setValue(root: JNode, path: Path, value: JNode): JNode {
  return update(root, path, () => value);
}

/** Renames the member at `path`; a no-op for array items. */
export function rename(root: JNode, path: Path, key: string): JNode {
  if (path.length === 0) return root;
  const id = path[path.length - 1];
  return withChildren(root, path.slice(0, -1), (kids, parent) =>
    parent.t === "obj" ? kids.map((c) => (c.id === id ? { ...(c as Entry), key } : c)) : kids,
  );
}

export function remove(root: JNode, path: Path): JNode {
  if (path.length === 0) return root;
  const id = path[path.length - 1];
  return withChildren(root, path.slice(0, -1), (kids) => kids.filter((c) => c.id !== id));
}

/** A key not yet used in `obj`, based on `base`. */
export function freeKey(obj: JNode, base: string): string {
  const used = new Set(obj.t === "obj" ? obj.entries.map((e) => e.key) : []);
  if (!used.has(base)) return base;
  for (let i = 2; ; i++) if (!used.has(`${base}${i}`)) return `${base}${i}`;
}

/**
 * Inserts a new member or item into the container at `parent`, after the
 * child `after` (or at the end with null; at the start with -1). Returns
 * the new root and the new child's path.
 */
export function insert(
  root: JNode,
  parent: Path,
  after: number | null,
  value: JNode,
  key = "key",
): { root: JNode; path: Path } {
  const container = nodeAt(root, parent);
  const id = newId();
  const child = container?.t === "obj" ? { id, key: freeKey(container, key), v: value } : { id, v: value };
  const next = withChildren(root, parent, (kids) => {
    const at = after === null ? kids.length : after === -1 ? 0 : kids.findIndex((c) => c.id === after) + 1;
    kids.splice(at, 0, child);
    return kids;
  });
  return { root: next, path: [...parent, id] };
}

/** A deep copy with fresh ids: a new value, not the original's. */
export function cloneFresh(node: JNode): JNode {
  if (node.t === "obj") return { t: "obj", entries: node.entries.map((e) => ({ id: newId(), key: e.key, v: cloneFresh(e.v) })) };
  if (node.t === "arr") return { t: "arr", items: node.items.map((i) => ({ id: newId(), v: cloneFresh(i.v) })) };
  return node;
}

/** Duplicates the member or item at `path` just after it; returns the copy's path. */
export function duplicate(root: JNode, path: Path): { root: JNode; path: Path } {
  const member = memberAt(root, path);
  if (!member) return { root, path };
  const key = "key" in member ? `${member.key}_copy` : undefined;
  return insert(root, path.slice(0, -1), member.id, cloneFresh(member.v), key);
}

/** A value of type `t` made from `node`, keeping what carries over. */
export function convert(node: JNode, t: JType): JNode {
  if (node.t === t) return node;
  const text = node.t === "str" ? node.v : node.t === "num" ? node.raw : node.t === "bool" ? String(node.v) : "";
  switch (t) {
    case "str":
      return { t, v: isContainer(node) ? toJson(node) : node.t === "null" ? "" : text };
    case "num":
      return { t, raw: isJsonNumber(text.trim()) ? text.trim() : node.t === "bool" ? (node.v ? "1" : "0") : "0" };
    case "bool":
      return { t, v: node.t === "str" ? text === "true" : node.t === "num" ? Number(node.raw) !== 0 : false };
    case "null":
      return { t };
    case "obj":
      return {
        t,
        entries: node.t === "arr" ? node.items.map((item, i) => ({ id: item.id, key: String(i), v: item.v })) : [],
      };
    case "arr":
      return { t, items: node.t === "obj" ? node.entries.map((e) => ({ id: e.id, v: e.v })) : [] };
  }
}

/** A blank value of a type, for "Add". */
export function blank(t: JType): JNode {
  switch (t) {
    case "str":
      return { t, v: "" };
    case "num":
      return { t, raw: "0" };
    case "bool":
      return { t, v: false };
    case "null":
      return { t };
    case "obj":
      return { t, entries: [] };
    case "arr":
      return { t, items: [] };
  }
}

const NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;

/** True if `text` is a JSON number literal. */
export function isJsonNumber(text: string): boolean {
  return NUMBER.test(text);
}

// ---- Comparing with the original ----

/** Where each original member or item is: its parent's id and its node. */
export interface DocIndex {
  /** id → the member or item, with its parent container's path. */
  byId: Map<number, { parent: Path; member: Entry | Item }>;
}

export function indexDoc(root: JNode): DocIndex {
  const byId = new Map<number, { parent: Path; member: Entry | Item }>();
  const walk = (node: JNode, path: Path) => {
    for (const c of children(node)) {
      byId.set(c.id, { parent: path, member: c });
      if (isContainer(c.v)) walk(c.v, [...path, c.id]);
    }
  };
  walk(root, []);
  return { byId };
}

export type RowStatus = "same" | "added" | "changed" | "renamed";

/** How the member or item `member` differs from the original. */
export function statusOf(member: Entry | Item, original: DocIndex): RowStatus {
  const was = original.byId.get(member.id);
  if (!was) return "added";
  const node = member.v;
  const old = was.member.v;
  const changed = node.t !== old.t || (!isContainer(node) && !sameJson(node, old));
  if (changed) return "changed";
  if ("key" in member && "key" in was.member && member.key !== was.member.key) return "renamed";
  return "same";
}

/** How many of the original's children of `id` (ROOT_ID for the root) are gone. */
export function removedCount(node: JNode, id: number, original: DocIndex, originalRoot: JNode): number {
  const before = id === ROOT_ID ? originalRoot : original.byId.get(id)?.member.v;
  if (!before || before.t !== node.t) return 0;
  const now = new Set(children(node).map((c) => c.id));
  return children(before).filter((c) => !now.has(c.id)).length;
}

/**
 * Gives `result` the ids of `original` where they line up: members by key
 * (the last of a repeated key, as JSON readers do), items by index. Used to
 * rebuild edits from a saved rule's dry run, where everything the rule left
 * alone must look untouched.
 */
export function align(original: JNode, result: JNode): JNode {
  if (original.t === "obj" && result.t === "obj") {
    const byKey = new Map<string, Entry>();
    for (const e of original.entries) byKey.set(e.key, e);
    const used = new Set<number>();
    return {
      t: "obj",
      entries: result.entries.map((e) => {
        const match = byKey.get(e.key);
        if (!match || used.has(match.id)) return e;
        used.add(match.id);
        return { id: match.id, key: e.key, v: align(match.v, e.v) };
      }),
    };
  }
  if (original.t === "arr" && result.t === "arr") {
    return {
      t: "arr",
      items: result.items.map((item, i) => {
        const match = original.items[i];
        return match ? { id: match.id, v: align(match.v, item.v) } : item;
      }),
    };
  }
  return result;
}
