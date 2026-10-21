import { cn } from "@/lib/utils";

/** Class names shared by the WebSocket and SSE message lists. */
export const listHeaderClass =
  "bg-pane-header text-muted-foreground grid h-[22px] shrink-0 items-center gap-2 border-b px-2 text-2xs font-medium uppercase tracking-wide";

export function rowClass(selected: boolean, index: number): string {
  return cn(
    "grid h-full cursor-default items-center gap-2 px-2 font-mono text-xs",
    selected
      ? "bg-[var(--selection-bg)] text-[var(--selection-fg)]"
      : cn("hover:bg-[var(--pane-row-hover)]", index % 2 === 1 && "bg-[var(--pane-row-alt)]"),
  );
}
