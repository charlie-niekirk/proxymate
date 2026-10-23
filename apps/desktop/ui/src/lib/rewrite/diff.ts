/**
 * Turns the Rewrite dialog's edits into rule actions that patch only what
 * changed (`docs/PLAN.md`, "M13: Visual rewrite"):
 * - JSON: a changed scalar is a `JsonSet` of its exact text, a removed
 *   member a `JsonRemove`, an added one a `JsonSet` of its subtree, a
 *   rename both. Arrays take appends (`…/-`); any other insert, removal or
 *   reorder sets the whole array (the tree badges it). A new root type, or
 *   the "Replace whole body" toggle, is a `SetBody`.
 * - Headers (names compared case-insensitively): `SetHeader` or
 *   `RemoveHeader`; repeated ones a `RemoveHeader` and an `AddHeader` each.
 * - Query and form fields: `SetQuery`/`RemoveQuery`, `FormSet`/`FormRemove`.
 * - Status: `SetStatus`.
 *
 * `pins` (from a saved rule being reopened) are paths whose action is kept
 * even when the value equals the flow's: `json:<phase>:<pointer>`,
 * `header:<phase>:<name>`, `query:<name>`, `form:<phase>:<name>`, `status`.
 * Everything here is pure; `diff.test.ts` is its table.
 */

import type { Phase } from "@/bindings";
import { escapeToken, ROOT_ID, toJson, type Entry, type Item, type JNode } from "@/lib/json/doc";
import type { Action } from "@/lib/rules";

export type Pins = ReadonlySet<string>;

export const NO_PINS: Pins = new Set();

export const pinKey = {
  json: (phase: Phase, pointer: string) => `json:${phase}:${pointer}`,
  header: (phase: Phase, name: string) => `header:${phase}:${name.toLowerCase()}`,
  query: (name: string) => `query:${name}`,
  form: (phase: Phase, name: string) => `form:${phase}:${name}`,
  status: () => "status",
};

/** A JSON set of a node's exact text. */
export function jsonSet(phase: Phase, pointer: string, node: JNode): Action {
  return { type: "JsonSet", phase, pointer, value: null, raw: toJson(node) };
}

export interface JsonDiff {
  actions: Action[];
  /** Ids (of the member or item holding it; `ROOT_ID` for the root) of arrays set as a whole. */
  replaced: Set<number>;
  /** The root changed type: only a whole-body set can say that. */
  wholeBody: boolean;
}

/**
 * The actions that turn `original` into `edited` (both from `lib/json/doc`,
 * `edited` derived from `original` so ids line up). `prefix` is where the
 * documents sit in the body (`/variables` for GraphQL variables).
 */
export function diffJson(original: JNode, edited: JNode, phase: Phase, pins: Pins = NO_PINS, prefix = ""): JsonDiff {
  const out: JsonDiff = { actions: [], replaced: new Set(), wholeBody: false };
  const set = (pointer: string, node: JNode) => out.actions.push(jsonSet(phase, pointer, node));

  const walk = (before: JNode, after: JNode, pointer: string, holder: number) => {
    if (pointer !== prefix && pins.has(pinKey.json(phase, pointer))) return set(pointer, after);
    if (before.t !== after.t) {
      if (pointer === "") out.wholeBody = true;
      else set(pointer, after);
      return;
    }
    switch (after.t) {
      case "str":
      case "num":
      case "bool":
        if (toJson(before) !== toJson(after)) {
          if (pointer === "") out.wholeBody = true;
          else set(pointer, after);
        }
        return;
      case "null":
        return;
      case "obj": {
        const old = (before as typeof after).entries;
        const now = new Map(after.entries.map((e) => [e.id, e]));
        const was = new Set(old.map((e) => e.id));
        const removes: Action[] = [];
        const sets: [string, JNode][] = [];
        const inner: [Entry, Entry][] = [];
        for (const e of old) {
          const next = now.get(e.id);
          if (!next) removes.push({ type: "JsonRemove", phase, pointer: `${pointer}/${escapeToken(e.key)}` });
          else if (next.key !== e.key) {
            removes.push({ type: "JsonRemove", phase, pointer: `${pointer}/${escapeToken(e.key)}` });
            sets.push([`${pointer}/${escapeToken(next.key)}`, next.v]);
          } else inner.push([e, next]);
        }
        for (const e of after.entries) if (!was.has(e.id)) sets.push([`${pointer}/${escapeToken(e.key)}`, e.v]);
        // Removals first, so a rename onto a removed key (or a swap) lands.
        out.actions.push(...removes);
        for (const [e, next] of inner) walk(e.v, next.v, `${pointer}/${escapeToken(e.key)}`, e.id);
        for (const [p, v] of sets) set(p, v);
        return;
      }
      case "arr": {
        const old = (before as typeof after).items;
        const oldIds = new Set(old.map((item) => item.id));
        const appendsOnly =
          after.items.length >= old.length &&
          old.every((item, i) => after.items[i]!.id === item.id) &&
          after.items.slice(old.length).every((item) => !oldIds.has(item.id));
        if (!appendsOnly) {
          if (pointer === "") out.wholeBody = true;
          else {
            set(pointer, after);
            out.replaced.add(holder);
          }
          return;
        }
        old.forEach((item, i) => walk(item.v, after.items[i]!.v, `${pointer}/${i}`, item.id));
        for (const item of after.items.slice(old.length)) set(`${pointer}/-`, item.v);
      }
    }
  };
  walk(original, edited, prefix, ROOT_ID);
  return out;
}

