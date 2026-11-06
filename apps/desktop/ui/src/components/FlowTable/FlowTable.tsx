import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";

import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";
import { useFlowView } from "@/hooks/useFlowView";
import { FLOW_TABLE_ID, focusInspector } from "@/lib/focus";
import { backend } from "@/lib/backend";
import { type Row } from "@/lib/flow";
import { isSelected, range, selectionCount } from "@/lib/selection";
import { MAX_ROWS_PER_CALL } from "@/lib/rowCache";
import { usePrefs } from "@/store/prefs";
import { useUi } from "@/store/ui";

import { ROW_HEIGHT, gridTemplate, minTableWidth, placeColumns, rowDomId, type PlacedColumn } from "./columns";
import { EmptyState, ViewError } from "./EmptyState";
import { FlowContextMenu } from "./FlowContextMenu";
import { FlowRowView } from "./FlowRow";
import { HEADER_HEIGHT, Header } from "./Header";
import { tableKey } from "./keys";


/**
 * The flow table: a hand-built div grid virtualised with TanStack Virtual.
 * Rows come from the non-reactive RowCache; only the visible window is ever
 * fetched from Rust. Columns are resized, reordered, hidden and sorted from
 * the header; the layout persists (store/prefs.ts).
 *
 * Keys: ↑/↓, PgUp/PgDn, Home/End move the selection (with ⇧, extend it);
 * Enter focuses the inspector; Escape drops back to the focused flow.
 * ⌘-click toggles a flow, ⇧-click selects a range (M17): the selection is
 * flow ids (`lib/selection.ts`), and the inspector follows the focused one.
 */
