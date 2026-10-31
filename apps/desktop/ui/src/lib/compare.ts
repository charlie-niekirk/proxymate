import type { Header } from "@/bindings";

/**
 * Compare (M17): two flows side by side. Headers are compared as
 * name → values (case-insensitive names, repeated headers joined), so
 * order doesn't count; bodies go through the M13 line diff.
 */

export type KvKind = "same" | "changed" | "added" | "removed";

export interface KvDiffRow {
  name: string;
  left: string | null;
  right: string | null;
  kind: KvKind;
  /** A header that differs between any two requests (dates, ids, traces). */
  volatile: boolean;
}

/** Headers that change on every request, hidden by "Ignore volatile headers". */
export const VOLATILE_HEADERS = new Set([
  "date",
  "age",
  "expires",
  "last-modified",
  "etag",
  "x-request-id",
  "x-correlation-id",
  "x-trace-id",
  "traceparent",
  "tracestate",
  "x-amzn-requestid",
  "x-amzn-trace-id",
  "x-amz-cf-id",
  "x-amz-request-id",
  "cf-ray",
  "server-timing",
  "x-runtime",
  "x-response-time",
  "report-to",
  "nel",
]);

function grouped(headers: readonly Header[]): Map<string, { name: string; value: string }> {
  const out = new Map<string, { name: string; value: string }>();
  for (const h of headers) {
    const key = h.name.toLowerCase();
    const prev = out.get(key);
    out.set(key, prev ? { name: prev.name, value: `${prev.value}, ${h.value}` } : { name: h.name, value: h.value });
  }
  return out;
}

/** A key/value diff of two header lists, in the left's order then the right's additions. */
export function diffHeaders(left: readonly Header[], right: readonly Header[], ignoreVolatile = false): KvDiffRow[] {
  const a = grouped(left);
  const b = grouped(right);
  const rows: KvDiffRow[] = [];
  for (const [key, l] of a) {
    const r = b.get(key);
    const kind: KvKind = r === undefined ? "removed" : r.value === l.value ? "same" : "changed";
    rows.push({ name: l.name, left: l.value, right: r?.value ?? null, kind, volatile: VOLATILE_HEADERS.has(key) });
  }
  for (const [key, r] of b) {
    if (!a.has(key)) rows.push({ name: r.name, left: null, right: r.value, kind: "added", volatile: VOLATILE_HEADERS.has(key) });
  }
  return ignoreVolatile ? rows.filter((r) => !r.volatile) : rows;
}

/** How many rows differ. */
export function changedCount(rows: readonly KvDiffRow[]): number {
  return rows.filter((r) => r.kind !== "same").length;
}
