import type {
  Action_Serialize,
  FlowDetail_Serialize,
  KvMatch_Serialize,
  Matcher_Serialize,
  Pattern_Serialize,
  PresetKind,
  Rule_Serialize,
  RuleOrigin_Serialize,
} from "@/bindings";

/** Rules as the editor holds them: every field present (the serialize shape). */
export type Rule = Rule_Serialize;
export type Action = Action_Serialize;
export type ActionType = Action["type"];
export type Matcher = Matcher_Serialize;
export type Pattern = Pattern_Serialize;
export type KvMatch = KvMatch_Serialize;

/** The toolbar's network conditions rule (`pm_model::GLOBAL_THROTTLE_RULE_ID`). */
export const GLOBAL_THROTTLE_RULE_ID = "builtin-throttle";

/** Built-in rules are managed elsewhere (the toolbar) and can't be deleted. */
export function isBuiltin(rule: Pick<Rule, "id">): boolean {
  return rule.id.startsWith("builtin-");
}

export function emptyMatcher(): Matcher {
  return {
    methods: [],
    scheme: null,
    host: null,
    port: null,
    path: null,
    query: [],
    headers: [],
    client_app: null,
    response: null,
  };
}

/** A blank, unsaved user rule. */
export function newRule(): Rule {
  return {
    id: "",
    name: "New rule",
    enabled: true,
    order: 0,
    origin: { kind: "user" },
    matcher: emptyMatcher(),
    actions: [],
  };
}

/** Fills in fields a deserialize-shaped rule may omit. */
export function normalizeRule(rule: Rule): Rule {
  return {
    ...rule,
    matcher: { ...emptyMatcher(), ...rule.matcher },
  };
}

export function newPattern(kind: Pattern["kind"] = "glob", value = ""): Pattern {
  return { kind, value, case_insensitive: kind === "glob" };
}

/** Labels for the "Add action" menu, in the order shown. */
export const ACTION_TYPES: { type: ActionType; label: string; group: string }[] = [
  { type: "SetHeader", label: "Set header", group: "Headers" },
  { type: "RemoveHeader", label: "Remove header", group: "Headers" },
  { type: "ReplaceHeaderValue", label: "Replace in header", group: "Headers" },
  { type: "SetQuery", label: "Set query parameter", group: "Query" },
  { type: "RemoveQuery", label: "Remove query parameter", group: "Query" },
  { type: "SetBody", label: "Set body", group: "Body" },
  { type: "ReplaceBody", label: "Replace in body", group: "Body" },
  { type: "JsonSet", label: "JSON set", group: "Body" },
  { type: "JsonRemove", label: "JSON remove", group: "Body" },
  { type: "SetStatus", label: "Set status", group: "Status" },
  { type: "MapLocal", label: "Map Local", group: "Routing" },
  { type: "MapRemote", label: "Map Remote", group: "Routing" },
  { type: "Breakpoint", label: "Breakpoint", group: "Flow control" },
  { type: "Delay", label: "Delay", group: "Flow control" },
  { type: "Throttle", label: "Throttle", group: "Flow control" },
  { type: "Block", label: "Block", group: "Flow control" },
];

export function actionLabel(type: ActionType): string {
  return ACTION_TYPES.find((a) => a.type === type)?.label ?? type;
}

/** A new action of `type` with sensible defaults. */
export function defaultAction(type: ActionType): Action {
  switch (type) {
    case "SetHeader":
      return { type, phase: "response", name: "X-Proxymate", value: "1" };
    case "RemoveHeader":
      return { type, phase: "request", name: "" };
    case "ReplaceHeaderValue":
      return { type, phase: "request", name: "", pattern: newPattern("exact"), replacement: "" };
    case "SetQuery":
      return { type, name: "", value: "" };
    case "RemoveQuery":
      return { type, name: "" };
    case "SetBody":
      return { type, phase: "response", body: "", content_type: null };
    case "ReplaceBody":
      return { type, phase: "response", pattern: newPattern("exact"), replacement: "" };
    case "JsonSet":
      return { type, phase: "response", pointer: "/", value: null };
    case "JsonRemove":
      return { type, phase: "response", pointer: "/" };
    case "SetStatus":
      return { type, status: 200 };
    case "MapLocal":
      return { type, source: { kind: "inline", body: "" }, status: 200, headers: [], content_type: "application/json" };
    case "MapRemote":
      return { type, scheme: null, host: null, port: null, path: null, preserve_host: false };
    case "Breakpoint":
      return { type, request: true, response: false };
    case "Delay":
      return { type, phase: "request", ms: 1000 };
    case "Throttle":
      return { type, download_kbps: 780, upload_kbps: 330, latency_ms: 100 };
    case "Block":
      return { type, mode: { kind: "status", status: 403 } };
  }
}

