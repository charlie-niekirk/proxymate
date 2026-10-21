import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, Lock, MoreHorizontal, PencilLine } from "lucide-react";
import { memo, useCallback, useEffect, useRef, type KeyboardEvent } from "react";

import { useFlowView } from "@/hooks/useFlowView";
import { backend } from "@/lib/backend";
import { formatDuration, formatSize, formatTime, type Row } from "@/lib/flow";
import { proxyStatusKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

import { MethodPill, StateDot, StatusPill, UrlText } from "./cells";
import { COLUMNS, GRID_TEMPLATE, ROW_HEIGHT, type Column } from "./columns";

const HEADER_HEIGHT = 22;
const MIN_TABLE_WIDTH = 1000;

/**
 * The flow table: a hand-built div grid virtualised with TanStack Virtual.
 * Rows come from the non-reactive RowCache; only the visible window is ever
 * fetched from Rust.
 */
export function FlowTable() {
  const { cache, version } = useFlowView();
  const len = useUi((s) => s.stats.len);
  const selectedId = useUi((s) => s.selectedId);
  const select = useUi((s) => s.select);
  const autoScroll = useUi((s) => s.autoScroll);
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectedIndex = useRef<number | null>(null);

  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; it re-renders through our version state.
  const virtualizer = useVirtualizer({
    count: len,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    paddingStart: HEADER_HEIGHT,
    overscan: 24,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items.at(-1)?.index ?? -1;

  useEffect(() => cache?.ensure(first, last), [cache, first, last, len]);

  // Auto Select: follow and select the newest flow.
  useEffect(() => {
    if (!autoScroll || len === 0) return;
    virtualizer.scrollToIndex(len - 1, { align: "end" });
    const newest = cache?.get(len - 1);
    if (newest && newest.id !== useUi.getState().selectedId) select(newest.id);
  }, [autoScroll, len, version, cache, virtualizer, select]);

  // Stable, so memoised rows that stay on screen skip re-rendering while scrolling.
  const onRowClick = useCallback(
    (index: number, row: Row | undefined) => {
      selectedIndex.current = index;
      select(row?.id ?? null);
    },
    [select],
  );

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const from = selectedIndex.current ?? first;
    const to = Math.max(0, Math.min(len - 1, from + (e.key === "ArrowDown" ? 1 : -1)));
    selectedIndex.current = to;
    select(cache?.get(to)?.id ?? null);
    virtualizer.scrollToIndex(to);
  };

  return (
    <div
      ref={scrollRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="bg-pane-table relative h-full overflow-auto outline-none"
      data-testid="flow-table"
    >
      <div className="relative" style={{ height: virtualizer.getTotalSize(), minWidth: MIN_TABLE_WIDTH }}>
        <Header />
        {items.map((item) => {
          const row = cache?.get(item.index);
          return (
            <FlowRowView
              key={item.key}
              index={item.index}
              start={item.start}
              row={row}
              selected={row !== undefined && row.id === selectedId}
              onClick={onRowClick}
            />
          );
        })}
      </div>
      {len === 0 && <EmptyState />}
    </div>
  );
}

function EmptyState() {
  const total = useUi((s) => s.stats.total);
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const proxy = status?.listening ?? "127.0.0.1:9090";
  return (
    <div className="text-muted-foreground absolute inset-0 top-[22px] flex flex-col items-center justify-center gap-2 text-sm">
      {total > 0 ? (
        "No flows match the current filter"
      ) : (
        <>
          <span>Waiting for traffic</span>
          <code className="bg-muted rounded px-2 py-1 font-mono text-xs" data-selectable>
            curl -x http://{proxy} http://example.com/
          </code>
        </>
      )}
    </div>
  );
}

function Header() {
  const sort = useUi((s) => s.sort);
  const setSort = useUi((s) => s.setSort);
  const onSort = (col: Column) => {
    if (!col.sortKey) return;
    const dir = sort.key === col.sortKey && sort.dir === "asc" ? "desc" : "asc";
    setSort({ key: col.sortKey, dir });
  };
  return (
    <div
      className="bg-pane-header text-muted-foreground sticky top-0 z-10 grid border-b text-xs font-medium"
      style={{ gridTemplateColumns: GRID_TEMPLATE, height: HEADER_HEIGHT }}
    >
      {COLUMNS.map((col) => (
        <div
          key={col.id}
          onClick={() => onSort(col)}
          className={cn(
            "flex items-center gap-0.5 truncate border-r border-border/60 px-1.5 last:border-r-0",
            col.align === "right" && "justify-end",
            col.align === "center" && "justify-center",
            col.sortKey && "hover:text-foreground",
          )}
        >
          {col.title}
          {col.sortKey && sort.key === col.sortKey && (sort.key !== "id" || sort.dir === "desc") &&
            (sort.dir === "asc" ? <ArrowUp className="size-3" /> : <ArrowDown className="size-3" />)}
        </div>
      ))}
    </div>
  );
}

interface FlowRowViewProps {
  index: number;
  start: number;
  row: Row | undefined;
  selected: boolean;
  onClick: (index: number, row: Row | undefined) => void;
}

const cell = "flex min-w-0 items-center px-1.5";

const FlowRowView = memo(function FlowRowView({ index, start, row, selected, onClick }: FlowRowViewProps) {
  return (
    <div
      data-selected={selected}
      onMouseDown={() => onClick(index, row)}
      className={cn(
        "group/row absolute top-0 left-0 grid w-full font-mono text-[11.5px] leading-none",
        index % 2 === 1 && "bg-[var(--pane-row-alt)]",
        "hover:bg-[var(--pane-row-hover)]",
        "data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
      )}
      style={{ gridTemplateColumns: GRID_TEMPLATE, height: ROW_HEIGHT, transform: `translateY(${start}px)` }}
    >
      {row ? <RowCells row={row} /> : <div data-placeholder className="text-muted-foreground/40 col-span-full px-2 leading-[22px]">…</div>}
    </div>
  );
});

function RowCells({ row }: { row: Row }) {
  const dim = "text-muted-foreground group-data-[selected=true]/row:text-selection-foreground";
  return (
    <>
      <div className={cn(cell, "justify-end tabular-nums", dim)}>{row.id + 1}</div>
      <div className={cn(cell, "justify-center")}>
        <StateDot state={row.state} />
      </div>
      <div className={cell}>
        <UrlText row={row} />
      </div>
      <div className={cn(cell, "truncate font-sans text-xs")}>
        <span className="truncate">{row.client ?? "—"}</span>
      </div>
      <div className={cn(cell, "justify-center")}>
        <MethodPill method={row.method} />
      </div>
      <div className={cn(cell, "justify-center")}>
        <StatusPill status={row.status} state={row.state} />
      </div>
      <div className={cn(cell, "justify-end tabular-nums", dim)}>{formatTime(row.startedAtMs)}</div>
      <div className={cn(cell, "justify-end tabular-nums")}>{formatDuration(row.durationMs)}</div>
      <div className={cn(cell, "justify-end tabular-nums", dim)}>{formatSize(row.reqSize)}</div>
      <div className={cn(cell, "justify-end tabular-nums")}>{formatSize(row.respSize)}</div>
      <div className={cn(cell, "justify-center", dim)}>
        {(row.scheme === "https" || row.scheme === "wss") && row.state !== "tunnel" && <Lock className="size-3" />}
      </div>
      <div className={cn(cell, "justify-center")}>
        {row.edited && <PencilLine className="size-3 text-[var(--m-patch)] group-data-[selected=true]/row:text-selection-foreground" />}
      </div>
      <div className={cn(cell, "justify-center opacity-0 group-hover/row:opacity-60")}>
        <MoreHorizontal className="size-3" />
      </div>
    </>
  );
}
