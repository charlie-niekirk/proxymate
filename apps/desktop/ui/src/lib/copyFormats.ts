/**
 * The text forms the context menus copy: header lines, query strings,
 * cookies, hex dumps, SSE events, filter terms. Pure, so they're tested
 * without a clipboard.
 */

/** Ordered name/value pairs (headers, query, form fields); repeats kept. */
export type Pairs = [string, string][];

/** `Name: value`. */
export const headerLine = (name: string, value: string) => `${name}: ${value}`;

/** One `Name: value` line per header. */
export const headersBlock = (rows: Pairs) => rows.map(([k, v]) => headerLine(k, v)).join("\n");

/** `k: v` lines, for plain tables (TLS, certificate fields). */
export const plainBlock = (rows: Pairs) => rows.map(([k, v]) => `${k}: ${v}`).join("\n");

/** One `k=v` pair, URL-encoded as a form or query sends it. */
export const formPair = (name: string, value: string) => new URLSearchParams([[name, value]]).toString();

/** `a=1&b=2`, re-encoded. */
export const queryString = (rows: Pairs) => new URLSearchParams(rows).toString();

/** Pairs as a JSON object; a repeated name becomes an array of its values. */
export function pairsJson(rows: Pairs): string {
  const out: Record<string, string | string[]> = {};
  for (const [k, v] of rows) {
    const prev = out[k];
    out[k] = prev === undefined ? v : Array.isArray(prev) ? [...prev, v] : [prev, v];
  }
  return JSON.stringify(out, null, 2);
}

export interface Cookie {
  name: string;
  value: string;
}

/** The cookies of a `Cookie` header (`a=1; b=2`). */
export function parseCookie(header: string): Cookie[] {
  const out: Cookie[] = [];
  for (const part of header.split(";")) {
    const cookie = cookiePair(part);
    if (cookie) out.push(cookie);
  }
  return out;
}

/** The cookie a `Set-Cookie` header sets: its first pair, before the attributes. */
export function parseSetCookie(header: string): Cookie | null {
  return cookiePair(header.split(";")[0] ?? "");
}

function cookiePair(part: string): Cookie | null {
  const eq = part.indexOf("=");
  const name = (eq < 0 ? part : part.slice(0, eq)).trim();
  if (!name) return null;
  return { name, value: eq < 0 ? "" : part.slice(eq + 1).trim() };
}

/** The credentials of an `Authorization` value without its scheme (`Bearer abc` → `abc`). */
export function authToken(value: string): string | null {
  const m = /^\s*[A-Za-z][\w-]*\s+(\S.*)$/.exec(value);
  return m ? m[1]!.trim() : null;
}

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));
/** Bytes per hex dump row, as the Hex viewer shows them. */
export const HEX_WIDTH = 16;

/** `de ad be ef`. */
export function hexBytes(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => HEX[b]!).join(" ");
}

/** Width of the offset column for a body of `length` bytes. */
export const offsetDigits = (length: number) => Math.max(8, length.toString(16).length);

/** One row of a hex dump: its offset, hex bytes (a gap after eight) and ASCII (`.` for the rest). */
export function hexRow(bytes: Uint8Array, row: number): { offset: string; hex: string; ascii: string } {
  const start = row * HEX_WIDTH;
  const slice = bytes.subarray(start, start + HEX_WIDTH);
  const hex = Array.from(slice, (b, i) => (i === 8 ? ` ${HEX[b]}` : HEX[b])).join(" ");
  const ascii = Array.from(slice, (b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".")).join("");
  return { offset: start.toString(16).padStart(offsetDigits(bytes.length), "0"), hex, ascii };
}

/** An `xxd`-style dump of the whole body. */
export function hexDump(bytes: Uint8Array): string {
  const rows = Math.ceil(bytes.length / HEX_WIDTH);
  const width = HEX_WIDTH * 3; // 16 × "xx", 15 separating spaces and the gap after eight
  const lines: string[] = [];
  for (let r = 0; r < rows; r++) {
    const { offset, hex, ascii } = hexRow(bytes, r);
    lines.push(`${offset}  ${hex.padEnd(width)}  ${ascii}`);
  }
  return lines.join("\n");
}

/** Base64 of any size (btoa takes a binary string; built in chunks to stay off the stack limit). */
export function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/** A server-sent event as it travels: `event:`, `id:`, `retry:` and one `data:` per line. */
export function sseWire(e: { event: string | null; id: string | null; data: string; retry: number | null }): string {
  const lines: string[] = [];
  if (e.event !== null) lines.push(`event: ${e.event}`);
  if (e.id !== null) lines.push(`id: ${e.id}`);
  if (e.retry !== null) lines.push(`retry: ${e.retry}`);
  for (const line of e.data.split("\n")) lines.push(`data: ${line}`);
  return lines.join("\n");
}

/** Tab-separated `label  value` lines, which paste into a spreadsheet as two columns. */
export const tableText = (rows: [string, string][]) => rows.map(([k, v]) => `${k}\t${v}`).join("\n");

/**
 * A filter DSL term (`pm-core/src/store/query.rs`). Values with spaces are
 * quoted; the DSL has no escape for `"`, so a value is cut at its first one,
 * which still matches as a substring.
 */
export function dslTerm(key: string, value: string): string {
  const quote = value.indexOf('"');
  const v = quote < 0 ? value : value.slice(0, quote);
  return /\s/.test(v) || v === "" ? `${key}:"${v}"` : `${key}:${v}`;
}

/** `text` narrowed by `term`, unless it already has it. */
export function withTerm(text: string, term: string): string {
  const current = text.trim();
  if (current.split(/\s+/).includes(term)) return current;
  return current ? `${current} ${term}` : term;
}
