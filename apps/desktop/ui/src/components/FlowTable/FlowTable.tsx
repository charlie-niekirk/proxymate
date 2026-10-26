import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, MoreHorizontal, PencilLine, Pin } from "lucide-react";
import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from "react";

import { AppIcon } from "@/components/AppIcon";
import { CopyMenu } from "@/components/menus/items";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useFlowView } from "@/hooks/useFlowView";
import { FLOW_TABLE_ID, focusInspector } from "@/lib/focus";
import { backend } from "@/lib/backend";
import { formatDuration, formatSize, formatTime, versionLabel, type Row } from "@/lib/flow";
import { proxyStatusKey } from "@/lib/query";
import { sslTitle } from "@/lib/tls";
import { cn } from "@/lib/utils";
import { usePrefs } from "@/store/prefs";
import { useUi } from "@/store/ui";

import { MethodPill, SslIcon, StateDot, StatusPill, UrlText } from "./cells";
import {
  COLUMNS,
  ROW_HEIGHT,
  gridTemplate,
  minTableWidth,
  placeColumns,
  type ColumnId,
  type PlacedColumn,
} from "./columns";
import { FlowContextMenu } from "./FlowContextMenu";

const HEADER_HEIGHT = 22;

/**
 * The flow table: a hand-built div grid virtualised with TanStack Virtual.
 * Rows come from the non-reactive RowCache; only the visible window is ever
 * fetched from Rust. Columns are resized, reordered, hidden and sorted from
 * the header; the layout persists (store/prefs.ts).
 *
 * Keys: ↑/↓, PgUp/PgDn, Home/End move the selection; Enter focuses the
 * inspector.
 */
