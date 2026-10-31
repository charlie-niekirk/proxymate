import { createContext, useContext } from "react";

/**
 * Search highlighting in bodies (M17): the filter's `body:` and bare-text
 * terms (`lib/dsl.ts` `bodySearchTerms`), provided by the Body tab for its
 * viewers. CodeMirror viewers and the JSON tree mark every match and step
 * through them with the match bar.
 */
export const BodySearchContext = createContext<readonly string[]>([]);

export function useBodySearch(): readonly string[] {
  return useContext(BodySearchContext);
}

/** At most this many matches are counted and stepped through. */
export const MATCH_LIMIT = 10_000;

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** One case-insensitive pattern for every term (longest first, so it wins). */
export function searchRegExp(terms: readonly string[]): RegExp | null {
  if (terms.length === 0) return null;
  return new RegExp(terms.map(escapeRegExp).join("|"), "gi");
}

/** Where `re` matches in `text`, up to [`MATCH_LIMIT`]. */
export function matchRanges(text: string, re: RegExp): [number, number][] {
  const out: [number, number][] = [];
  const g = new RegExp(re.source, "gi");
  for (let m = g.exec(text); m && out.length < MATCH_LIMIT; m = g.exec(text)) {
    if (m[0].length === 0) {
      g.lastIndex++;
      continue;
    }
    out.push([m.index, m.index + m[0].length]);
  }
  return out;
}

