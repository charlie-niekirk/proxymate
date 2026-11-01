import { create } from "zustand";

import type {
  ContentGroup,
  ContentKind,
  DeepProgress,
  DocumentInfo,
  FilterSpec_Deserialize,
  ProtocolFilter,
  RequestSpec_Serialize,
  SaveProgress_Serialize,
  SortSpec,
} from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import { EMPTY_SELECTION, single, toggle, type TableSelection } from "@/lib/selection";
import type { Rule } from "@/lib/rules";
import type { Script } from "@/lib/scripts";
import { usePrefs } from "@/store/prefs";

export interface ViewStats {
  len: number;
  total: number;
}

/** Inspector tabs; the choice sticks as the selection changes. */
export type RequestTab = "Header" | "Query" | "Body" | "Raw";
export type ResponseTab = "Header" | "Body" | "Raw" | "TLS" | "Timing";

/** Device Setup Wizard tabs. */
export type DeviceTab = "simulator" | "iphone" | "android" | "emulator";

/** Settings pages, in the order the dialog lists them. */
export type SettingsPage =
  | "proxy"
  | "network"
  | "ssl"
  | "clients"
  | "storage"
  | "protobuf"
  | "mocking"
  | "appearance"
  | "advanced"
  | "mcp"
  | "updates";

/** The live capture's store; opened documents have their own. */
export const LIVE_STORE = 0;

/** A background save or export, for the footer. */
export interface SaveJob {
  /** What is being written, e.g. "Saving session". */
  label: string;
  progress: SaveProgress_Serialize;
}

/** A flow in some store (Compare takes flows from any tabs). */
export interface FlowRef {
  store: number;
  id: number;
}

export interface FilterTiming {
  /** Engine refilter time reported by Rust. */
  engineMs: number;
  /** Filter change to visible rows painted. */
  endToEndMs: number;
}

interface UiState {
  /** The store the table and inspector show: the live capture or a document tab. */
  store: number;
  /** Opened sessions and HARs, in tab order. */
  documents: DocumentInfo[];
  /** The flow table's engine view (its filter and sort), for "export what I see". */
  view: number | null;
  /** The Compose window: open or not, and what it starts from. */
  compose: { open: boolean; spec: RequestSpec_Serialize | null; seq: number };
  save: SaveJob | null;
  filter: FilterSpec_Deserialize;
  sort: SortSpec;
  /** The focused flow: the inspector shows it. Always in `selection`, if any. */
  selectedId: number | null;
  /** Every selected flow (M17); bulk actions take it. */
  selection: TableSelection;
  /** Two flows side by side (M17), from any stores. */
  compare: { open: boolean; a: FlowRef | null; b: FlowRef | null };
  /** "Mark for Compare": the flow a later "Compare with Marked" (any tab) goes against. */
  compareMark: FlowRef | null;
  /** The Repeat dialog (M17) for a flow of `store`. */
  repeat: { open: boolean; store: number; flow: number | null };
  /** The comment editor (M17) for a flow of `store`. */
  comment: { open: boolean; store: number; flow: number | null };
  /** The "Add header column" dialog (M17). */
  addColumnOpen: boolean;
  stats: ViewStats;
  /** The table's deep search (header and body terms), while its filter has any. */
  deep: DeepProgress | null;
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
  /**
   * The Rewrite dialog: a flow of `store` to edit, and the rule it edits
   * (a saved one reopened visually), or null for a new rewrite.
   */
  rewrite: { open: boolean; store: number; flow: number | null; rule: Rule | null; seq: number };
  /** The Scripts window, like the Rules Editor: a saved script or an unsaved draft. */
  scripts: { open: boolean; scriptId: string | null; draft: Script | null; seq: number };
  /** The script view last picked in a body viewer (`script:<id>:<n>`); sticks across flows. */
  pickedScriptView: string | null;
  /** The user chose a built-in viewer over script views: stop auto-selecting them. */
  scriptViewsOff: boolean;
  breakpointsOpen: boolean;
  /** The breakpoint shown in the Breakpoint Editor. */
  selectedBreakpoint: number | null;
  deviceSetupOpen: boolean;
  deviceTab: DeviceTab;
  settings: { open: boolean; page: SettingsPage };
  paletteOpen: boolean;

