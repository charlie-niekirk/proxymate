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

/**
 * An HTTP message rebuilt from its parsed parts, for the Raw tab. Not the
 * exact wire bytes: header case is normalised and framing may differ.
 */
export function rawMessage(startLine: string, headers: Header[], body: string | null): string {
  const lines = [startLine, ...headers.map((h) => `${displayHeaderName(h.name)}: ${h.value}`)];
  return `${lines.join("\n")}\n\n${body ?? ""}`;
}
