/**
 * A line diff for the Rewrite dialog's preview. The engine's JSON edits
 * keep every other byte, so a line diff of the pretty-printed before and
 * after shows exactly the changed paths.
 */

import { parseDoc, toJson } from "@/lib/json/doc";

export type LineKind = "same" | "added" | "removed";

export interface DiffLine {
  kind: LineKind;
  text: string;
}

/** Above this many cells the middle of a diff is shown as removed + added. */
const LCS_CELLS = 4_000_000;

export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const out: DiffLine[] = a.slice(0, start).map((text) => ({ kind: "same", text }));
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  if (midA.length * midB.length > LCS_CELLS) {
    out.push(...midA.map((text) => ({ kind: "removed" as const, text })), ...midB.map((text) => ({ kind: "added" as const, text })));
  } else {
    // Longest common subsequence, then walk it.
    const n = midA.length;
    const m = midB.length;
    const lcs = new Uint32Array((n + 1) * (m + 1));
    for (let i = n - 1; i >= 0; i--) {
      for (let j = m - 1; j >= 0; j--) {
        lcs[i * (m + 1) + j] =
          midA[i] === midB[j] ? lcs[(i + 1) * (m + 1) + j + 1]! + 1 : Math.max(lcs[(i + 1) * (m + 1) + j]!, lcs[i * (m + 1) + j + 1]!);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && midA[i] === midB[j]) {
        out.push({ kind: "same", text: midA[i]! });
        i++;
        j++;
      } else if (i < n && (j === m || lcs[(i + 1) * (m + 1) + j]! >= lcs[i * (m + 1) + j + 1]!)) {
        out.push({ kind: "removed", text: midA[i]! });
        i++;
      } else {
        out.push({ kind: "added", text: midB[j]! });
        j++;
      }
    }
  }
  out.push(...a.slice(endA).map((text) => ({ kind: "same" as const, text })));
  return out;
}

/** JSON pretty-printed with its key order and number text kept; other text as is. */
export function prettyForDiff(text: string): string {
  try {
    return toJson(parseDoc(text), 2);
  } catch {
    return text;
  }
}
