import type { FlowSelection_Deserialize } from "@/bindings";

/**
 * The flow table's multi-selection (M17), by flow id, never by position:
 * rows move as traffic arrives and filters change. ⌘A selects the whole
 * view without listing it: `all` with `ids` as the exceptions.
 */
export interface TableSelection {
  all: boolean;
  /** Selected ids, or with `all`, the ids left out. */
  ids: ReadonlySet<number>;
}

export const EMPTY_SELECTION: TableSelection = { all: false, ids: new Set() };

export function single(id: number | null): TableSelection {
  return { all: false, ids: new Set(id === null ? [] : [id]) };
}

export function isSelected(sel: TableSelection, id: number): boolean {
  return sel.all ? !sel.ids.has(id) : sel.ids.has(id);
}

/** How many flows are selected, in a view of `len` rows. */
export function selectionCount(sel: TableSelection, len: number): number {
  return sel.all ? Math.max(0, len - sel.ids.size) : sel.ids.size;
}

/** ⌘-click: in or out of the selection. */
export function toggle(sel: TableSelection, id: number): TableSelection {
  const ids = new Set(sel.ids);
  if (ids.has(id)) ids.delete(id);
  else ids.add(id);
  return { all: sel.all, ids };
}

/** ⇧-click: exactly the rows between the anchor and the click. */
export function range(ids: readonly number[]): TableSelection {
  return { all: false, ids: new Set(ids) };
}

/** The selection as the engine takes it; ids in arrival order. */
export function toFlowSelection(sel: TableSelection, view: number | null): FlowSelection_Deserialize {
  if (sel.all && view !== null) return { kind: "view", view, except: [...sel.ids].sort((a, b) => a - b) };
  if (sel.all) return { kind: "all" };
  return { kind: "ids", ids: [...sel.ids].sort((a, b) => a - b) };
}

/** True for more than one flow (the bulk menu's case). */
export function isMulti(sel: TableSelection, len: number): boolean {
  return selectionCount(sel, len) > 1;
}

/** The selected ids when the selection lists them (not a whole view). */
export function listedIds(sel: TableSelection): number[] | null {
  return sel.all ? null : [...sel.ids].sort((a, b) => a - b);
}
