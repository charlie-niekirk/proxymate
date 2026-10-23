/**
 * The Rewrite dialog's matcher chips: which requests a rewrite of this flow
 * applies to. "Ignore query" (method, host, path) is the default.
 */

import type { Scheme } from "@/bindings";
import { emptyMatcher, type Matcher } from "@/lib/rules";

export type MatchMode = "exact" | "path" | "glob";

export interface FlowTarget {
  method: string;
  scheme: Scheme;
  host: string;
  port: number;
  path: string;
  /** Decoded query pairs, in order. */
  query: [string, string][];
}

/** A path segment that looks like an id: numbers, UUIDs, long hex or tokens. */
export function isIdLike(segment: string): boolean {
  return (
    /^\d+$/.test(segment) ||
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(segment) ||
    /^[0-9a-f]{12,}$/i.test(segment) ||
    (segment.length >= 16 && /\d/.test(segment) && /^[\w-]+$/.test(segment))
  );
}

/** `/users/42/posts` → `/users/*\/posts`: id-like segments become `*`. */
export function suggestGlob(path: string): string {
  return path
    .split("/")
    .map((s) => (s !== "" && isIdLike(s) ? "*" : s))
    .join("/");
}

const defaultPort = (scheme: Scheme) => (scheme === "https" || scheme === "wss" ? 443 : 80);

export function matcherFor(target: FlowTarget, mode: MatchMode, anyMethod: boolean): Matcher {
  return {
    ...emptyMatcher(),
    methods: anyMethod ? [] : [target.method],
    host: { kind: "exact", value: target.host, case_insensitive: true },
    port: target.port === defaultPort(target.scheme) ? null : target.port,
    path:
      mode === "glob"
        ? { kind: "glob", value: suggestGlob(target.path), case_insensitive: false }
        : { kind: "exact", value: target.path, case_insensitive: false },
    query:
      mode === "exact"
        ? target.query.map(([name, value]) => ({ name, value: { kind: "exact", value, case_insensitive: false }, negate: false }))
        : [],
  };
}

/** Which chip a matcher is, if any (else it was edited by hand: "Advanced"). */
export function modeOf(target: FlowTarget, matcher: Matcher): { mode: MatchMode; anyMethod: boolean } | null {
  for (const mode of ["path", "exact", "glob"] as const) {
    for (const anyMethod of [false, true]) {
      if (JSON.stringify(matcherFor(target, mode, anyMethod)) === JSON.stringify({ ...emptyMatcher(), ...matcher })) return { mode, anyMethod };
    }
  }
  return null;
}