  /** Switches tabs; the selection doesn't carry over (ids are per store). */
  setStore(store: number): void;
  setDocuments(documents: DocumentInfo[]): void;
  setView(view: number | null): void;
  openCompose(spec?: RequestSpec_Serialize | null): void;
  closeCompose(): void;
  setSave(save: SaveJob | null): void;
  setText(text: string): void;
  setProtocol(protocol: ProtocolFilter): void;
  toggleContent(group: ContentGroup): void;
  clearContent(): void;
  setScope(scope: { host?: string | null; client?: string | null }): void;
  setSort(sort: SortSpec): void;
  /** Selects just this flow (or nothing) and focuses it. */
  select(id: number | null): void;
  /** ⌘-click: adds or removes a flow, focusing it when added. */
  toggleSelected(id: number): void;
  /** Replaces the selection (⇧-click, ⌘A), focusing `focus`. */
  setSelection(selection: TableSelection, focus?: number | null): void;
  openCompare(a: FlowRef, b: FlowRef): void;
  markForCompare(flow: FlowRef | null): void;
  closeCompare(): void;
  openRepeat(store: number, flow: number): void;
  closeRepeat(): void;
  openComment(store: number, flow: number): void;
  closeComment(): void;
  setAddColumnOpen(open: boolean): void;
  setStats(stats: ViewStats): void;
  setDeep(deep: DeepProgress | null): void;
  setFilterTiming(timing: FilterTiming): void;
  setAutoScroll(on: boolean): void;
  setRequestTab(tab: RequestTab): void;
  setResponseTab(tab: ResponseTab): void;
  setViewer(kind: ContentKind, viewer: ViewerId): void;
  setCertWizardOpen(open: boolean): void;
  openRulesEditor(target?: { ruleId?: string | null; draft?: Rule | null }): void;
  closeRulesEditor(): void;
  openRewrite(target: { store: number; flow: number; rule?: Rule | null }): void;
  closeRewrite(): void;
  openScripts(target?: { scriptId?: string | null; draft?: Script | null }): void;
  closeScripts(): void;
  /** A script view (or, with null, a built-in viewer) was picked in a body viewer. */
  pickScriptView(value: string | null): void;
  setBreakpointsOpen(open: boolean): void;
  selectBreakpoint(id: number | null): void;
  setDeviceSetupOpen(open: boolean): void;
  setDeviceTab(tab: DeviceTab): void;
  openSettings(page?: SettingsPage): void;
  closeSettings(): void;
  setPaletteOpen(open: boolean): void;
}

