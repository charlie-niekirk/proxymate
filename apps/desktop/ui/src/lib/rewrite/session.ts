/**
 * The Rewrite dialog's model, kept pure for tests: what a flow offers to
 * edit (`Source`), what the user made of it (`Edits`), the rule actions
 * that follow (`buildActions`, through `diff.ts`), and rebuilding the edits
 * of a saved rule from its dry run (`rebuild`).
 */

import type { ContentKind, Header, Phase, RuleTestResult_Serialize } from "@/bindings";
import { parseGraphQlBody } from "@/lib/body/graphql";
import { align, children, newId, nodeAt, parseDoc, sameJson, setValue, toJson, type JNode, type Path } from "@/lib/json/doc";
import type { Action, Rule } from "@/lib/rules";

import {
  actionPin,
  diffForm,
  diffHeaders,
  diffJson,
  diffQuery,
  diffStatus,
  formatPairs,
  isVisual,
  parsePairs,
  pinKey,
  uncovered,
  type KvRow,
  type Pins,
} from "./diff";
import type { FlowTarget } from "./matcher";

/** Bodies above this are edited as text, not as a tree. */
export const TREE_MAX = 5 * 1024 * 1024;
/** The dialog's dry runs show bodies up to this. */
export const PREVIEW_LIMIT = 2 * 1024 * 1024;
/** Above this many actions the dialog suggests replacing the whole body. */
export const MANY_ACTIONS = 100;

export type BodySource =
  | { kind: "none"; note: string | null }
  | { kind: "json"; text: string; doc: JNode; pretty: boolean }
  /** GraphQL variables: inside a JSON body at `at`, or the `variables` query parameter of a GET. */
  | { kind: "graphql"; text: string; via: "body" | "query"; full: JNode | null; at: (string | number)[]; vars: JNode }
  | { kind: "form"; text: string; rows: KvRow[] }
  | { kind: "text"; text: string; note: string | null };

export interface Source {
  store: number;
  flow: number;
  target: FlowTarget;
  url: string;
  request: { headers: KvRow[]; query: KvRow[]; body: BodySource; contentType: string | null };
  response: { status: number; headers: KvRow[]; body: BodySource; contentType: string | null } | null;
}

export type BodyEdit =
  | { kind: "none" }
  | { kind: "json"; doc: JNode; whole: boolean }
  | { kind: "graphql"; vars: JNode; whole: boolean }
  | { kind: "form"; rows: KvRow[]; whole: boolean }
  | { kind: "text"; text: string; mode: "replace" | "find"; pairs: { find: string; replace: string }[] };

export interface Edits {
  request: { headers: KvRow[]; query: KvRow[]; body: BodyEdit };
  response: { status: number; headers: KvRow[]; body: BodyEdit } | null;
}

const rowId = () => newId();

export function headerRows(headers: readonly Header[]): KvRow[] {
  return headers.map((h) => ({ id: rowId(), name: h.name, value: h.value }));
}

/** The raw query string of a URL, without `?` or fragment. */
export function rawQuery(url: string): string {
  const q = url.indexOf("?");
  if (q < 0) return "";
  const hash = url.indexOf("#", q);
  return url.slice(q + 1, hash < 0 ? undefined : hash);
}

/** Decides how a body is edited: a JSON tree, GraphQL variables, a form table or text. */
export function bodySource(
  bytes: Uint8Array,
  contentType: string | null,
  kind: ContentKind,
  phase: Phase,
  query: KvRow[],
  /** The body as a document, if it was parsed already (in a worker). */
  doc?: JNode | null,
): BodySource {
  const ct = (contentType ?? "").toLowerCase();
  if (bytes.length === 0) {
    const vars = query.find((r) => r.name === "variables");
    if (phase === "request" && vars && query.some((r) => r.name === "query")) {
      try {
        return { kind: "graphql", text: "", via: "query", full: null, at: [], vars: parseDoc(vars.value) };
      } catch {
        // Not JSON: edited as a query parameter.
      }
    }
    return { kind: "none", note: null };
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(bytes);
  } catch {
    return { kind: "none", note: "A binary body can't be edited here." };
  }
  if (bytes.length > TREE_MAX) return { kind: "text", text, note: "Over 5 MB: edited as text." };
  if (phase === "request") {
    const ops = parseGraphQlBody(text, contentType);
    if (ops && !ct.startsWith("application/graphql")) {
      try {
        const full = parseDoc(text);
        const at: (string | number)[] = full.t === "arr" ? [0, "variables"] : ["variables"];
        return { kind: "graphql", text, via: "body", full, at, vars: nodeAtTokens(full, at) ?? { t: "null" } };
      } catch {
        // Falls through to text.
      }
    }
  }
  if (ct.includes("x-www-form-urlencoded") || kind === "formurlencoded") {
    return { kind: "form", text, rows: parsePairs(text.trim(), rowId) };
  }
  const t = text.trimStart();
  if (kind === "json" || kind === "graphql" || ct.includes("json") || t.startsWith("{") || t.startsWith("[")) {
    try {
      return { kind: "json", text, doc: doc ?? parseDoc(text), pretty: /\n/.test(text.trim()) };
    } catch {
      // Not valid JSON after all.
    }
  }
  return { kind: "text", text, note: null };
}