/** A row of a name/value table (headers, query, form). */
export interface KvRow {
  id: number;
  name: string;
  value: string;
}

/** Headers the proxy owns: re-framing sets them. */
export const READ_ONLY_HEADERS = new Set(["content-length", "content-encoding", "transfer-encoding"]);

/** Rows grouped by name (`fold` lower-cases), in order of first appearance. */
function groups(rows: readonly KvRow[], fold: boolean): Map<string, KvRow[]> {
  const out = new Map<string, KvRow[]>();
  for (const r of rows) {
    if (r.name.trim() === "") continue;
    const k = fold ? r.name.toLowerCase() : r.name;
    out.set(k, [...(out.get(k) ?? []), r]);
  }
  return out;
}

const sameValues = (a: readonly KvRow[] | undefined, b: readonly KvRow[]) =>
  !!a && a.length === b.length && a.every((r, i) => r.value === b[i]!.value);

export function diffHeaders(original: readonly KvRow[], edited: readonly KvRow[], phase: Phase, pins: Pins = NO_PINS): Action[] {
  const before = groups(original, true);
  const after = groups(edited, true);
  const out: Action[] = [];
  for (const [k, rows] of before) {
    if (!after.has(k) && !READ_ONLY_HEADERS.has(k)) out.push({ type: "RemoveHeader", phase, name: rows[0]!.name });
  }
  for (const [k, rows] of after) {
    if (READ_ONLY_HEADERS.has(k)) continue;
    if (sameValues(before.get(k), rows) && !pins.has(pinKey.header(phase, k))) continue;
    const name = rows[0]!.name.trim();
    if (rows.length === 1) out.push({ type: "SetHeader", phase, name, value: rows[0]!.value });
    else {
      out.push({ type: "RemoveHeader", phase, name });
      for (const r of rows) out.push({ type: "AddHeader", phase, name, value: r.value });
    }
  }
  return out;
}

/**
 * Query parameters or form fields. A repeated name can be removed but not
 * edited (the editor makes its rows read-only); `problems` says if one was.
 */
function diffFields(
  original: readonly KvRow[],
  edited: readonly KvRow[],
  make: { set: (name: string, value: string) => Action; remove: (name: string) => Action; pin: (name: string) => string },
  pins: Pins,
): { actions: Action[]; problems: string[] } {
  const before = groups(original, false);
  const after = groups(edited, false);
  const actions: Action[] = [];
  const problems: string[] = [];
  for (const k of before.keys()) if (!after.has(k)) actions.push(make.remove(k));
  for (const [k, rows] of after) {
    const was = before.get(k);
    if (sameValues(was, rows) && !pins.has(make.pin(k))) continue;
    if (rows.length > 1) {
      problems.push(`${k} is repeated: it can be removed, not edited`);
      continue;
    }
    actions.push(make.set(k, rows[0]!.value));
  }
  return { actions, problems };
}

export function diffQuery(original: readonly KvRow[], edited: readonly KvRow[], pins: Pins = NO_PINS) {
  return diffFields(
    original,
    edited,
    {
      set: (name, value) => ({ type: "SetQuery", name, value }),
      remove: (name) => ({ type: "RemoveQuery", name }),
      pin: pinKey.query,
    },
    pins,
  );
}