export const useUi = create<UiState>()((set) => ({
  store: LIVE_STORE,
  documents: [],
  view: null,
  compose: { open: false, spec: null, seq: 0 },
  save: null,
  filter: { text: "", protocol: "all", content: [], host: null, client: null },
  // Remembered across launches (store/prefs.ts).
  sort: usePrefs.getState().sort,
  selectedId: null,
  selection: EMPTY_SELECTION,
  compare: { open: false, a: null, b: null },
  compareMark: null,
  repeat: { open: false, store: LIVE_STORE, flow: null },
  comment: { open: false, store: LIVE_STORE, flow: null },
  addColumnOpen: false,
  stats: { len: 0, total: 0 },
  deep: null,
  filterTiming: null,
  autoScroll: false,
  requestTab: "Header",
  responseTab: "Body",
  viewerByKind: {},
  certWizardOpen: false,
  rulesEditor: { open: false, ruleId: null, draft: null, seq: 0 },
  rewrite: { open: false, store: LIVE_STORE, flow: null, rule: null, seq: 0 },
  scripts: { open: false, scriptId: null, draft: null, seq: 0 },
  pickedScriptView: null,
  scriptViewsOff: false,
  breakpointsOpen: false,
  selectedBreakpoint: null,
  deviceSetupOpen: false,
  deviceTab: "simulator",
  settings: { open: false, page: "proxy" },
  paletteOpen: false,

  setStore: (store) =>
    set((s) => (s.store === store ? {} : { store, selectedId: null, selection: EMPTY_SELECTION, stats: { len: 0, total: 0 } })),
  setDocuments: (documents) => set({ documents }),
  setView: (view) => set({ view }),
  // Bumping `seq` remounts the form, so a new spec never inherits the last one's edits.
  openCompose: (spec = null) => set((s) => ({ compose: { open: true, spec, seq: s.compose.seq + 1 } })),
  closeCompose: () => set((s) => ({ compose: { ...s.compose, open: false } })),
  setSave: (save) => set({ save }),
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
  setSort: (sort) => {
    usePrefs.getState().setSort(sort);
    set({ sort });
  },
  select: (selectedId) => set({ selectedId, selection: single(selectedId) }),
  toggleSelected: (id) =>
    set((s) => {
      const selection = toggle(s.selection, id);
      const added = selection.all ? !selection.ids.has(id) : selection.ids.has(id);
      // Unselecting the focused flow moves the focus to another selected one.
      const rest = selection.all ? null : [...selection.ids].at(-1);
      const selectedId = added ? id : s.selectedId === id ? (rest ?? null) : s.selectedId;
      return { selection, selectedId };
    }),
  setSelection: (selection, focus) => set((s) => ({ selection, selectedId: focus === undefined ? s.selectedId : focus })),
  openCompare: (a, b) => set({ compare: { open: true, a, b } }),
  markForCompare: (compareMark) => set({ compareMark }),
  closeCompare: () => set((s) => ({ compare: { ...s.compare, open: false } })),
  openRepeat: (store, flow) => set({ repeat: { open: true, store, flow } }),
  closeRepeat: () => set((s) => ({ repeat: { ...s.repeat, open: false } })),
  openComment: (store, flow) => set({ comment: { open: true, store, flow } }),
  closeComment: () => set((s) => ({ comment: { ...s.comment, open: false } })),
  setAddColumnOpen: (addColumnOpen) => set({ addColumnOpen }),
  setStats: (stats) => set({ stats }),
  setDeep: (deep) => set((s) => (sameDeep(s.deep, deep) ? {} : { deep })),
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
  // Bumping `seq` reloads the dialog, so it never shows the last flow's edits.
  openRewrite: ({ store, flow, rule = null }) =>
    set((s) => ({ rewrite: { open: true, store, flow, rule, seq: s.rewrite.seq + 1 } })),
  closeRewrite: () => set((s) => ({ rewrite: { ...s.rewrite, open: false } })),
  openScripts: (target = {}) =>
    set((s) => ({
      scripts: {
        open: true,
        draft: target.draft ?? null,
        scriptId: target.draft ? null : target.scriptId !== undefined ? target.scriptId : s.scripts.scriptId,
        seq: s.scripts.seq + 1,
      },
    })),
  closeScripts: () => set((s) => ({ scripts: { ...s.scripts, open: false, draft: null } })),
  pickScriptView: (value) => set({ pickedScriptView: value, scriptViewsOff: value === null }),
  setBreakpointsOpen: (breakpointsOpen) => set({ breakpointsOpen }),
  selectBreakpoint: (selectedBreakpoint) => set({ selectedBreakpoint }),
  setDeviceSetupOpen: (deviceSetupOpen) => set({ deviceSetupOpen }),
  setDeviceTab: (deviceTab) => set({ deviceTab }),
  openSettings: (page) => set((s) => ({ settings: { open: true, page: page ?? s.settings.page } })),
  closeSettings: () => set((s) => ({ settings: { ...s.settings, open: false } })),
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
}));

function sameDeep(a: DeepProgress | null, b: DeepProgress | null): boolean {
  if (a === null || b === null) return a === b;
  return a.checked === b.checked && a.total === b.total && a.matched === b.matched && a.running === b.running;
}