export function FlowTable() {
  const { cache, version, error, retry } = useFlowView();
  const len = useUi((s) => s.stats.len);
  const selectedId = useUi((s) => s.selectedId);
  const selection = useUi((s) => s.selection);
  const select = useUi((s) => s.select);
  const autoScroll = useUi((s) => s.autoScroll);
  const layout = usePrefs((s) => s.columns);
  const headerColumns = usePrefs((s) => s.headerColumns);
  const columns = useMemo(() => placeColumns(layout, headerColumns), [layout, headerColumns]);
  const multi = selectionCount(selection, len) > 1;
  // The engine fills header column values for every store.
  useEffect(() => {
    backend.setHeaderColumns(headerColumns).catch((e: unknown) => console.error("header columns", e));
  }, [headerColumns]);
  const template = useMemo(() => gridTemplate(columns), [columns]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectedIndex = useRef<number | null>(null);
  // Where a ⇧-click or ⇧-arrow range starts.
  const anchorIndex = useRef<number | null>(null);
  // A keyboard move to a row that isn't loaded yet: selected once it is.
  const pendingIndex = useRef<number | null>(null);
  // The row under a right-click: set by the row, read by the menu trigger.
  const pressedRow = useRef<Row | null>(null);
  const [menu, setMenu] = useState<{ row: Row; column: PlacedColumn | null } | null>(null);

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

  /** The ids of rows `a..=b` (either order): from the cache, else fetched. */
  const idsBetween = useCallback(
    async (a: number, b: number): Promise<number[]> => {
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      const ids: number[] = [];
      for (let i = lo; i <= hi; i++) {
        const row = cache?.get(i);
        if (!row) break;
        ids.push(row.id);
      }
      if (ids.length === hi - lo + 1) return ids;
      const view = useUi.getState().view;
      if (view === null) return ids;
      // `get_rows` answers at most MAX_ROWS_PER_CALL rows: ask in steps.
      const all: number[] = [];
      for (let start = lo; start <= hi; start += MAX_ROWS_PER_CALL) {
        const count = Math.min(MAX_ROWS_PER_CALL, hi - start + 1);
        const page = await backend.getRows(view, start, count);
        all.push(...page.rows.map((r) => r[0]));
        if (page.rows.length < count) break;
      }
      return all;
    },
    [cache],
  );

  const selectRange = useCallback(
    (to: number, focus: number) => {
      const from = anchorIndex.current ?? to;
      const total = useUi.getState().stats.len;
      // The whole view: as a view selection, not 100k ids.
      if (Math.min(from, to) === 0 && Math.max(from, to) >= total - 1 && total > 1) {
        useUi.getState().setSelection({ all: true, ids: new Set() }, focus);
        return;
      }
      void idsBetween(from, to).then((ids) => useUi.getState().setSelection(range(ids), focus));
    },
    [idsBetween],
  );

  // Stable, so memoised rows that stay on screen skip re-rendering while scrolling.
  const onRowClick = useCallback(
    (index: number, row: Row | undefined, mods: { shift: boolean; toggle: boolean }) => {
      pendingIndex.current = null;
      if (!row) return;
      selectedIndex.current = index;
      if (mods.shift) {
        selectRange(index, row.id);
        return;
      }
      anchorIndex.current = index;
      if (mods.toggle) useUi.getState().toggleSelected(row.id);
      else select(row.id);
    },
    [select, selectRange],
  );

  const onRowContextMenu = useCallback(
    (index: number, row: Row | undefined) => {
      pressedRow.current = row ?? null;
      if (!row) return;
      // A right-click inside a multi-selection acts on all of it.
      const ui = useUi.getState();
      if (selectionCount(ui.selection, ui.stats.len) > 1 && isSelected(ui.selection, row.id)) return;
      selectedIndex.current = index;
      anchorIndex.current = index;
      select(row.id);
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
    setMenu({ row, column: columns[at] ?? null });
  };

  const moveTo = (index: number, extend = false) => {
    if (len === 0) return;
    const to = Math.max(0, Math.min(len - 1, index));
    selectedIndex.current = to;
    const row = cache?.get(to);
    pendingIndex.current = row || extend ? null : to;
    if (extend && row) {
      anchorIndex.current ??= to;
      selectRange(to, row.id);
    } else if (row) {
      anchorIndex.current = to;
      select(row.id);
    }
    virtualizer.scrollToIndex(to, { align: "auto" });
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // Keys on the header's buttons and resize handles are theirs.
    if (e.target !== e.currentTarget) return;
    // Where the selection is now: the remembered index if it still holds
    // that flow, else wherever the flow is among the loaded rows.
    let from = selectedIndex.current;
    if (selectedId !== null && (from === null || cache?.get(from)?.id !== selectedId)) {
      from = cache?.indexOf(selectedId) ?? null;
    }
    const page = Math.max(1, Math.floor(((scrollRef.current?.clientHeight ?? 0) - HEADER_HEIGHT) / ROW_HEIGHT) - 1);
    const action = tableKey(e, { from, first, last, len, page, multi, selected: selectedId !== null });
    if (!action) return;
    e.preventDefault();
    switch (action.kind) {
      case "move":
        if (action.extend && anchorIndex.current === null) anchorIndex.current = from;
        moveTo(action.to, action.extend);
        break;
      case "inspect":
        focusInspector();
        break;
      case "collapse":
        select(selectedId);
        break;
      case "none":
        break;
    }
  };

  // The focused flow's row, while it is rendered: what a screen reader reads as the grid's current row.
  const active = selectedId !== null && items.some((i) => cache?.get(i.index)?.id === selectedId) ? rowDomId(selectedId) : undefined;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={onTableContextMenu}>
        <div
          ref={scrollRef}
          id={FLOW_TABLE_ID}
          tabIndex={0}
          role="grid"
          aria-label="Flows"
          aria-rowcount={len + 1}
          aria-colcount={columns.length}
          aria-multiselectable
          aria-activedescendant={active}
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
                  selected={row !== undefined && isSelected(selection, row.id)}
                  focused={multi && row !== undefined && row.id === selectedId}
                  onClick={onRowClick}
                  onContextMenu={onRowContextMenu}
                />
              );
            })}
          </div>
          {error !== null ? <ViewError error={error} retry={retry} /> : len === 0 && <EmptyState />}
        </div>
      </ContextMenuTrigger>
      <FlowContextMenu row={menu?.row ?? null} column={menu?.column ?? null} />
    </ContextMenu>
  );
}
