/**
 * Autocomplete for the search box's filter DSL (parsed in Rust:
 * `pm-core/src/store/query.rs`). Suggests keys, then values for the key
 * being typed: known hosts, apps and content types come from the sidebar.
 */

export interface Suggestion {
  /** What the term becomes. */
  insert: string;
  /** Shown in the list. */
  label: string;
  detail?: string;
  /** True for a whole term: a space follows it. A key (`host:`) waits for its value. */
  complete: boolean;
}

/** What the box knows about the capture, for value suggestions. */
export interface DslContext {
  hosts: string[];
  /** Registrable domains, suggested as `*.domain`. */
  domains: string[];
  apps: string[];
}

interface Key {
  key: string;
  detail: string;
  /** Numeric keys take an operator instead of `:`. */
  numeric?: boolean;
}

export const DSL_KEYS: Key[] = [
  { key: "host", detail: "exact or glob; *.example.com includes example.com" },
  { key: "path", detail: "glob over the path, * crosses /" },
  { key: "method", detail: "GET, POST…; comma-separated" },
  { key: "status", detail: "404, 4xx or 400-499" },
  { key: "ct", detail: "content type: json, form, xml, js, css, graphql, document, media…" },
  { key: "grpc", detail: "gRPC status: 0, not_found; !0 for failed calls" },
  { key: "app", detail: "client app (substring)" },
  { key: "scheme", detail: "http, https, ws, wss" },
  { key: "state", detail: "active, paused, completed, failed, blocked, tunnel" },
  { key: "is", detail: "edited, pinned or a state" },
  { key: "header", detail: "any header line contains… (deep search)" },
  { key: "reqheader", detail: "a request header contains… (deep search)" },
  { key: "resheader", detail: "a response header contains… (deep search)" },
  { key: "body", detail: "the decoded request or response body contains… (deep search)" },
  { key: "reqbody", detail: "the request body contains… (deep search)" },
  { key: "resbody", detail: "the response body contains… (deep search)" },
  { key: "dur", detail: "duration in ms (or 2s): dur>500", numeric: true },
  { key: "size", detail: "response size: size>1mb", numeric: true },
  { key: "reqsize", detail: "request size: reqsize>10kb", numeric: true },
  { key: "id", detail: "flow number: id>=100", numeric: true },
];

const VALUES: Record<string, string[]> = {
  method: ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT"],
  status: ["2xx", "3xx", "4xx", "5xx", "200", "301", "304", "400", "401", "403", "404", "500", "502", "503"],
  ct: ["json", "form", "xml", "js", "css", "graphql", "document", "media", "other", "html", "image", "video", "font", "pdf", "binary", "grpc", "protobuf", "msgpack", "cbor"],
  grpc: ["!0", "0", "ok", "not_found", "unavailable", "internal", "unauthenticated", "permission_denied", "deadline_exceeded"],
  scheme: ["http", "https", "ws", "wss"],
  state: ["active", "paused", "completed", "failed", "blocked", "tunnel"],
  is: ["edited", "pinned", "failed", "active", "paused", "blocked", "tunnel", "completed"],
};

const ALIASES: Record<string, string> = {
  domain: "host",
  client: "app",
  type: "ct",
  content: "ct",
  proto: "scheme",
  respbody: "resbody",
  respheader: "resheader",
};

const MAX = 8;

/** The term the caret is in: where it starts and its text up to the caret. */
export function termAt(text: string, caret: number): { start: number; term: string } {
  let start = caret;
  let quotes = 0;
  for (let i = 0; i < caret; i++) if (text[i] === '"') quotes++;
  // Inside a quoted value the term runs back to the space before its quote.
  while (start > 0) {
    const c = text[start - 1]!;
    if (c === '"') quotes--;
    if (/\s/.test(c) && quotes % 2 === 0) break;
    start--;
  }
  return { start, term: text.slice(start, caret) };
}

/** Suggestions for the term at `caret`. */
export function suggest(text: string, caret: number, ctx: DslContext): Suggestion[] {
  const { term } = termAt(text, caret);
  const negate = term.startsWith("-") ? "-" : "";
  const body = term.slice(negate.length);
  const colon = body.indexOf(":");
  if (colon < 0) {
    if (/[<>=]/.test(body)) return [];
    const typed = body.toLowerCase();
    return DSL_KEYS.filter((k) => k.key.startsWith(typed))
      .flatMap((k): Suggestion[] =>
        k.numeric
          ? [">", "<"].map((op) => ({ insert: `${negate}${k.key}${op}`, label: `${k.key}${op}`, detail: k.detail, complete: false }))
          : [{ insert: `${negate}${k.key}:`, label: `${k.key}:`, detail: k.detail, complete: false }],
      )
      .slice(0, MAX);
  }
  const rawKey = body.slice(0, colon).toLowerCase();
  const key = ALIASES[rawKey] ?? rawKey;
  const value = body.slice(colon + 1);
  // A comma list: complete its last item.
  const comma = value.lastIndexOf(",");
  const done = value.slice(0, comma + 1);
  const typed = value.slice(comma + 1).toLowerCase();
  const prefix = `${negate}${body.slice(0, colon + 1)}${done}`;
  let options: { value: string; detail?: string }[];
  switch (key) {
    case "host":
      options = [
        ...ctx.domains.map((d) => ({ value: `*.${d}`, detail: "the domain and its hosts" })),
        ...ctx.hosts.map((h) => ({ value: h })),
      ];
      break;
    case "app":
      options = ctx.apps.map((a) => ({ value: a.includes(" ") ? `"${a}"` : a }));
      break;
    default:
      options = (VALUES[key] ?? []).map((v) => ({ value: v }));
  }
  const seen = new Set<string>();
  const plain = (o: { value: string }) => o.value.replace(/^"|^\*\./, "").toLowerCase();
  return options
    .filter((o) => {
      const v = plain(o);
      const match = v.startsWith(typed) || (typed.length > 1 && v.includes(typed));
      if (!match || seen.has(o.value) || o.value.toLowerCase() === typed) return false;
      seen.add(o.value);
      return true;
    })
    // What starts with the typed text first; the sort is stable.
    .sort((a, b) => Number(!plain(a).startsWith(typed)) - Number(!plain(b).startsWith(typed)))
    .slice(0, MAX)
    .map((o) => ({ insert: `${prefix}${o.value}`, label: o.value, detail: o.detail, complete: true }));
}

/** `text` with the term at `caret` replaced by `s`; returns the new text and caret. */
export function accept(text: string, caret: number, s: Suggestion): { text: string; caret: number } {
  const { start } = termAt(text, caret);
  let end = caret;
  while (end < text.length && !/\s/.test(text[end]!)) end++;
  const before = text.slice(0, start);
  const after = text.slice(end).replace(/^\s+/, "");
  const insert = s.complete ? `${s.insert} ` : s.insert;
  const rest = after ? (s.complete ? after : ` ${after}`) : "";
  return { text: before + insert + rest, caret: before.length + insert.length };
}

/** True if the query has header or body terms, which run as a deep search. */
export function hasDeepTerms(text: string): boolean {
  return /(^|\s)-?(header|headers|reqheader|resheader|respheader|req\.header|res\.header|body|reqbody|resbody|respbody|req\.body|res\.body):\S/i.test(
    text,
  );
}