/** The node at keys and indices (not ids). */
export function nodeAtTokens(root: JNode, tokens: readonly (string | number)[]): JNode | undefined {
  return nodeAt(root, idPath(root, tokens) ?? [-1]);
}

function idPath(root: JNode, tokens: readonly (string | number)[]): Path | null {
  const path: number[] = [];
  let node = root;
  for (const t of tokens) {
    const kids = children(node);
    const child = node.t === "obj" ? [...node.entries].reverse().find((e) => e.key === t) : typeof t === "number" ? kids[t] : undefined;
    if (!child) return null;
    path.push(child.id);
    node = child.v;
  }
  return path;
}

/** The edits that change nothing. */
export function initialEdits(src: Source): Edits {
  return {
    request: { headers: src.request.headers, query: src.request.query, body: unchanged(src.request.body) },
    response: src.response && { status: src.response.status, headers: src.response.headers, body: unchanged(src.response.body) },
  };
}

function unchanged(body: BodySource): BodyEdit {
  switch (body.kind) {
    case "none":
      return { kind: "none" };
    case "json":
      return { kind: "json", doc: body.doc, whole: false };
    case "graphql":
      return { kind: "graphql", vars: body.vars, whole: false };
    case "form":
      return { kind: "form", rows: body.rows, whole: false };
    case "text":
      return { kind: "text", text: body.text, mode: "find", pairs: [] };
  }
}

export interface Built {
  actions: Action[];
  /** Ids of arrays set as a whole, per side, for the tree's badges. */
  replaced: Record<Phase, Set<number>>;
  problems: string[];
}

/** The rule actions for `edits` (request side first), then the `kept` ones. */
export function buildActions(src: Source, edits: Edits, pins: Pins, kept: readonly Action[] = []): Built {
  const replaced: Built["replaced"] = { request: new Set(), response: new Set() };
  const problems: string[] = [];
  const actions: Action[] = [];
  actions.push(...diffHeaders(src.request.headers, edits.request.headers, "request", pins));
  const query = diffQuery(src.request.query, edits.request.query, pins);
  problems.push(...query.problems);
  actions.push(...query.actions);
  actions.push(...bodyActions(src.request.body, edits.request.body, "request", pins, replaced.request, problems));
  if (src.response && edits.response) {
    actions.push(...diffStatus(src.response.status, edits.response.status, pins));
    actions.push(...diffHeaders(src.response.headers, edits.response.headers, "response", pins));
    actions.push(...bodyActions(src.response.body, edits.response.body, "response", pins, replaced.response, problems));
  }
  return { actions: [...actions, ...kept], replaced, problems };
}

function setBody(phase: Phase, body: string): Action {
  return { type: "SetBody", phase, body, content_type: null };
}

function bodyActions(src: BodySource, edit: BodyEdit, phase: Phase, pins: Pins, replaced: Set<number>, problems: string[]): Action[] {
  if (src.kind === "json" && edit.kind === "json") {
    if (edit.whole) return [setBody(phase, toJson(edit.doc, src.pretty ? 2 : 0))];
    const d = diffJson(src.doc, edit.doc, phase, pins);
    if (d.wholeBody) return [setBody(phase, toJson(edit.doc, src.pretty ? 2 : 0))];
    d.replaced.forEach((id) => replaced.add(id));
    return d.actions;
  }
  if (src.kind === "graphql" && edit.kind === "graphql") {
    if (src.via === "query") {
      return sameJson(src.vars, edit.vars) && !pins.has(pinKey.query("variables"))
        ? []
        : [{ type: "SetQuery", name: "variables", value: toJson(edit.vars) }];
    }
    const prefix = src.at.map((t) => `/${t}`).join("");
    if (edit.whole && src.full) {
      const path = idPath(src.full, src.at);
      const full = path ? setValue(src.full, path, edit.vars) : src.full;
      return [setBody(phase, toJson(full, /\n/.test(src.text.trim()) ? 2 : 0))];
    }
    const d = diffJson(src.vars, edit.vars, phase, pins, prefix);
    d.replaced.forEach((id) => replaced.add(id));
    return d.actions;
  }
  if (src.kind === "form" && edit.kind === "form") {
    if (edit.whole) return formatPairs(edit.rows) === formatPairs(src.rows) ? [] : [setBody(phase, formatPairs(edit.rows))];
    const d = diffForm(src.rows, edit.rows, phase, pins);
    problems.push(...d.problems);
    return d.actions;
  }
  if (src.kind === "text" && edit.kind === "text") {
    if (edit.mode === "replace") return edit.text === src.text ? [] : [setBody(phase, edit.text)];
    return edit.pairs
      .filter((p) => p.find !== "")
      .map((p) => ({ type: "ReplaceBody", phase, pattern: { kind: "exact", value: p.find, case_insensitive: false }, replacement: p.replace }));
  }
  return [];
}

