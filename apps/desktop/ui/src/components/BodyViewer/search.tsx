import { ChevronDown, ChevronUp } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

import { MATCH_LIMIT, matchRanges } from "./searchTerms";

/** `text` with its matches marked. */
export function Highlighted({ text, re, current }: { text: string; re: RegExp | null; current?: boolean }): ReactNode {
  if (!re) return text;
  const ranges = matchRanges(text, re);
  if (ranges.length === 0) return text;
  const parts: ReactNode[] = [];
  let at = 0;
  ranges.forEach(([from, to], i) => {
    if (from > at) parts.push(text.slice(at, from));
    parts.push(
      <mark
        key={i}
        className={cn("rounded-[2px] text-inherit", current ? "bg-[var(--code-match-current)]" : "bg-[var(--code-match)]")}
        data-testid="body-match"
      >
        {text.slice(from, to)}
      </mark>,
    );
    at = to;
  });
  if (at < text.length) parts.push(text.slice(at));
  return parts;
}

/** "3 of 12 ⌃ ⌄": the matches of the filter's terms in a body, and stepping through them. */
export function MatchBar({
  count,
  current,
  onStep,
  className,
}: {
  count: number;
  /** Index of the match last stepped to, or -1. */
  current: number;
  onStep: (direction: 1 | -1) => void;
  className?: string;
}) {
  const label =
    count === 0 ? "No matches" : current < 0 ? `${count >= MATCH_LIMIT ? `${MATCH_LIMIT.toLocaleString()}+` : count.toLocaleString()} matches` : `${current + 1} of ${count.toLocaleString()}`;
  return (
    <div
      className={cn(
        "bg-popover text-popover-foreground flex items-center gap-0.5 rounded-md border py-0.5 pr-0.5 pl-2 text-[11px] shadow-sm",
        className,
      )}
      data-testid="match-bar"
    >
      <span className="text-muted-foreground mr-1 tabular-nums" data-testid="match-count">
        {label}
      </span>
      <button
        type="button"
        aria-label="Previous match"
        disabled={count === 0}
        onClick={() => onStep(-1)}
        className="hover:bg-accent rounded-sm p-0.5 disabled:opacity-40"
        data-testid="match-prev"
      >
        <ChevronUp className="size-3.5" />
      </button>
      <button
        type="button"
        aria-label="Next match"
        disabled={count === 0}
        onClick={() => onStep(1)}
        className="hover:bg-accent rounded-sm p-0.5 disabled:opacity-40"
        data-testid="match-next"
      >
        <ChevronDown className="size-3.5" />
      </button>
    </div>
  );
}
