import { SEARCH_INPUT_ID } from "@/components/FilterBar";
import { focusFlowTable, focusInspector } from "@/lib/focus";
import { SIDEBAR_FILTER_ID } from "@/components/Sidebar";
import {
  clearFlows,
  copyFlowCurl,
  copyFlowUrl,
  editAndReplay,
  exportHar,
  openDocument,
  pinFlows,
  replayFlow,
  reportError,
  saveSession,
  setRecording,
} from "@/lib/actions";
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
  /** Extra words the palette matches on. */
  keywords?: string[];
  /** False hides it from the palette and ignores its key. */
  enabled?: () => boolean;
  run: () => void | Promise<void>;
}

/** How a command's key is shown: ⌘R, ⌘⌫. */
export function shortcutLabel(key: string): string {
  const mac = navigator.userAgent.includes("Mac");
  const name = key === "Backspace" ? "⌫" : key.length === 1 ? key.toUpperCase() : key;
  return mac ? `⌘${name}` : `Ctrl+${name}`;
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
    run: () => copyFlowUrl(useUi.getState().store, useUi.getState().selectedId!),
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
    target.closest(".cm-editor") !== null
  );
}

/** The command `e` triggers, if any (the palette's own ⌘K aside). */
export function commandForKey(e: KeyboardEvent): AppCommand | undefined {
  if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return undefined;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const command = COMMANDS.find((c) => c.key === key);
  if (!command || command.enabled?.() === false) return undefined;
  // In text, ⌘⌫ deletes to the start of the line; in a body viewer ⌘F
  // searches the body.
  if (command.id === "clear" && isEditable(e.target)) return undefined;
  if (command.id === "search" && e.target instanceof HTMLElement && e.target.closest(".cm-editor")) return undefined;
  return command;
}
