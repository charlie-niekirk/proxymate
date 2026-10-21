import { create } from "zustand";

import type { ContentGroup, FilterSpec_Deserialize, ProtocolFilter, SortSpec } from "@/bindings";

export interface ViewStats {
  len: number;
  total: number;
}

export interface FilterTiming {
  /** Engine refilter time reported by Rust. */
  engineMs: number;
  /** Filter change to visible rows painted. */
  endToEndMs: number;
}

interface UiState {
  filter: FilterSpec_Deserialize;
  sort: SortSpec;
  selectedId: number | null;
  stats: ViewStats;
  filterTiming: FilterTiming | null;
  /** Keep the newest row in view while traffic arrives. */
  autoScroll: boolean;

  setText(text: string): void;
  setProtocol(protocol: ProtocolFilter): void;
  toggleContent(group: ContentGroup): void;
  clearContent(): void;
  setScope(scope: { host?: string | null; client?: string | null }): void;
  setSort(sort: SortSpec): void;
  select(id: number | null): void;
  setStats(stats: ViewStats): void;
  setFilterTiming(timing: FilterTiming): void;
  setAutoScroll(on: boolean): void;
}

export const useUi = create<UiState>()((set) => ({
  filter: { text: "", protocol: "all", content: [], host: null, client: null },
  sort: { key: "id", dir: "asc" },
  selectedId: null,
  stats: { len: 0, total: 0 },
  filterTiming: null,
  autoScroll: false,

  setText: (text) => set((s) => ({ filter: { ...s.filter, text } })),
  setProtocol: (protocol) => set((s) => ({ filter: { ...s.filter, protocol } })),
  toggleContent: (group) =>
    set((s) => {
      const content = s.filter.content.includes(group)
        ? s.filter.content.filter((g) => g !== group)
        : [...s.filter.content, group];
      return { filter: { ...s.filter, content } };
    }),
  clearContent: () => set((s) => ({ filter: { ...s.filter, content: [] } })),
  setScope: ({ host = null, client = null }) => set((s) => ({ filter: { ...s.filter, host, client } })),
  setSort: (sort) => set({ sort }),
  select: (selectedId) => set({ selectedId }),
  setStats: (stats) => set({ stats }),
  setFilterTiming: (filterTiming) => set({ filterTiming }),
  setAutoScroll: (autoScroll) => set({ autoScroll }),
}));
