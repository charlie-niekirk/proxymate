import { SEARCH_INPUT_ID } from "@/components/FilterBar";
import { focusFlowTable, focusInspector } from "@/lib/focus";
import { SIDEBAR_FILTER_ID } from "@/components/Sidebar";
import {
  clearFlows,
  compareSelection,
  copyFlowCode,
  copyFlowCurl,
  copyFlowUrl,
  copySelectionUrls,
  deleteSelection,
  editAndReplay,
  exportHar,
  labelSelection,
  openDocument,
  openRewrite,
  pinFlows,
  replayFlow,
  replaySelection,
  reportError,
  saveSession,
  setRecording,
} from "@/lib/actions";
import { CODE_LANGUAGES, FLOW_LABELS } from "@/lib/flow";
import { FLOW_TABLE_ID } from "@/lib/focus";
import { selectionCount } from "@/lib/selection";
import { backend } from "@/lib/backend";
import { proxyStatusKey, queryClient } from "@/lib/query";
import { usePrefs } from "@/store/prefs";
import { LIVE_STORE, useUi, type SettingsPage } from "@/store/ui";

export type CommandGroup = "Flows" | "Capture" | "Navigate" | "Windows" | "Settings";

/**
 * One thing the app can do from the keyboard: listed in the ⌘K palette,
 * and bound to `key` (with ⌘, or Ctrl elsewhere) when it has one.
 */
export interface AppCommand {
  id: string;
  title: string;
  group: CommandGroup;
  /** The key pressed with ⌘: a letter, `Backspace` or `,`. */
  key?: string;
  /** With ⌥ as well (⌥⌘R). */
  alt?: boolean;
  /**
   * The key alone, without ⌘ (⌫ deletes the selection), and only while the
   * flow table has the focus.
   */
  bare?: boolean;
  /** Only while the flow table has the focus (⌘A selects every flow). */
  inTable?: boolean;
  /** Extra words the palette matches on. */
  keywords?: string[];
  /** False hides it from the palette and ignores its key. */
  enabled?: () => boolean;
  run: () => void | Promise<void>;
}

/** How a command's key is shown: ⌘R, ⌘⌫, ⌥⌘R, ⌫. */
export function shortcutLabel(key: string, mods: { alt?: boolean; bare?: boolean } = {}): string {
  const mac = navigator.userAgent.includes("Mac");
  const name = key === "Backspace" ? (mac ? "⌫" : "Backspace") : key.length === 1 ? key.toUpperCase() : key;
  if (mods.bare) return name;
  if (mac) return `${mods.alt ? "⌥" : ""}⌘${name}`;
  return `Ctrl+${mods.alt ? "Alt+" : ""}${name}`;
}

/** The selected flow's summary, if one is selected. */
async function selectedFlow() {
  const { store, selectedId } = useUi.getState();
  if (selectedId === null) return null;
  return backend.getFlow(store, selectedId);
}

/** Replay needs an HTTP request that isn't a tunnel or a WebSocket. */
async function sendableSelection(): Promise<number | null> {
  const flow = await selectedFlow();
  if (!flow) {
    reportError("Nothing to replay", "select a flow first");
    return null;
  }
  if (flow.method === "CONNECT" || flow.scheme === "ws" || flow.scheme === "wss") {
    reportError("Can't replay this flow", "tunnels and WebSockets can't be sent again yet");
    return null;
  }
  return flow.id;
}

const hasSelection = () => useUi.getState().selectedId !== null;
const selectedCount = () => selectionCount(useUi.getState().selection, useUi.getState().stats.len);
const isLive = () => useUi.getState().store === LIVE_STORE;

function focusInput(id: string) {
  const input = document.getElementById(id) as HTMLInputElement | null;
  input?.focus();
  input?.select();
}

const SETTINGS_PAGES: [SettingsPage, string][] = [
  ["proxy", "Proxy"],
  ["ssl", "SSL Proxying"],
  ["clients", "Clients"],
  ["storage", "Storage"],
  ["appearance", "Appearance"],
  ["advanced", "Advanced"],
  ["mcp", "MCP"],
];

