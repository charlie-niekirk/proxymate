import { MessageSquareText, MoreHorizontal, PencilLine, Pin } from "lucide-react";
import { memo, type MouseEvent as ReactMouseEvent } from "react";

import { AppIcon } from "@/components/AppIcon";
import { formatDuration, formatSize, formatTime, versionLabel, type Row } from "@/lib/flow";
import { sslTitle } from "@/lib/tls";
import { cn } from "@/lib/utils";

import { MethodPill, SslIcon, StateDot, StatusPill, UrlText } from "./cells";
import { ROW_HEIGHT, rowDomId, type PlacedColumn } from "./columns";

interface FlowRowViewProps {
  index: number;
  start: number;
  row: Row | undefined;
  columns: PlacedColumn[];
  template: string;
  selected: boolean;
  /** The focused row of a multi-selection: the one the inspector shows. */
  focused: boolean;
  onClick: (index: number, row: Row | undefined, mods: { shift: boolean; toggle: boolean }) => void;
  onContextMenu: (index: number, row: Row | undefined) => void;
}

const cell = "flex min-w-0 items-center px-1.5";

export const FlowRowView = memo(function FlowRowView({
  index,
  start,
  row,
  columns,
  template,
  selected,
  focused,
  onClick,
  onContextMenu,
}: FlowRowViewProps) {
  return (
    // The grid keeps the focus and names this row with aria-activedescendant, so rows aren't tab stops.
    // eslint-disable-next-line jsx-a11y-x/interactive-supports-focus
    <div
      role="row"
      id={row ? rowDomId(row.id) : undefined}
      // Row 1 is the header.
      aria-rowindex={index + 2}
      aria-selected={selected}
      data-selected={selected}
      data-focused={focused || undefined}
      data-flow-id={row?.id}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        // ⇧-click would also select text in the rows.
        if (e.shiftKey) e.preventDefault();
        onClick(index, row, { shift: e.shiftKey, toggle: e.metaKey || e.ctrlKey });
      }}
      onContextMenu={() => onContextMenu(index, row)}
      className={cn(
        "group/row absolute top-0 left-0 grid w-full font-mono text-[11.5px] leading-none",
        index % 2 === 1 && "bg-[var(--pane-row-alt)]",
        "hover:bg-[var(--pane-row-hover)]",
        "data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
        "data-[focused]:shadow-[inset_0_0_0_1px_var(--selection-fg)]",
      )}
      style={{ gridTemplateColumns: template, height: ROW_HEIGHT, transform: `translateY(${start}px)` }}
    >
      {row ? (
        columns.map((col) => <RowCell key={col.id} column={col} row={row} />)
      ) : (
        <div role="gridcell" data-placeholder className="text-muted-foreground/40 col-span-full px-2 leading-[22px]">
          …
        </div>
      )}
    </div>
  );
});

const dim = "text-muted-foreground group-data-[selected=true]/row:text-selection-foreground";

/** The row's "…": opens the same menu a right-click does, under the button. */
function openRowMenu(e: ReactMouseEvent<HTMLButtonElement>) {
  const box = e.currentTarget.getBoundingClientRect();
  e.currentTarget.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: box.left, clientY: box.bottom }),
  );
}

function RowCell({ column, row }: { column: PlacedColumn; row: Row }) {
  if (column.header) {
    const value = row.extra[column.header.index] ?? null;
    return (
      <div role="gridcell" className={cn(cell, "truncate", value === null && dim)} title={value ?? undefined}>
        <span className="truncate">{value ?? "—"}</span>
      </div>
    );
  }
  switch (column.id) {
    case "id":
      return (
        <div role="gridcell" className={cn(cell, "justify-end gap-1 tabular-nums", dim)}>
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
        <div role="gridcell" className={cn(cell, "justify-center")}>
          <StateDot state={row.state} />
        </div>
      );
    case "url":
      return (
        <div role="gridcell" className={cell}>
          <UrlText row={row} operation={row.graphqlOp} sentBy={row.sentBy} />
        </div>
      );
    case "client":
      return (
        <div role="gridcell" className={cn(cell, "gap-1.5 truncate font-sans text-xs")} title={row.client ?? undefined}>
          {row.client && <AppIcon name={row.client} />}
          <span className="truncate">{row.client ?? "—"}</span>
        </div>
      );
    case "method":
      return (
        <div role="gridcell" className={cn(cell, "justify-center")}>
          <MethodPill method={row.method} />
        </div>
      );
    case "status":
      return (
        <div role="gridcell" className={cn(cell, "justify-center")}>
          <StatusPill status={row.status} state={row.state} grpc={row.grpcStatus} />
        </div>
      );
    case "time":
      return <div role="gridcell" className={cn(cell, "justify-end tabular-nums", dim)}>{formatTime(row.startedAtMs)}</div>;
    case "duration":
      return <div role="gridcell" className={cn(cell, "justify-end tabular-nums")}>{formatDuration(row.durationMs)}</div>;
    case "req":
      return <div role="gridcell" className={cn(cell, "justify-end tabular-nums", dim)}>{formatSize(row.reqSize)}</div>;
    case "resp":
      return <div role="gridcell" className={cn(cell, "justify-end tabular-nums")}>{formatSize(row.respSize)}</div>;
    case "proto":
      return (
        <div role="gridcell" className={cn(cell, "justify-center", dim)} title={row.httpVersion}>
          {versionLabel(row.httpVersion)}
        </div>
      );
    case "ssl":
      return (
        <div role="gridcell" className={cn(cell, "justify-center", dim)} title={sslTitle(row.tls)}>
          <SslIcon tls={row.tls} />
        </div>
      );
    case "notes":
      return (
        <div role="gridcell" className={cn(cell, "justify-center gap-1")} title={row.comment ?? undefined}>
          {row.label && (
            <span
              aria-label={`Label: ${row.label}`}
              className="size-2 shrink-0 rounded-full group-data-[selected=true]/row:ring-1 group-data-[selected=true]/row:ring-[var(--selection-fg)]"
              style={{ background: `var(--label-${row.label})` }}
              data-label={row.label}
            />
          )}
          {row.comment && (
            <MessageSquareText
              aria-label="Commented"
              className="text-muted-foreground size-3 shrink-0 group-data-[selected=true]/row:text-selection-foreground"
            />
          )}
          {row.edited && (
            <PencilLine
              aria-label="Changed by a rule"
              className="size-3 shrink-0 text-[var(--rule)] group-data-[selected=true]/row:text-selection-foreground"
            />
          )}
        </div>
      );
    case "tools":
    default:
      return (
        <div role="gridcell" className={cn(cell, "justify-center")}>
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
