import type { Header } from "@/bindings";

/**
 * h1 header names arrive lower-cased from the parser (the proxy still
 * forwards their original case). Shown in the conventional Title-Case;
 * names that already carry capitals are left alone.
 */
export function displayHeaderName(name: string): string {
  if (name !== name.toLowerCase()) return name;
  return name.replace(/(^|-)([a-z])/g, (_, sep: string, c: string) => sep + c.toUpperCase());
}

/** First value of a header, case-insensitively. */
export function headerValue(headers: Header[], name: string): string | undefined {
  const lower = name.toLowerCase();
  return headers.find((h) => h.name.toLowerCase() === lower)?.value;
}

/** Query string of a URL as ordered key/value pairs (repeated keys kept). */
export function queryPairs(url: string): [string, string][] {
  const q = url.indexOf("?");
  if (q < 0) return [];
  const hash = url.indexOf("#", q);
  return [...new URLSearchParams(url.slice(q + 1, hash < 0 ? undefined : hash))];
}

/** UTF-8 text of a body, or null if it looks binary. */
export function bodyText(bytes: Uint8Array): string | null {
  if (bytes.subarray(0, 8192).includes(0)) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

/** Pretty-printed JSON, or null if `text` isn't JSON. */
export function prettyJson(text: string): string | null {
  const start = text.trimStart()[0];
  if (start !== "{" && start !== "[") return null;
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return null;
  }
}

export type JsonTokenKind = "key" | "string" | "number" | "literal" | "punct";

/** Splits pretty-printed JSON into coloured runs. Whitespace stays in "punct". */
export function jsonTokens(pretty: string): { text: string; kind: JsonTokenKind }[] {
  const out: { text: string; kind: JsonTokenKind }[] = [];
  const re = /("(?:[^"\\]|\\.)*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|(true|false|null)/g;
  let last = 0;
  for (let m = re.exec(pretty); m !== null; m = re.exec(pretty)) {
    if (m.index > last) out.push({ text: pretty.slice(last, m.index), kind: "punct" });
    if (m[1] !== undefined) {
      out.push({ text: m[1], kind: m[2] ? "key" : "string" });
      if (m[2]) out.push({ text: m[2], kind: "punct" });
    } else if (m[3] !== undefined) {
      out.push({ text: m[3], kind: "number" });
    } else {
      out.push({ text: m[0], kind: "literal" });
    }
    last = re.lastIndex;
  }
  if (last < pretty.length) out.push({ text: pretty.slice(last), kind: "punct" });
  return out;
}

/**
 * An HTTP message rebuilt from its parsed parts, for the Raw tab. Not the
 * exact wire bytes: header case is normalised and framing may differ.
 */
export function rawMessage(startLine: string, headers: Header[], body: string | null): string {
  const lines = [startLine, ...headers.map((h) => `${displayHeaderName(h.name)}: ${h.value}`)];
  return `${lines.join("\n")}\n\n${body ?? ""}`;
}
