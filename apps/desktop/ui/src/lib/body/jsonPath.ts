/**
 * Path filters for the JSON viewer: an RFC 6901 pointer (`/items/0/name`)
 * or a JSONPath subset (`$.items[*].name`, `$..id`, `$['a b'][0:2]`).
 * Works on `parseLossless` values: a JsonNumber is a leaf, not an object.
 */

import { isJsonNumber } from "./losslessJson";

export type PathSegment = string | number;

export interface PathMatch {
  path: PathSegment[];
  value: unknown;
}

export type PathResult = { matches: PathMatch[] } | { error: string };

type Step =
  | { kind: "key"; key: string }
  | { kind: "index"; index: number }
  | { kind: "slice"; start: number | null; end: number | null }
  | { kind: "wildcard" }
  | { kind: "descend"; then: Step };

export function evaluatePath(root: unknown, expr: string): PathResult {
  const e = expr.trim();
  if (e === "" || e === "$") return { matches: [{ path: [], value: root }] };
  if (e.startsWith("/")) return pointer(root, e);
  if (!e.startsWith("$")) return { error: "Start with $ (JSONPath) or / (JSON pointer)" };
  let steps: Step[];
  try {
    steps = parseJsonPath(e);
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
  let current: PathMatch[] = [{ path: [], value: root }];
  for (const step of steps) current = current.flatMap((m) => apply(step, m));
  return { matches: current };
}

/** A JSON object (not an array, null or a JsonNumber). */
function isObject(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === "object" && !Array.isArray(v) && !isJsonNumber(v);
}

function pointer(root: unknown, expr: string): PathResult {
  const tokens = expr
    .slice(1)
    .split("/")
    .map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
  let value = root;
  const path: PathSegment[] = [];
  for (const t of tokens) {
    if (Array.isArray(value) && /^(0|[1-9]\d*)$/.test(t) && Number(t) < value.length) {
      value = value[Number(t)];
      path.push(Number(t));
    } else if (isObject(value) && Object.hasOwn(value, t)) {
      value = value[t];
      path.push(t);
    } else {
      return { matches: [] };
    }
  }
  return { matches: [{ path, value }] };
}

function parseJsonPath(e: string): Step[] {
  const steps: Step[] = [];
  let i = 1;
  const fail = (msg: string): never => {
    throw new Error(`${msg} at position ${i}`);
  };
  const name = () => {
    const m = /^[A-Za-z_$\u00a0-\uffff][\w$\u00a0-\uffff-]*/.exec(e.slice(i));
    if (!m) fail("Expected a name");
    i += m![0].length;
    return m![0];
  };
  const bracket = (): Step => {
    i++; // [
    const rest = e.slice(i);
    let m: RegExpExecArray | null;
    let step: Step;
    if ((m = /^\s*\*\s*\]/.exec(rest))) step = { kind: "wildcard" };
    else if ((m = /^\s*(-?\d*)\s*:\s*(-?\d*)\s*\]/.exec(rest))) {
      step = { kind: "slice", start: m[1] ? Number(m[1]) : null, end: m[2] ? Number(m[2]) : null };
    } else if ((m = /^\s*(-?\d+)\s*\]/.exec(rest))) step = { kind: "index", index: Number(m[1]) };
    else if ((m = /^\s*'((?:[^'\\]|\\.)*)'\s*\]/.exec(rest)) || (m = /^\s*"((?:[^"\\]|\\.)*)"\s*\]/.exec(rest))) {
      step = { kind: "key", key: m[1]!.replace(/\\(.)/g, "$1") };
    } else return fail("Unsupported bracket expression");
    i += m[0].length;
    return step;
  };
  while (i < e.length) {
    if (e.startsWith("..", i)) {
      i += 2;
      const then: Step = e[i] === "[" ? bracket() : e[i] === "*" ? (i++, { kind: "wildcard" }) : { kind: "key", key: name() };
      steps.push({ kind: "descend", then });
    } else if (e[i] === ".") {
      i++;
      if (e[i] === "*") {
        i++;
        steps.push({ kind: "wildcard" });
      } else steps.push({ kind: "key", key: name() });
    } else if (e[i] === "[") {
      steps.push(bracket());
    } else {
      fail("Unexpected character");
    }
  }
  return steps;
}

function children(m: PathMatch): PathMatch[] {
  const v = m.value;
  if (Array.isArray(v)) return v.map((value, i) => ({ path: [...m.path, i], value }));
  if (isObject(v)) return Object.entries(v).map(([k, value]) => ({ path: [...m.path, k], value }));
  return [];
}

function apply(step: Step, m: PathMatch): PathMatch[] {
  const v = m.value;
  switch (step.kind) {
    case "key":
      return isObject(v) && Object.hasOwn(v, step.key) ? [{ path: [...m.path, step.key], value: v[step.key] }] : [];
    case "index": {
      if (!Array.isArray(v)) return [];
      const i = step.index < 0 ? v.length + step.index : step.index;
      return i >= 0 && i < v.length ? [{ path: [...m.path, i], value: v[i] }] : [];
    }
    case "slice": {
      if (!Array.isArray(v)) return [];
      const norm = (n: number | null, d: number) => (n === null ? d : n < 0 ? Math.max(0, v.length + n) : Math.min(n, v.length));
      const out: PathMatch[] = [];
      for (let i = norm(step.start, 0); i < norm(step.end, v.length); i++) out.push({ path: [...m.path, i], value: v[i] });
      return out;
    }
    case "wildcard":
      return children(m);
    case "descend": {
      const out: PathMatch[] = [];
      const visit = (node: PathMatch) => {
        out.push(...apply(step.then, node));
        for (const child of children(node)) visit(child);
      };
      visit(m);
      return out;
    }
  }
}

/** `/items/0/na~1me`. */
export function toPointer(path: PathSegment[]): string {
  return path.map((s) => `/${String(s).replace(/~/g, "~0").replace(/\//g, "~1")}`).join("");
}

/** `$.items[0]['na/me']`. */
export function toJsonPath(path: PathSegment[]): string {
  return (
    "$" +
    path
      .map((s) =>
        typeof s === "number" ? `[${s}]` : /^[A-Za-z_$][\w$]*$/.test(s) ? `.${s}` : `['${s.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}']`,
      )
      .join("")
  );
}