export function diffForm(original: readonly KvRow[], edited: readonly KvRow[], phase: Phase, pins: Pins = NO_PINS) {
  return diffFields(
    original,
    edited,
    {
      set: (name, value) => ({ type: "FormSet", phase, name, value }),
      remove: (name) => ({ type: "FormRemove", phase, name }),
      pin: (name) => pinKey.form(phase, name),
    },
    pins,
  );
}

export function diffStatus(original: number, edited: number, pins: Pins = NO_PINS): Action[] {
  return original !== edited || pins.has(pinKey.status()) ? [{ type: "SetStatus", status: edited }] : [];
}

/** `a=1&b=x%20y` as rows (decoded like the engine: `+` is a space). */
export function parsePairs(text: string, id: () => number): KvRow[] {
  const decode = (s: string) => {
    try {
      return decodeURIComponent(s.replace(/\+/g, " "));
    } catch {
      return s;
    }
  };
  return text
    .split("&")
    .filter((p) => p !== "")
    .map((pair) => {
      const at = pair.indexOf("=");
      return { id: id(), name: decode(at < 0 ? pair : pair.slice(0, at)), value: at < 0 ? "" : decode(pair.slice(at + 1)) };
    });
}

/** Rows back to `a=1&b=x%20y` (for a whole-body form). */
export function formatPairs(rows: readonly KvRow[]): string {
  return rows
    .filter((r) => r.name !== "")
    .map((r) => `${encodeURIComponent(r.name)}=${encodeURIComponent(r.value)}`)
    .join("&");
}

/** The key an action is pinned by, if it is one the dialog shows. */
export function actionPin(a: Action): string | null {
  switch (a.type) {
    case "JsonSet":
      return pinKey.json(a.phase, a.pointer);
    case "SetHeader":
    case "AddHeader":
      return pinKey.header(a.phase, a.name);
    case "SetQuery":
      return pinKey.query(a.name);
    case "FormSet":
      return pinKey.form(a.phase, a.name);
    case "SetStatus":
      return pinKey.status();
    default:
      return null;
  }
}

/** Actions the dialog can show as edits; a saved rule's others are kept as they are. */
export function isVisual(a: Action): boolean {
  switch (a.type) {
    case "SetHeader":
    case "AddHeader":
    case "RemoveHeader":
    case "SetQuery":
    case "RemoveQuery":
    case "SetBody":
    case "JsonSet":
    case "JsonRemove":
    case "FormSet":
    case "FormRemove":
    case "SetStatus":
      return true;
    default:
      return false;
  }
}

/** What identifies an action's target, to tell whether rebuilt edits cover it. */
export function actionTarget(a: Action): string | null {
  switch (a.type) {
    case "JsonSet":
    case "JsonRemove":
      return pinKey.json(a.phase, a.pointer);
    case "SetHeader":
    case "AddHeader":
    case "RemoveHeader":
      return pinKey.header(a.phase, a.name);
    case "SetQuery":
    case "RemoveQuery":
      return pinKey.query(a.name);
    case "FormSet":
    case "FormRemove":
      return pinKey.form(a.phase, a.name);
    case "SetStatus":
      return pinKey.status();
    case "SetBody":
      return `body:${a.phase}`;
    default:
      return null;
  }
}

/**
 * Visual actions of a saved rule that rebuilt edits don't account for (a
 * pointer missing on this flow, say): they are kept as they are. One under
 * a JSON value the rebuilt edits set as a whole is covered by it.
 */
export function uncovered(saved: readonly Action[], rebuilt: readonly Action[]): Action[] {
  const targets = new Set(rebuilt.map(actionTarget));
  const wholes = rebuilt.filter((a) => a.type === "JsonSet").map((a) => a as Extract<Action, { type: "JsonSet" }>);
  return saved.filter((a) => {
    if (!isVisual(a)) return false;
    if (targets.has(actionTarget(a))) return false;
    if (a.type === "JsonSet" || a.type === "JsonRemove") {
      return !wholes.some((w) => w.phase === a.phase && (a.pointer.startsWith(`${w.pointer}/`) || w.pointer === ""));
    }
    return true;
  });
}

/** For tests and the tree: members and items in one list. */
export type Child = Entry | Item;
