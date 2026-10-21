import { Search, X } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, type ReactNode } from "react";

import { Input } from "@/components/ui/input";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { cn } from "@/lib/utils";

/**
 * Layout shared by the WebSocket and SSE panels: a toolbar over a
 * virtualised message list beside the selected message's detail.
 */
export function StreamShell({
  title,
  status,
  controls,
  list,
  detail,
  testId,
}: {
  title: string;
  status: ReactNode;
  controls: ReactNode;
  list: ReactNode;
  detail: ReactNode;
  testId: string;
}) {
  return (
    <div className="bg-background flex h-full flex-col" data-testid={testId}>
      <div className="bg-pane-toolbar flex h-8 shrink-0 items-center gap-2 border-b px-2">
        <span className="text-xs font-semibold">{title}</span>
        <div className="text-muted-foreground flex min-w-0 flex-1 items-center gap-2 text-xs">{status}</div>
        {controls}
      </div>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel defaultSize="55" minSize={240}>
          {list}
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="45" minSize={200}>
          <div className="h-full min-h-0">{detail}</div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}

export function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <div className="relative w-48">
      <Search className="text-muted-foreground absolute top-1/2 left-2 size-3 -translate-y-1/2" />
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        spellCheck={false}
        className="h-6 pr-6 pl-6 font-mono text-xs"
      />
      {value && (
        <button
          aria-label="Clear search"
          onClick={() => onChange("")}
          className="text-muted-foreground hover:text-foreground absolute top-1/2 right-1.5 -translate-y-1/2"
        >
          <X className="size-3" />
        </button>
      )}
    </div>
  );
}

/** A pulsing "live" dot with a label, or a plain label once closed. */
export function LiveStatus({ live, label }: { live: boolean; label: string }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5" data-testid="stream-status">
      <span
        className={cn("inline-block size-1.5 shrink-0 rounded-full", live && "animate-pulse")}
        style={{ backgroundColor: live ? "var(--s-active)" : "var(--s-blocked)" }}
      />
      <span className="truncate">{label}</span>
    </span>
  );
}

const ROW_HEIGHT = 22;

/**
 * The virtualised message list: fixed 22 px rows, `row(index)` renders
 * one. It follows the end as rows arrive, like a terminal, until the user
 * scrolls up; scrolling back to the bottom follows again. The visible
 * range goes to `onRange`, for fetching just those rows.
 */
export function MessageList({
  count,
  header,
  row,
  emptyText,
  onRange,
}: {
  count: number;
  header: ReactNode;
  row: (index: number) => ReactNode;
  emptyText: string;
  onRange: (range: [number, number]) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; this re-renders on count changes.
  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items.at(-1)?.index ?? -1;

  useEffect(() => onRange([first, last]), [onRange, first, last]);
  useEffect(() => {
    if (following.current && count > 0) virtualizer.scrollToIndex(count - 1, { align: "end" });
  }, [count, virtualizer]);
  const onScroll = () => {
    const el = scrollRef.current;
    if (el) following.current = el.scrollHeight - el.scrollTop - el.clientHeight < ROW_HEIGHT * 2;
  };

  return (
    <div className="bg-pane-table flex h-full flex-col">
      {header}
      <div ref={scrollRef} onScroll={onScroll} className="relative min-h-0 flex-1 overflow-auto">
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {items.map((item) => (
            <div key={item.key} className="absolute inset-x-0" style={{ top: item.start, height: ROW_HEIGHT }}>
              {row(item.index)}
            </div>
          ))}
        </div>
        {count === 0 && <div className="text-muted-foreground p-4 text-center text-xs">{emptyText}</div>}
      </div>
    </div>
  );
}