export function patternLabel(p: Pattern | null): string {
  if (!p) return "any";
  const flag = p.case_insensitive ? " (Aa)" : "";
  switch (p.kind) {
    case "exact":
      return `= ${p.value}${flag}`;
    case "glob":
      return `${p.value}${flag}`;
    case "regex":
      return `/${p.value}/${p.case_insensitive ? "i" : ""}`;
  }
}

/** One line describing what a rule matches, for the rules list. */
export function matcherSummary(m: Matcher): string {
  const parts: string[] = [];
  if (m.methods.length) parts.push(m.methods.join("|"));
  if (m.scheme) parts.push(`${m.scheme}://`);
  parts.push(m.host ? patternLabel(m.host).replace(/^= /, "") : "*");
  if (m.port !== null) parts.push(`:${m.port}`);
  if (m.path) parts.push(patternLabel(m.path).replace(/^= /, ""));
  const extras = m.query.length + m.headers.length + (m.client_app ? 1 : 0) + (m.response ? 1 : 0);
  if (extras) parts.push(`+${extras}`);
  return parts.join(" ");
}

/** A short description of an action, for the rules list. */
export function actionSummary(a: Action): string {
  switch (a.type) {
    case "SetHeader":
      return `${a.phase} ${a.name}: ${a.value}`;
    case "RemoveHeader":
      return `remove ${a.phase} ${a.name}`;
    case "ReplaceHeaderValue":
      return `${a.phase} ${a.name} ${patternLabel(a.pattern)} → ${a.replacement}`;
    case "SetQuery":
      return `?${a.name}=${a.value}`;
    case "RemoveQuery":
      return `remove ?${a.name}`;
    case "SetBody":
      return `${a.phase} body`;
    case "ReplaceBody":
      return `${a.phase} body ${patternLabel(a.pattern)} → ${a.replacement}`;
    case "JsonSet":
      return `${a.phase} ${a.pointer} = ${JSON.stringify(a.value)}`;
    case "JsonRemove":
      return `${a.phase} remove ${a.pointer}`;
    case "SetStatus":
      return `status ${a.status}`;
    case "MapLocal":
      return `Map Local ${a.source.kind === "inline" ? "inline" : a.source.kind === "file" ? a.source.path : `${a.source.root}/`}`;
    case "MapRemote":
      return `→ ${a.scheme ? `${a.scheme}://` : ""}${a.host ?? "same host"}${a.port !== null ? `:${a.port}` : ""}${a.path ?? ""}`;
    case "Breakpoint":
      return `breakpoint ${[a.request && "request", a.response && "response"].filter(Boolean).join(" + ") || "off"}`;
    case "Delay":
      return `delay ${a.phase} ${a.ms} ms`;
    case "Throttle":
      return `throttle ${a.download_kbps}/${a.upload_kbps} kbps, ${a.latency_ms} ms`;
    case "Block":
      return a.mode.kind === "status" ? `block ${a.mode.status}` : "block (reset)";
  }
}

/** The badge text for a rule's origin. */
export function originLabel(origin: RuleOrigin_Serialize): string | null {
  switch (origin.kind) {
    case "user":
      return null;
    case "preset":
      return PRESET_LABELS[origin.preset];
    case "agent":
      return `AI · ${origin.client}`;
  }
}

export const PRESET_LABELS: Record<PresetKind, string> = {
  map_local: "Map Local",
  breakpoint: "Breakpoint",
  rewrite: "Rewrite",
  map_remote: "Map Remote",
  block: "Block",
  throttle: "Throttle",
};

/** Network conditions offered by the toolbar's throttle menu. */
export interface ThrottlePreset {
  name: string;
  download_kbps: number;
  upload_kbps: number;
  latency_ms: number;
}