export const COMMANDS: AppCommand[] = [
  {
    id: "replay",
    title: "Replay selected flow",
    group: "Flows",
    key: "r",
    enabled: hasSelection,
    run: async () => {
      const id = await sendableSelection();
      if (id !== null) await replayFlow(useUi.getState().store, id).catch((e: unknown) => reportError("Replay failed", e));
    },
  },
  {
    id: "edit-replay",
    title: "Edit & Replay selected flow…",
    group: "Flows",
    keywords: ["compose", "resend"],
    enabled: hasSelection,
    run: async () => {
      const id = await sendableSelection();
      if (id !== null) await editAndReplay(useUi.getState().store, id);
    },
  },
  {
    id: "rewrite",
    title: "Rewrite selected flow…",
    group: "Flows",
    keywords: ["edit", "modify", "rule", "json"],
    enabled: hasSelection,
    run: async () => {
      const id = await sendableSelection();
      if (id !== null) await openRewrite(useUi.getState().store, id);
    },
  },
  {
    id: "pin",
    title: "Pin or unpin selected flow",
    group: "Flows",
    keywords: ["keep", "favorite"],
    enabled: () => hasSelection() && isLive(),
    run: async () => {
      const flow = await selectedFlow();
      if (flow) await pinFlows([flow.id], !flow.pinned);
    },
  },
  {
    id: "copy-url",
    title: "Copy URL of selected flow",
    group: "Flows",
    enabled: hasSelection,
    run: () =>
      selectedCount() > 1 ? copySelectionUrls() : copyFlowUrl(useUi.getState().store, useUi.getState().selectedId!),
  },
  ...CODE_LANGUAGES.map(
    (l): AppCommand => ({
      id: `copy-code-${l.id}`,
      title: `Copy selected flow as ${l.label}`,
      group: "Flows",
      keywords: ["code", "snippet", "export"],
      enabled: hasSelection,
      run: () => copyFlowCode(useUi.getState().store, useUi.getState().selectedId!, l.id),
    }),
  ),
  {
    id: "repeat",
    title: "Repeat selected flow…",
    group: "Flows",
    key: "r",
    alt: true,
    keywords: ["load", "iterations", "rate limit", "stress"],
    enabled: hasSelection,
    run: async () => {
      const id = await sendableSelection();
      if (id !== null) useUi.getState().openRepeat(useUi.getState().store, id);
    },
  },
  {
    id: "select-all",
    title: "Select all flows",
    group: "Flows",
    key: "a",
    inTable: true,
    run: () => useUi.getState().setSelection({ all: true, ids: new Set() }),
  },
  {
    id: "delete",
    title: "Delete selected flows",
    group: "Flows",
    key: "Backspace",
    bare: true,
    keywords: ["remove"],
    enabled: () => selectedCount() > 0,
    run: () => deleteSelection().then(() => undefined).catch((e: unknown) => reportError("Delete failed", e)),
  },
  {
    id: "replay-selection",
    title: "Replay selected flows in order",
    group: "Flows",
    enabled: () => selectedCount() > 1,
    run: () => replaySelection().catch((e: unknown) => reportError("Replay failed", e)),
  },
  {
    id: "compare",
    title: "Compare the two selected flows",
    group: "Flows",
    keywords: ["diff"],
    enabled: () => selectedCount() === 2,
    run: () => compareSelection(),
  },
  {
    id: "comment",
    title: "Comment on selected flow…",
    group: "Flows",
    keywords: ["note", "annotate"],
    enabled: hasSelection,
    run: () => useUi.getState().openComment(useUi.getState().store, useUi.getState().selectedId!),
  },
  ...FLOW_LABELS.map(
    (label): AppCommand => ({
      id: `label-${label}`,
      title: `Label selected flows ${label[0]!.toUpperCase()}${label.slice(1)}`,
      group: "Flows",
      keywords: ["colour", "color", "tag", "mark"],
      enabled: () => selectedCount() > 0,
      run: () => labelSelection(label).catch((e: unknown) => reportError("Label failed", e)),
    }),
  ),
  {
    id: "label-none",
    title: "Remove label from selected flows",
    group: "Flows",
    keywords: ["colour", "color", "tag"],
    enabled: () => selectedCount() > 0,
    run: () => labelSelection(null).catch((e: unknown) => reportError("Label failed", e)),
  },
  {
    id: "add-header-column",
    title: "Add header column…",
    group: "Flows",
    keywords: ["column", "header", "table"],
    run: () => useUi.getState().setAddColumnOpen(true),
  },
  {
    id: "copy-curl",
    title: "Copy selected flow as cURL",
    group: "Flows",
    enabled: hasSelection,
    run: () => copyFlowCurl(useUi.getState().store, useUi.getState().selectedId!),
  },
  {
    id: "compose",
    title: "New request…",
    group: "Flows",
    keywords: ["compose"],
    run: () => useUi.getState().openCompose(),
  },
  {
    id: "clear",
    title: "Clear all flows",
    group: "Capture",
    key: "Backspace",
    enabled: isLive,
    run: () => clearFlows(),
  },
  {
    id: "record",
    title: "Pause or resume recording",
    group: "Capture",
    key: "e",
    keywords: ["record", "capture"],
    run: async () => {
      const status = queryClient.getQueryData<{ recording: boolean }>(proxyStatusKey) ?? (await backend.getProxyStatus());
      await setRecording(!status.recording);
    },
  },
  {
    id: "auto-select",
    title: "Toggle Auto Select",
    group: "Capture",
    keywords: ["follow", "scroll"],
    run: () => useUi.getState().setAutoScroll(!useUi.getState().autoScroll),
  },
  {
    id: "open",
    title: "Open session or HAR…",
    group: "Capture",
    run: () => openDocument(),
  },
  {
    id: "save-session",
    title: "Save session…",
    group: "Capture",
    run: () => saveSession(),
  },
  {
    id: "export-har",
    title: "Export HAR…",
    group: "Capture",
    run: () => exportHar(),
  },
  {
    id: "search",
    title: "Search flows",
    group: "Navigate",
    key: "f",
    keywords: ["find", "filter", "dsl"],
    run: () => focusInput(SEARCH_INPUT_ID),
  },
  {
    id: "filter",
    title: "Filter apps and domains",
    group: "Navigate",
    key: "l",
    keywords: ["sidebar"],
    run: () => focusInput(SIDEBAR_FILTER_ID),
  },
  {
    id: "pinned",
    title: "Show pinned flows",
    group: "Navigate",
    run: () => useUi.getState().setText("is:pinned"),
  },
  {
    id: "labelled",
    title: "Show labelled flows",
    group: "Navigate",
    keywords: ["colour", "color", "tag"],
    run: () => useUi.getState().setText("is:labelled"),
  },
  {
    id: "commented",
    title: "Show commented flows",
    group: "Navigate",
    keywords: ["note"],
    run: () => useUi.getState().setText("is:commented"),
  },
  {
    id: "deep-search",
    title: "Search request and response bodies…",
    group: "Navigate",
    keywords: ["body", "header", "deep"],
    run: () => {
      const text = useUi.getState().filter.text;
      if (!/\bbody:/.test(text)) useUi.getState().setText(`${text ? `${text} ` : ""}body:`);
      document.getElementById(SEARCH_INPUT_ID)?.focus();
    },
  },
  { id: "table", title: "Focus the flow table", group: "Navigate", run: focusFlowTable },
  { id: "inspector", title: "Focus the inspector", group: "Navigate", enabled: hasSelection, run: focusInspector },
  {
    id: "live",
    title: "Show live capture",
    group: "Navigate",
    enabled: () => !isLive(),
    run: () => useUi.getState().setStore(LIVE_STORE),
  },
  { id: "rules", title: "Rules…", group: "Windows", keywords: ["map local", "rewrite"], run: () => useUi.getState().openRulesEditor() },
  {
    id: "scripts",
    title: "Scripts…",
    group: "Windows",
    keywords: ["javascript", "decode", "base64", "mock", "view"],
    run: () => useUi.getState().openScripts(),
  },
  { id: "breakpoints", title: "Breakpoints…", group: "Windows", run: () => useUi.getState().setBreakpointsOpen(true) },
  {
    id: "devices",
    title: "Device setup…",
    group: "Windows",
    keywords: ["iphone", "android", "simulator"],
    run: () => useUi.getState().setDeviceSetupOpen(true),
  },
  {
    id: "certificate",
    title: "Certificate wizard…",
    group: "Windows",
    keywords: ["ca", "trust", "https"],
    run: () => useUi.getState().setCertWizardOpen(true),
  },
  { id: "settings", title: "Settings…", group: "Settings", key: ",", run: () => useUi.getState().openSettings() },
  ...SETTINGS_PAGES.map(
    ([page, label]): AppCommand => ({
      id: `settings-${page}`,
      title: `Settings: ${label}`,
      group: "Settings",
      run: () => useUi.getState().openSettings(page),
    }),
  ),
  ...(["system", "light", "dark"] as const).map(
    (appearance): AppCommand => ({
      id: `appearance-${appearance}`,
      title: `Appearance: ${appearance[0]!.toUpperCase()}${appearance.slice(1)}`,
      group: "Settings",
      keywords: ["theme", "mode"],
      run: () => usePrefs.getState().setAppearance(appearance),
    }),
  ),
];