export function FlowTable() {
  const { cache, version } = useFlowView();
  const len = useUi((s) => s.stats.len);
  const selectedId = useUi((s) => s.selectedId);
  const select = useUi((s) => s.select);
  const autoScroll = useUi((s) => s.autoScroll);
  const layout = usePrefs((s) => s.columns);
  const columns = useMemo(() => placeColumns(layout), [layout]);
  const template = useMemo(() => gridTemplate(columns), [columns]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectedIndex = useRef<number | null>(null);
  // A keyboard move to a row that isn't loaded yet: selected once it is.
  const pendingIndex = useRef<number | null>(null);
  // The row under a right-click: set by the row, read by the menu trigger.
  const pressedRow = useRef<Row | null>(null);
  const [menu, setMenu] = useState<{ row: Row; column: ColumnId | null } | null>(null);

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

  // A keyboard move landed on a row that has loaded since.
  useEffect(() => {
    const pending = pendingIndex.current;
    if (pending === null) return;
    const row = cache?.get(pending);
    if (row) {
      pendingIndex.current = null;
      select(row.id);
    }
  }, [version, cache, select]);

  // Stable, so memoised rows that stay on screen skip re-rendering while scrolling.
  const onRowClick = useCallback(
    (index: number, row: Row | undefined) => {
      selectedIndex.current = index;
      pendingIndex.current = null;
      select(row?.id ?? null);
    },
    [select],
  );

  const onRowContextMenu = useCallback(
    (index: number, row: Row | undefined) => {
      pressedRow.current = row ?? null;
      selectedIndex.current = index;
      select(row?.id ?? null);
    },
    [select],
  );

  // Rows bubble their right-click first; outside a row there is no menu.
  // The cell under the cursor (by its place in the row's grid) picks the
  // menu's "Copy <column>" item.
  const onTableContextMenu = (e: MouseEvent) => {
    const row = pressedRow.current;
    pressedRow.current = null;
    if (!row) {
      e.preventDefault();
      return;
    }
    const cellEl = (e.target as Element).closest<HTMLElement>('[role="row"] > div');
    const at = cellEl?.parentElement ? Array.prototype.indexOf.call(cellEl.parentElement.children, cellEl) : -1;
    setMenu({ row, column: columns[at]?.id ?? null });
  };

  const moveTo = (index: number) => {
    if (len === 0) return;
    const to = Math.max(0, Math.min(len - 1, index));
    selectedIndex.current = to;
    const row = cache?.get(to);
    pendingIndex.current = row ? null : to;
    if (row) select(row.id);
    virtualizer.scrollToIndex(to, { align: "auto" });
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    // Where the selection is now: the remembered index if it still holds
    // that flow, else wherever the flow is among the loaded rows.
    let from = selectedIndex.current;
    if (selectedId !== null && (from === null || cache?.get(from)?.id !== selectedId)) {
      from = cache?.indexOf(selectedId) ?? null;
    }
    const page = Math.max(1, Math.floor(((scrollRef.current?.clientHeight ?? 0) - HEADER_HEIGHT) / ROW_HEIGHT) - 1);
    switch (e.key) {
      case "ArrowDown":
        moveTo(from === null ? first : from + 1);
        break;
      case "ArrowUp":
        moveTo(from === null ? last : from - 1);
        break;
      case "PageDown":
        moveTo((from ?? first) + page);
        break;
      case "PageUp":
        moveTo((from ?? first) - page);
        break;
      case "Home":
        moveTo(0);
        break;
      case "End":
        moveTo(len - 1);
        break;
      case "Enter":
        if (selectedId !== null) focusInspector();
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={onTableContextMenu}>
        <div
          ref={scrollRef}
          id={FLOW_TABLE_ID}
          tabIndex={0}
          role="grid"
          aria-label="Flows"
          aria-rowcount={len}
          onKeyDown={onKeyDown}
          className="bg-pane-table focus-visible:ring-ring/70 relative h-full overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-inset"
          data-testid="flow-table"
        >
          <div
            className="relative"
            style={{ height: virtualizer.getTotalSize(), minWidth: minTableWidth(columns) }}
          >
            <Header columns={columns} template={template} />
            {items.map((item) => {
              const row = cache?.get(item.index);
              return (
                <FlowRowView
                  key={item.key}
                  index={item.index}
                  start={item.start}
                  row={row}
                  columns={columns}
                  template={template}
                  selected={row !== undefined && row.id === selectedId}
                  onClick={onRowClick}
                  onContextMenu={onRowContextMenu}
                />
              );
            })}
          </div>
          {len === 0 && <EmptyState />}
        </div>
      </ContextMenuTrigger>
      <FlowContextMenu row={menu?.row ?? null} column={menu?.column ?? null} />
    </ContextMenu>
  );
}

function EmptyState() {
  const total = useUi((s) => s.stats.total);
  const deep = useUi((s) => s.deep);
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const proxy = status?.listening ?? "127.0.0.1:9090";
  return (
    <div className="text-muted-foreground absolute inset-0 top-[22px] flex flex-col items-center justify-center gap-2 text-sm">
      {deep?.running ? (
        "Searching headers and bodies…"
      ) : total > 0 ? (
        "No flows match the current filter"
      ) : (
        <>
          <span>Waiting for traffic</span>
          <CopyMenu items={[{ label: "Copy command", text: `curl -x http://${proxy} http://example.com/`, what: "command" }]}>
            <code className="bg-muted rounded px-2 py-1 font-mono text-xs" data-selectable data-testid="empty-curl">
              curl -x http://{proxy} http://example.com/
            </code>
          </CopyMenu>
        </>
      )}
    </div>
  );
}

/** Column headers: click sorts, drag the right edge resizes, drag a header reorders, right-click picks columns. */
function Header({ columns, template }: { columns: PlacedColumn[]; template: string }) {
  const sort = useUi((s) => s.sort);
  const setSort = useUi((s) => s.setSort);
  const layout = usePrefs((s) => s.columns);
  const setHidden = usePrefs((s) => s.setColumnHidden);
  const resetColumns = usePrefs((s) => s.resetColumns);
  const moveColumn = usePrefs((s) => s.moveColumn);
  const [dropTarget, setDropTarget] = useState<ColumnId | null>(null);
  const onSort = (col: PlacedColumn) => {
    if (!col.sortKey) return;
    const dir = sort.key === col.sortKey && sort.dir === "asc" ? "desc" : "asc";
    setSort({ key: col.sortKey, dir });
  };
  const onDrop = (e: DragEvent, target: ColumnId) => {
    e.preventDefault();
    setDropTarget(null);
    const moved = e.dataTransfer.getData("application/x-pm-column") as ColumnId;
    if (moved && moved !== target) moveColumn(moved, target);
  };
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="row"
          className="bg-pane-header text-muted-foreground sticky top-0 z-10 grid border-b text-xs font-medium"
          style={{ gridTemplateColumns: template, height: HEADER_HEIGHT }}
          data-testid="flow-table-header"
        >
          {columns.map((col) => {
            const sorted = col.sortKey && sort.key === col.sortKey && (sort.key !== "id" || sort.dir === "desc");
            return (
              <div
                key={col.id}
                role="columnheader"
                aria-sort={sorted ? (sort.dir === "asc" ? "ascending" : "descending") : undefined}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData("application/x-pm-column", col.id);
                  e.dataTransfer.effectAllowed = "move";
                }}
                onDragOver={(e) => {
                  if (!e.dataTransfer.types.includes("application/x-pm-column")) return;
                  e.preventDefault();
                  setDropTarget(col.id);
                }}
                onDragLeave={() => setDropTarget((t) => (t === col.id ? null : t))}
                onDrop={(e) => onDrop(e, col.id)}
                onClick={() => onSort(col)}
                className={cn(
                  "relative flex min-w-0 items-center gap-0.5 border-r border-border/60 px-1.5 last:border-r-0",
                  col.align === "right" && "justify-end",
                  col.align === "center" && "justify-center",
                  col.sortKey && "hover:text-foreground",
                  dropTarget === col.id && "shadow-[inset_2px_0_0_var(--primary)]",
                )}
                data-column={col.id}
              >
                <span className="truncate">{col.title}</span>
                {sorted && (sort.dir === "asc" ? <ArrowUp className="size-3 shrink-0" /> : <ArrowDown className="size-3 shrink-0" />)}
                <ResizeHandle column={col} />
              </div>
            );
          })}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="w-48" data-testid="column-menu">
        <ContextMenuLabel className="text-muted-foreground text-xs">Columns</ContextMenuLabel>
        {COLUMNS.filter((c) => !c.required).map((c) => (
          <ContextMenuCheckboxItem
            key={c.id}
            checked={!layout.hidden.includes(c.id)}
            onCheckedChange={(on) => setHidden(c.id, on !== true)}
            onSelect={(e) => e.preventDefault()}
            data-testid={`column-toggle-${c.id}`}
          >
            {c.label}
          </ContextMenuCheckboxItem>
        ))}
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={resetColumns}>Reset columns</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** Drag to resize; double-click restores the default width. */
function ResizeHandle({ column }: { column: PlacedColumn }) {
  const setWidth = usePrefs((s) => s.setColumnWidth);
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startWidth = column.px;
    const onMove = (ev: globalThis.PointerEvent) =>
      setWidth(column.id, Math.max(column.minWidth, startWidth + ev.clientX - startX));
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${column.label}`}
      onPointerDown={onPointerDown}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => {
        e.stopPropagation();
        setWidth(column.id, column.width);
      }}
      draggable={false}
      className="hover:bg-primary/40 absolute top-0 -right-1 z-10 h-full w-2 cursor-col-resize"
      data-testid={`resize-${column.id}`}
    />
  );
}

interface FlowRowViewProps {
  index: number;
  start: number;
  row: Row | undefined;
  columns: PlacedColumn[];
  template: string;
  selected: boolean;
  onClick: (index: number, row: Row | undefined) => void;
  onContextMenu: (index: number, row: Row | undefined) => void;
}

const cell = "flex min-w-0 items-center px-1.5";

const FlowRowView = memo(function FlowRowView({
  index,
  start,
  row,
  columns,
  template,
  selected,
  onClick,
  onContextMenu,
}: FlowRowViewProps) {
  return (
    <div
      role="row"
      aria-rowindex={index + 1}
      aria-selected={selected}
      data-selected={selected}
      data-flow-id={row?.id}
      onMouseDown={(e) => e.button === 0 && onClick(index, row)}
      onContextMenu={() => onContextMenu(index, row)}
      className={cn(
        "group/row absolute top-0 left-0 grid w-full font-mono text-[11.5px] leading-none",
        index % 2 === 1 && "bg-[var(--pane-row-alt)]",
        "hover:bg-[var(--pane-row-hover)]",
        "data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
      )}
      style={{ gridTemplateColumns: template, height: ROW_HEIGHT, transform: `translateY(${start}px)` }}
    >
      {row ? (
        columns.map((col) => <RowCell key={col.id} id={col.id} row={row} />)
      ) : (
        <div data-placeholder className="text-muted-foreground/40 col-span-full px-2 leading-[22px]">
          …
        </div>
      )}
    </div>
  );
});

const dim = "text-muted-foreground group-data-[selected=true]/row:text-selection-foreground";

/** The row's "…": opens the same menu a right-click does, under the button. */
function openRowMenu(e: React.MouseEvent<HTMLButtonElement>) {
  const box = e.currentTarget.getBoundingClientRect();
  e.currentTarget.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: box.left, clientY: box.bottom }),
  );
}

function RowCell({ id, row }: { id: ColumnId; row: Row }) {
  switch (id) {
    case "id":
      return (
        <div className={cn(cell, "justify-end gap-1 tabular-nums", dim)}>
          {row.pinned && (
            <Pin
              aria-label="Pinned"
              className="size-2.5 shrink-0 text-[var(--primary)] group-data-[selected=true]/row:text-selection-foreground"
            />
          )}
          {row.id + 1}
        </div>
      );
    case "state":
      return (
        <div className={cn(cell, "justify-center")}>
          <StateDot state={row.state} />
        </div>
      );
    case "url":
      return (
        <div className={cell}>
          <UrlText row={row} operation={row.graphqlOp} sentBy={row.sentBy} />
        </div>
      );
    case "client":
      return (
        <div className={cn(cell, "gap-1.5 truncate font-sans text-xs")} title={row.client ?? undefined}>
          {row.client && <AppIcon name={row.client} />}
          <span className="truncate">{row.client ?? "—"}</span>
        </div>
      );
    case "method":
      return (
        <div className={cn(cell, "justify-center")}>
          <MethodPill method={row.method} />
        </div>
      );
    case "status":
      return (
        <div className={cn(cell, "justify-center")}>
          <StatusPill status={row.status} state={row.state} />
        </div>
      );
    case "time":
      return <div className={cn(cell, "justify-end tabular-nums", dim)}>{formatTime(row.startedAtMs)}</div>;
    case "duration":
      return <div className={cn(cell, "justify-end tabular-nums")}>{formatDuration(row.durationMs)}</div>;
    case "req":
      return <div className={cn(cell, "justify-end tabular-nums", dim)}>{formatSize(row.reqSize)}</div>;
    case "resp":
      return <div className={cn(cell, "justify-end tabular-nums")}>{formatSize(row.respSize)}</div>;
    case "proto":
      return (
        <div className={cn(cell, "justify-center", dim)} title={row.httpVersion}>
          {versionLabel(row.httpVersion)}
        </div>
      );
    case "ssl":
      return (
        <div className={cn(cell, "justify-center", dim)} title={sslTitle(row.tls)}>
          <SslIcon tls={row.tls} />
        </div>
      );
    case "edited":
      return (
        <div className={cn(cell, "justify-center")}>
          {row.edited && (
            <PencilLine
              aria-label="Changed by a rule"
              className="size-3 text-[var(--rule)] group-data-[selected=true]/row:text-selection-foreground"
            />
          )}
        </div>
      );
    case "tools":
      return (
        <div className={cn(cell, "justify-center")}>
          <button
            type="button"
            aria-label="Flow actions"
            className="rounded-sm opacity-0 group-hover/row:opacity-60 hover:opacity-100 focus-visible:opacity-100"
            onClick={openRowMenu}
            data-testid="row-tools"
          >
            <MoreHorizontal className="size-3" />
          </button>
        </div>
      );
  }
}
