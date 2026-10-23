/// <reference types="node" />
import { readFileSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

// Vitest runs from the UI package (jsdom gives `import.meta.url` an http URL).
const css = readFileSync(path.resolve(process.cwd(), "src/styles/tokens.css"), "utf8");

/**
 * Every text colour token must reach 4.5:1 (WCAG AA) against what it is drawn
 * on, in both themes. Pills are a translucent tint over the table, so the
 * tint is composited first. Values come straight from tokens.css.
 */

type Rgba = [number, number, number, number];

function block(selector: RegExp): Map<string, string> {
  const match = selector.exec(css);
  if (!match) throw new Error(`no ${String(selector)} block`);
  const body = css.slice(match.index + match[0].length, css.indexOf("\n}", match.index));
  const vars = new Map<string, string>();
  for (const m of body.matchAll(/(--[\w-]+):\s*([^;]+);/g)) vars.set(m[1]!, m[2]!.trim());
  return vars;
}

const dark = block(/:root,\s*\[data-theme="dark"\]\s*\{/);
const light = new Map([...dark, ...block(/\[data-theme="light"\]\s*\{/)]);

function resolve(vars: Map<string, string>, name: string, depth = 0): string {
  const value = vars.get(name);
  if (value === undefined) throw new Error(`${name} is not defined`);
  const ref = /^var\((--[\w-]+)\)$/.exec(value);
  if (ref && depth < 10) return resolve(vars, ref[1]!, depth + 1);
  return value;
}

/** sRGB (0..1, gamma-encoded) and alpha of an `oklch(L C H / A)` value. */
function parse(value: string): Rgba {
  if (value === "transparent") return [0, 0, 0, 0];
  const m = /^oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+)(%?))?\s*\)$/.exec(value);
  if (!m) throw new Error(`can't read ${value}`);
  const [L, C, H] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const alpha = m[4] === undefined ? 1 : Number(m[4]) / (m[5] ? 100 : 1);
  const a = C * Math.cos((H * Math.PI) / 180);
  const b = C * Math.sin((H * Math.PI) / 180);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const mm = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const linear = [
    4.0767416621 * l - 3.3077115913 * mm + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * mm - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * mm + 1.707614701 * s,
  ].map((c) => Math.min(1, Math.max(0, c)));
  const encode = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
  return [encode(linear[0]!), encode(linear[1]!), encode(linear[2]!), alpha];
}

/** `top` drawn over `bottom` (opaque), as browsers blend: in encoded sRGB. */
function over(top: Rgba, bottom: Rgba): Rgba {
  const a = top[3];
  return [0, 1, 2].map((i) => top[i]! * a + bottom[i]! * (1 - a)).concat(1) as Rgba;
}

function luminance([r, g, b]: Rgba): number {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgba, b: Rgba): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
}

/** The colour of a stack of tokens, bottom first (the first must be opaque). */
function surface(vars: Map<string, string>, layers: string[]): Rgba {
  return layers.slice(1).reduce((acc, name) => over(parse(resolve(vars, name)), acc), parse(resolve(vars, layers[0]!)));
}

const TABLE = ["--pane-table"];
const ALT_ROW = ["--pane-table", "--pane-row-alt"];

/** [text token, the surface it sits on (bottom first)]. */
const PAIRS: [string, string[]][] = [
  ["--foreground", ["--background"]],
  ["--muted-foreground", ["--background"]],
  ["--muted-foreground", ["--pane-toolbar"]],
  ["--muted-foreground", ["--pane-header"]],
  ["--muted-foreground", ["--popover"]],
  ["--popover-foreground", ["--popover"]],
  ["--card-foreground", ["--card"]],
  ["--primary-foreground", ["--primary"]],
  ["--primary-chip-fg", ["--pane-toolbar", "--primary-chip-bg"]],
  ["--secondary-foreground", ["--secondary"]],
  ["--accent-foreground", ["--accent"]],
  ["--destructive", ["--background"]],
  ["--destructive-foreground", ["--destructive-solid"]],
  ["--selection-fg", ["--selection-bg"]],
  ["--muted-foreground", ALT_ROW],
  ["--url-scheme", ALT_ROW],
  ["--url-host", ALT_ROW],
  ["--url-port", ALT_ROW],
  ["--url-path", ALT_ROW],
  ["--url-query", ALT_ROW],
  ["--s-failed", ALT_ROW],
  ["--ssl-pinned", ALT_ROW],
  ["--ssl-passthrough", ALT_ROW],
  ["--rule", ALT_ROW],
  ...["get", "post", "put", "patch", "delete", "head", "options", "connect", "other"].map(
    (m): [string, string[]] => [`--m-${m}`, [...ALT_ROW, `--m-${m}-bg`]],
  ),
  ...["1xx", "2xx", "3xx", "4xx", "5xx", "active", "paused", "blocked", "tunnel"].map(
    (s): [string, string[]] => [`--s-${s}`, [...ALT_ROW, `--s-${s}-bg`]],
  ),
  ["--graphql", [...ALT_ROW, "--graphql-bg"]],
  ["--rule", [...TABLE, "--rule-bg"]],
  ["--rule-agent", [...TABLE, "--rule-agent-bg"]],
  ["--sent", [...ALT_ROW, "--sent-bg"]],
  ["--sent-agent", [...ALT_ROW, "--sent-agent-bg"]],
  ["--ws-sent", ["--background"]],
  ["--ws-received", ["--background"]],
  ...[
    "key",
    "string",
    "number",
    "literal",
    "keyword",
    "tag",
    "attr",
    "comment",
    "variable",
    "type",
    "punct",
    "meta",
  ].map((t): [string, string[]] => [`--syntax-${t}`, ["--background", "--code-active-line"]]),
  ...["add", "change", "del"].map((d): [string, string[]] => [`--diff-${d}`, ["--background", `--diff-${d}-bg`]]),
  ...["key", "string", "number", "literal", "punct"].flatMap((t): [string, string[]][] =>
    ["add", "change", "del"].map((d) => [`--syntax-${t}`, ["--background", `--diff-${d}-bg`]]),
  ),
  ["--foreground", ["--background", "--diff-change-bg"]],
  ["--code-gutter-fg", ["--code-gutter"]],
  ["--foreground", ["--background", "--code-selection"]],
  ["--foreground", ["--background", "--code-match-current"]],
];

describe.each([
  ["dark", dark],
  ["light", light],
])("%s theme", (theme, vars) => {
  it.each(PAIRS.map(([fg, bg]) => [fg, bg.join(" + ")] as const))("%s on %s reaches 4.5:1", (fg, bg) => {
    const ratio = contrast(parse(resolve(vars, fg)), surface(vars, bg.split(" + ")));
    expect(ratio, `${theme}: ${fg} on ${bg}: ${ratio.toFixed(2)}`).toBeGreaterThanOrEqual(4.5);
  });
});
