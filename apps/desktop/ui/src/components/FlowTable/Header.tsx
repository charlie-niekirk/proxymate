import { ArrowDown, ArrowUp } from "lucide-react";
import { memo, useState, type DragEvent, type KeyboardEvent, type PointerEvent } from "react";

import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { cn } from "@/lib/utils";
import { MAX_HEADER_COLUMNS, usePrefs } from "@/store/prefs";
import { useUi } from "@/store/ui";

import { COLUMNS, headerColumnId, isHeaderColumnId, type ColumnId, type PlacedColumn } from "./columns";
import { resizeKey } from "./keys";

export const HEADER_HEIGHT = 22;

/** The widest a column can be dragged or keyed to. */
export const MAX_COLUMN_WIDTH = 1600;

/**
 * Column headers: a sortable column's title is a button (click or ↩ sorts),
 * its right edge resizes (drag, or ←/→ when focused), dragging a header
 * reorders, right-click picks columns.
 * Memoised: the table re-renders on every delta, the header only when its columns or the sort change.
 */
export const Header = memo(function Header({ columns, template }: { columns: PlacedColumn[]; template: string }) {
  const sort = useUi((s) => s.sort);
  const setSort = useUi((s) => s.setSort);
  const layout = usePrefs((s) => s.columns);
  const headerColumns = usePrefs((s) => s.headerColumns);
  const setHidden = usePrefs((s) => s.setColumnHidden);
  const resetColumns = usePrefs((s) => s.resetColumns);
  const moveColumn = usePrefs((s) => s.moveColumn);
  const removeHeaderColumn = usePrefs((s) => s.removeHeaderColumn);
  const [dropTarget, setDropTarget] = useState<ColumnId | null>(null);
  // The header right-clicked, for "Remove column".
  const [pressed, setPressed] = useState<PlacedColumn | null>(null);
  const sortedBy = (col: PlacedColumn) =>
    col.sortKey !== undefined && sort.key === col.sortKey && (col.sortKey !== "extra" || (sort.extra ?? 0) === col.header?.index);
  const onSort = (col: PlacedColumn) => {
    if (!col.sortKey) return;
    const dir = sortedBy(col) && sort.dir === "asc" ? "desc" : "asc";
    setSort({ key: col.sortKey, dir, extra: col.header?.index ?? 0 });
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
          aria-rowindex={1}
          className="bg-pane-header text-muted-foreground sticky top-0 z-10 grid border-b text-xs font-medium"
          style={{ gridTemplateColumns: template, height: HEADER_HEIGHT }}
          data-testid="flow-table-header"
        >
          {columns.map((col) => {
            const sorted = sortedBy(col) && (col.sortKey !== "id" || sort.dir === "desc");
            const align = cn(col.align === "right" && "justify-end", col.align === "center" && "justify-center");
            const title = col.title ? (
              <span className="truncate">{col.title}</span>
            ) : (
              <span className="sr-only">{col.label}</span>
            );
            return (
              // Its sort button and resize handle are the focusable parts; reordering is by pointer
              // (or Reset columns), and the column menu shows and hides columns.
              // eslint-disable-next-line jsx-a11y-x/interactive-supports-focus
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
                onContextMenu={() => setPressed(col)}
                title={col.header ? col.label : undefined}
                className={cn(
                  "relative flex min-w-0 items-center border-r border-border/60 px-1.5 last:border-r-0",
                  align,
                  dropTarget === col.id && "shadow-[inset_2px_0_0_var(--primary)]",
                )}
                data-column={col.id}
              >
                {col.sortKey ? (
                  <button
                    type="button"
                    onClick={() => onSort(col)}
                    aria-label={col.title ? undefined : `Sort by ${col.label}`}
                    className={cn(
                      "hover:text-foreground focus-visible:text-foreground flex min-w-0 flex-1 items-center gap-0.5 self-stretch outline-none focus-visible:underline",
                      align,
                    )}
                  >
                    {title}
                    {sorted &&
                      (sort.dir === "asc" ? <ArrowUp className="size-3 shrink-0" /> : <ArrowDown className="size-3 shrink-0" />)}
                  </button>
                ) : (
                  title
                )}
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
        {headerColumns.map((h, i) => {
          const id = headerColumnId(h);
          return (
            <ContextMenuCheckboxItem
              key={id}
              checked={!layout.hidden.includes(id)}
              onCheckedChange={(on) => setHidden(id, on !== true)}
              onSelect={(e) => e.preventDefault()}
              data-testid={`column-toggle-header-${i}`}
            >
              <span className="truncate font-mono">{h.name}</span>
              <span className="text-muted-foreground ml-auto pl-2 text-[11px]">{h.part === "request" ? "req" : "resp"}</span>
            </ContextMenuCheckboxItem>
          );
        })}
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={headerColumns.length >= MAX_HEADER_COLUMNS}
          onSelect={() => useUi.getState().setAddColumnOpen(true)}
          data-testid="add-header-column"
        >
          Add header column…
        </ContextMenuItem>
        {pressed && isHeaderColumnId(pressed.id) && (
          <ContextMenuItem onSelect={() => removeHeaderColumn(pressed.id)} data-testid="remove-header-column">
            Remove “{pressed.title}” column
          </ContextMenuItem>
        )}
        <ContextMenuItem onSelect={resetColumns}>Reset columns</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});

/**
 * A column's right edge: drag to resize, double-click restores the default
 * width. Focused, ←/→ resize by 8 px (⇧: 32), Home/End go to the minimum
 * and maximum, ↩ restores the default (a WAI-ARIA window-splitter separator).
 */
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
      setWidth(column.id, Math.max(column.minWidth, Math.min(MAX_COLUMN_WIDTH, startWidth + ev.clientX - startX)));
    const onUp = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const width = resizeKey(e, column, MAX_COLUMN_WIDTH);
    if (width === null) return;
    e.preventDefault();
    e.stopPropagation();
    setWidth(column.id, width);
  };
  return (
    // A focusable separator is a widget (WAI-ARIA 1.2), which jsx-a11y's role table predates.
    // eslint-disable-next-line jsx-a11y-x/no-noninteractive-element-interactions
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize ${column.label}`}
      aria-valuenow={column.px}
      aria-valuemin={column.minWidth}
      aria-valuemax={MAX_COLUMN_WIDTH}
      // eslint-disable-next-line jsx-a11y-x/no-noninteractive-tabindex -- see above
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        setWidth(column.id, column.width);
      }}
      draggable={false}
      className="hover:bg-primary/40 focus-visible:bg-primary/60 absolute top-0 -right-1 z-10 h-full w-2 cursor-col-resize outline-none"
      data-testid={`resize-${column.id}`}
    />
  );
}
