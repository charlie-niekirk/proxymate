import { create } from "zustand";

import type { ContentGroup, ContentKind, FilterSpec_Deserialize, ProtocolFilter, SortSpec } from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import type { Rule } from "@/lib/rules";

export interface ViewStats {
  len: number;
  total: number;
}

/** Inspector tabs; the choice sticks as the selection changes. */
export type RequestTab = "Header" | "Query" | "Body" | "Raw";
export type ResponseTab = "Header" | "Body" | "Raw" | "TLS" | "Timing";

/** Device Setup Wizard tabs. */
export type DeviceTab = "simulator" | "iphone" | "android" | "emulator";

/** Settings pages; more arrive with M10. */
export type SettingsPage = "mcp";

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
  requestTab: RequestTab;
  responseTab: ResponseTab;
  /** Body viewer the user picked per content kind; sticks across flows. */
  viewerByKind: Partial<Record<ContentKind, ViewerId>>;
  certWizardOpen: boolean;
  /**
   * The Rules Editor: open or not, and what it shows. `draft` is an unsaved
   * rule (from a preset or "New"); otherwise `ruleId` names a saved one.
   */
  rulesEditor: { open: boolean; ruleId: string | null; draft: Rule | null; seq: number };
  breakpointsOpen: boolean;
  /** The breakpoint shown in the Breakpoint Editor. */
  selectedBreakpoint: number | null;
  deviceSetupOpen: boolean;
  deviceTab: DeviceTab;
  settings: { open: boolean; page: SettingsPage };

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
  setRequestTab(tab: RequestTab): void;
  setResponseTab(tab: ResponseTab): void;
  setViewer(kind: ContentKind, viewer: ViewerId): void;
  setCertWizardOpen(open: boolean): void;
  openRulesEditor(target?: { ruleId?: string | null; draft?: Rule | null }): void;
  closeRulesEditor(): void;
  setBreakpointsOpen(open: boolean): void;
  selectBreakpoint(id: number | null): void;
  setDeviceSetupOpen(open: boolean): void;
  setDeviceTab(tab: DeviceTab): void;
  openSettings(page?: SettingsPage): void;
  closeSettings(): void;
}

export const useUi = create<UiState>()((set) => ({
  filter: { text: "", protocol: "all", content: [], host: null, client: null },
  sort: { key: "id", dir: "asc" },
  selectedId: null,
  stats: { len: 0, total: 0 },
  filterTiming: null,
  autoScroll: false,
  requestTab: "Header",
  responseTab: "Body",
  viewerByKind: {},
  certWizardOpen: false,
  rulesEditor: { open: false, ruleId: null, draft: null, seq: 0 },
  breakpointsOpen: false,
  selectedBreakpoint: null,
  deviceSetupOpen: false,
  deviceTab: "simulator",
  settings: { open: false, page: "mcp" },

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
  setRequestTab: (requestTab) => set({ requestTab }),
  setResponseTab: (responseTab) => set({ responseTab }),
  setViewer: (kind, viewer) => set((s) => ({ viewerByKind: { ...s.viewerByKind, [kind]: viewer } })),
  setCertWizardOpen: (certWizardOpen) => set({ certWizardOpen }),
  openRulesEditor: (target = {}) =>
    set((s) => ({
      rulesEditor: {
        open: true,
        draft: target.draft ?? null,
        ruleId: target.draft ? null : target.ruleId !== undefined ? target.ruleId : s.rulesEditor.ruleId,
        // Remounts the form, so a new draft never inherits the last one's edits.
        seq: s.rulesEditor.seq + 1,
      },
    })),
  closeRulesEditor: () => set((s) => ({ rulesEditor: { ...s.rulesEditor, open: false, draft: null } })),
  setBreakpointsOpen: (breakpointsOpen) => set({ breakpointsOpen }),
  selectBreakpoint: (selectedBreakpoint) => set({ selectedBreakpoint }),
  setDeviceSetupOpen: (deviceSetupOpen) => set({ deviceSetupOpen }),
  setDeviceTab: (deviceTab) => set({ deviceTab }),
  openSettings: (page) => set((s) => ({ settings: { open: true, page: page ?? s.settings.page } })),
  closeSettings: () => set((s) => ({ settings: { ...s.settings, open: false } })),
}));
