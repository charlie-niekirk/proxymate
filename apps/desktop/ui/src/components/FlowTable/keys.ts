//! The flow table's keys, as a pure function of the key and where the table
//! stands, so they can be tested without a DOM (`keys.test.ts`).

/** Where the table stands when a key arrives. */
export interface TableState {
  /** The focused flow's row index, if it is known. */
  from: number | null;
  /** The first and last rendered rows. */
  first: number;
  last: number;
  /** Rows in the view. */
  len: number;
  /** Rows per page (PgUp/PgDn). */
  page: number;
  /** More than one flow is selected. */
  multi: boolean;
  /** A flow is focused. */
  selected: boolean;
}

export interface TableKey {
  key: string;
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}

export type TableAction =
  /** Select row `to` (clamped to the view); with ⇧, extend the selection to it. */
  | { kind: "move"; to: number; extend: boolean }
  /** Enter: into the inspector. */
  | { kind: "inspect" }
  /** Escape in a multi-selection: back to the focused flow alone. */
  | { kind: "collapse" }
  /** Handled, nothing to do (Enter with nothing selected). */
  | { kind: "none" };

/**
 * What `e` does to the table, or `null` when the table leaves it alone (any
 * key with ⌘, Ctrl or ⌥ belongs to the app's commands).
 */
export function tableKey(e: TableKey, s: TableState): TableAction | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key === "Escape") return s.multi ? { kind: "collapse" } : null;
  if (e.key === "Enter") return s.selected ? { kind: "inspect" } : { kind: "none" };
  const target = moveTarget(e.key, s);
  if (target === null) return null;
  if (s.len === 0) return { kind: "none" };
  return { kind: "move", to: Math.max(0, Math.min(s.len - 1, target)), extend: e.shiftKey };
}

function moveTarget(key: string, { from, first, last, len, page }: TableState): number | null {
  switch (key) {
    case "ArrowDown":
      return from === null ? first : from + 1;
    case "ArrowUp":
      return from === null ? last : from - 1;
    case "PageDown":
      return (from ?? first) + page;
    case "PageUp":
      return (from ?? first) - page;
    case "Home":
      return 0;
    case "End":
      return len - 1;
    default:
      return null;
  }
}

/** A column resize handle's keys: ←/→ by 8 px (⇧: 32), Home to the minimum, ↩ to the default. */
export function resizeKey(
  e: Pick<TableKey, "key" | "shiftKey">,
  col: { px: number; minWidth: number; width: number },
  max: number,
): number | null {
  const step = e.shiftKey ? 32 : 8;
  const clamp = (w: number) => Math.max(col.minWidth, Math.min(max, w));
  switch (e.key) {
    case "ArrowLeft":
      return clamp(col.px - step);
    case "ArrowRight":
      return clamp(col.px + step);
    case "Home":
      return col.minWidth;
    case "End":
      return max;
    case "Enter":
      return clamp(col.width);
    default:
      return null;
  }
}