/** Text fields and editors, where ⌘⌫ deletes text rather than flows. */
export function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    target.closest(".cm-editor") !== null ||
    // The JSON editor's own keys (⌫ removes a row, ⌘D duplicates).
    target.closest("[data-json-editor]") !== null
  );
}

function inFlowTable(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && target.closest(`#${FLOW_TABLE_ID}`) !== null;
}

/** The command `e` triggers, if any (the palette's own ⌘K aside). */
export function commandForKey(e: KeyboardEvent): AppCommand | undefined {
  if (e.shiftKey) return undefined;
  const modified = e.metaKey || e.ctrlKey;
  // ⌥ turns letters into symbols on a Mac keyboard: match the physical key.
  const key = e.key.length === 1 ? (e.altKey && /^Key[A-Z]$/.test(e.code) ? e.code.slice(3) : e.key).toLowerCase() : e.key;
  const command = COMMANDS.find(
    (c) => c.key === key && (c.bare ? !modified && !e.altKey : modified && Boolean(c.alt) === e.altKey),
  );
  if (!command || command.enabled?.() === false) return undefined;
  if ((command.bare || command.inTable) && !inFlowTable(e.target)) return undefined;
  // In text, ⌘⌫ deletes to the start of the line; in a body viewer ⌘F
  // searches the body.
  if (command.id === "clear" && isEditable(e.target)) return undefined;
  if (command.id === "search" && e.target instanceof HTMLElement && e.target.closest(".cm-editor")) return undefined;
  return command;
}
