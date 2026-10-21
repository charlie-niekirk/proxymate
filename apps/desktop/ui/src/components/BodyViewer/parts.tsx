import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/** A one-line note above a body: decoding, truncation, size limits. */
export function Note({ children, tone = "muted", testId = "body-note" }: { children: ReactNode; tone?: "muted" | "warn"; testId?: string }) {
  return (
    <div
      className={cn(
        "flex shrink-0 items-center gap-2 border-b px-2 py-1 text-[11px]",
        tone === "warn" ? "bg-[var(--s-4xx-bg)] text-foreground" : "text-muted-foreground",
      )}
      data-testid={testId}
    >
      {children}
    </div>
  );
}

/** The viewer's own toolbar row (zoom, beautify, copy, …). */
export function Bar({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex h-7 shrink-0 items-center gap-1 border-b px-2 text-[11px]", className)}>{children}</div>;
}

export function Centered({ children }: { children: ReactNode }) {
  return <div className="text-muted-foreground flex h-full items-center justify-center p-4 text-center text-xs">{children}</div>;
}