/** Gives `result` rows the ids of `original` rows with the same name and occurrence. */
export function alignRows(original: readonly KvRow[], result: readonly { name: string; value: string }[], fold: boolean): KvRow[] {
  const norm = (n: string) => (fold ? n.toLowerCase() : n);
  const seen = new Map<string, number>();
  return result.map((r) => {
    const k = norm(r.name);
    const n = seen.get(k) ?? 0;
    seen.set(k, n + 1);
    const match = original.filter((o) => norm(o.name) === k)[n];
    return { id: match?.id ?? rowId(), name: r.name, value: r.value };
  });
}

export interface Rebuilt {
  edits: Edits;
  pins: Pins;
  /** Actions the dialog can't show as edits, kept as they are. */
  kept: Action[];
}

/**
 * The edits a saved rule makes to this flow, from a dry run of its visual
 * actions (`dry`). Paths those actions name are pinned, so one whose value
 * equals this flow's is kept; the rest of the rule is kept unchanged.
 */
export function rebuild(src: Source, rule: Rule, dry: RuleTestResult_Serialize): Rebuilt {
  const visual = rule.actions.filter(isVisual);
  const pins = new Set(visual.map(actionPin).filter((p): p is string => p !== null));
  const sets = (phase: Phase) => visual.some((a) => a.type === "SetBody" && a.phase === phase);
  const lost: Phase[] = [];
  const body = (source: BodySource, text: string | null, truncated: boolean, phase: Phase, query: KvRow[]): BodyEdit => {
    const start = unchanged(source);
    if (source.kind === "graphql" && source.via === "query") {
      const vars = query.find((r) => r.name === "variables");
      try {
        return vars ? { kind: "graphql", vars: align(source.vars, parseDoc(vars.value)), whole: false } : start;
      } catch {
        return start;
      }
    }
    if (text === null || truncated) {
      if (source.kind !== "none") lost.push(phase);
      return start;
    }
    try {
      switch (source.kind) {
        case "json":
          return { kind: "json", doc: align(source.doc, parseDoc(text)), whole: sets(phase) };
        case "graphql":
          return { kind: "graphql", vars: align(source.vars, nodeAtTokens(parseDoc(text), source.at) ?? { t: "null" }), whole: sets(phase) };
        case "form":
          return { kind: "form", rows: alignRows(source.rows, parsePairs(text.trim(), rowId), false), whole: sets(phase) };
        case "text":
          return sets(phase) ? { kind: "text", text, mode: "replace", pairs: [] } : start;
        case "none":
          return start;
      }
    } catch {
      lost.push(phase);
      return start;
    }
  };
  const query = alignRows(src.request.query, parsePairs(rawQuery(dry.request.url), rowId), false);
  const edits: Edits = {
    request: {
      headers: alignRows(src.request.headers, dry.request.headers, true),
      query,
      body: body(src.request.body, dry.request_body, dry.request_body_truncated, "request", query),
    },
    response:
      src.response && dry.response
        ? {
            status: dry.response.status,
            headers: alignRows(src.response.headers, dry.response.headers, true),
            body: body(src.response.body, dry.response_body, dry.response_body_truncated, "response", []),
          }
        : src.response && { status: src.response.status, headers: src.response.headers, body: unchanged(src.response.body) },
  };
  const rebuilt = buildActions(src, edits, pins).actions;
  const bodyLost = (a: Action) =>
    "phase" in a && lost.includes(a.phase) && ["SetBody", "JsonSet", "JsonRemove", "FormSet", "FormRemove"].includes(a.type);
  const kept = [
    ...rule.actions.filter((a) => !isVisual(a) || bodyLost(a)),
    ...uncovered(visual.filter((a) => !bodyLost(a)), rebuilt),
  ];
  return { edits, pins, kept };
}
