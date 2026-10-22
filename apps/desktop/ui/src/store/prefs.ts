import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import type { SortSpec } from "@/bindings";
import { COLUMNS, type ColumnId } from "@/components/FlowTable/columns";

export type Appearance = "system" | "light" | "dark";

/** The flow table's columns as the user arranged them. */
export interface ColumnLayout {
  /** Every column, in display order. */
  order: ColumnId[];
  /** Widths in px the user dragged to; missing ones use the default. */
  widths: Partial<Record<ColumnId, number>>;
  hidden: ColumnId[];
}

export const DEFAULT_LAYOUT: ColumnLayout = {
  order: COLUMNS.map((c) => c.id),
  widths: {},
  hidden: [],
};

export const DEFAULT_SORT: SortSpec = { key: "id", dir: "asc" };

interface PrefsState {
  appearance: Appearance;
  columns: ColumnLayout;
  sort: SortSpec;
  setAppearance(appearance: Appearance): void;
  setColumnWidth(id: ColumnId, width: number): void;
  /** Moves `id` to just before `before` (or to the end). */
  moveColumn(id: ColumnId, before: ColumnId | null): void;
  setColumnHidden(id: ColumnId, hidden: boolean): void;
  resetColumns(): void;
  setSort(sort: SortSpec): void;
}

/** Storage that never throws: a private window or blocked storage just forgets. */
const safeStorage = createJSONStorage(() => ({
  getItem: (key) => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  setItem: (key, value) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Not saved; the preference still applies for this run.
    }
  },
  removeItem: (key) => {
    try {
      localStorage.removeItem(key);
    } catch {
      // Nothing to remove.
    }
  },
}));

/**
 * Per-user UI preferences that outlive a launch: the theme, the flow
 * table's columns (width, order, visibility) and its sort. Kept in the
 * webview's local storage; the engine has no say in them.
 */
export const usePrefs = create<PrefsState>()(
  persist(
    (set) => ({
      appearance: "system",
      columns: DEFAULT_LAYOUT,
      sort: DEFAULT_SORT,
      setAppearance: (appearance) => set({ appearance }),
      setColumnWidth: (id, width) =>
        set((s) => ({ columns: { ...s.columns, widths: { ...s.columns.widths, [id]: Math.round(width) } } })),
      moveColumn: (id, before) =>
        set((s) => {
          if (id === before) return {};
          const order = s.columns.order.filter((c) => c !== id);
          const at = before === null ? order.length : order.indexOf(before);
          order.splice(at < 0 ? order.length : at, 0, id);
          return { columns: { ...s.columns, order } };
        }),
      setColumnHidden: (id, hidden) =>
        set((s) => {
          const rest = s.columns.hidden.filter((c) => c !== id);
          return { columns: { ...s.columns, hidden: hidden ? [...rest, id] : rest } };
        }),
      resetColumns: () => set({ columns: DEFAULT_LAYOUT }),
      setSort: (sort) => set({ sort }),
    }),
    {
      name: "proxymate-prefs",
      version: 1,
      storage: safeStorage,
      // Columns added in a later version join at their default place;
      // ones that no longer exist are dropped.
      merge: (persisted, current) => {
        const saved = (persisted ?? {}) as Partial<PrefsState>;
        const known = new Set<ColumnId>(COLUMNS.map((c) => c.id));
        const order = (saved.columns?.order ?? []).filter((c) => known.has(c));
        for (const [i, c] of DEFAULT_LAYOUT.order.entries()) {
          if (!order.includes(c)) order.splice(Math.min(i, order.length), 0, c);
        }
        return {
          ...current,
          ...saved,
          columns: {
            order,
            widths: saved.columns?.widths ?? {},
            hidden: (saved.columns?.hidden ?? []).filter((c) => known.has(c)),
          },
        };
      },
    },
  ),
);