export const THROTTLE_PRESETS: readonly ThrottlePreset[] = [
  { name: "LTE", download_kbps: 12_000, upload_kbps: 6_000, latency_ms: 40 },
  { name: "Fast 3G", download_kbps: 1_600, upload_kbps: 750, latency_ms: 150 },
  { name: "3G", download_kbps: 780, upload_kbps: 330, latency_ms: 100 },
  { name: "Edge", download_kbps: 240, upload_kbps: 200, latency_ms: 400 },
  { name: "Very bad network", download_kbps: 50, upload_kbps: 20, latency_ms: 800 },
];

/** The preset the global throttle rule is set to, or null when it is off (or custom). */
export function activeThrottle(rule: Rule | undefined): ThrottlePreset | null {
  if (!rule?.enabled) return null;
  const t = rule.actions.find((a) => a.type === "Throttle");
  if (!t) return null;
  return (
    THROTTLE_PRESETS.find(
      (p) => p.download_kbps === t.download_kbps && p.upload_kbps === t.upload_kbps && p.latency_ms === t.latency_ms,
    ) ?? { name: "Custom", download_kbps: t.download_kbps, upload_kbps: t.upload_kbps, latency_ms: t.latency_ms }
  );
}

/** The global throttle rule switched to `preset` (or off with null). */
export function withThrottle(rule: Rule, preset: ThrottlePreset | null): Rule {
  if (!preset) return { ...rule, enabled: false };
  const action: Action = {
    type: "Throttle",
    download_kbps: preset.download_kbps,
    upload_kbps: preset.upload_kbps,
    latency_ms: preset.latency_ms,
  };
  return { ...rule, enabled: true, actions: [action] };
}

/** Moves `from` to `to` (both indices into the same list). */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const out = [...list];
  const [item] = out.splice(from, 1);
  if (item === undefined) return out;
  out.splice(Math.max(0, Math.min(out.length, to)), 0, item);
  return out;
}

/** Parses "200-299, 404" into status ranges; null if any part is invalid. */
export function parseStatusRanges(text: string): { from: number; to: number }[] | null {
  const parts = text
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean);
  const out: { from: number; to: number }[] = [];
  for (const part of parts) {
    const m = /^(\d{3})(?:\s*[-–]\s*(\d{3}))?$/.exec(part) ?? /^(\d)xx$/i.exec(part);
    if (!m) return null;
    if (/xx$/i.test(part)) {
      const d = Number(m[1]);
      out.push({ from: d * 100, to: d * 100 + 99 });
    } else {
      const from = Number(m[1]);
      const to = m[2] ? Number(m[2]) : from;
      if (to < from) return null;
      out.push({ from, to });
    }
  }
  return out;
}

export function formatStatusRanges(ranges: { from: number; to: number }[]): string {
  return ranges
    .map((r) => (r.from === r.to ? String(r.from) : r.from % 100 === 0 && r.to === r.from + 99 ? `${r.from / 100}xx` : `${r.from}-${r.to}`))
    .join(", ");
}

const PSEUDO = /^:/;

/**
 * A `curl` command line for a recorded request. A client-side stub: bodies
 * are left out. The real exporter (bodies, HTTP/2 flags) arrives in M8.
 */
export function curlCommand(detail: Pick<FlowDetail_Serialize, "request">): string {
  const { method, url, headers } = detail.request;
  const q = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;
  const parts = ["curl"];
  if (method !== "GET") parts.push("-X", method);
  parts.push(q(url));
  for (const h of headers) {
    if (PSEUDO.test(h.name) || ["host", "content-length"].includes(h.name.toLowerCase())) continue;
    parts.push("-H", q(`${h.name}: ${h.value}`));
  }
  return parts.join(" ");
}

/** True if `text` parses as JSON. */
export function parseJson(text: string): { ok: true; value: unknown } | { ok: false; error: string } {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** Header rows editable as name/value pairs (empty names dropped on save). */
export function cleanHeaders(rows: { name: string; value: string }[]): { name: string; value: string }[] {
  return rows.filter((r) => r.name.trim() !== "").map((r) => ({ name: r.name.trim(), value: r.value }));
}

