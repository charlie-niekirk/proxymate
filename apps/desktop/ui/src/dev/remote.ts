import type { ContentKind, MessageEdits_Deserialize, PresetKind, RequestSpec_Serialize } from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import { bridge } from "@/dev/bridge";
import { backend } from "@/lib/backend";
import {
  closeDocument,
  editAndReplay,
  exportHar,
  openDocument,
  openPreset,
  openRewrite,
  replayFlow,
  saveSession,
  setThrottle,
} from "@/lib/actions";
import { normalizeRule, THROTTLE_PRESETS } from "@/lib/rules";
import type { PointerEdit } from "@/components/Rewrite/RewriteDialog";
import { forceTheme } from "@/lib/theme";
import { usePrefs } from "@/store/prefs";
import { useUi, type DeviceTab, type RequestTab, type ResponseTab, type SettingsPage } from "@/store/ui";

/**
 * Dev only (`VITE_PM_REMOTE=1`): polls the Vite server's `/__pm_remote`
 * mailbox and applies each command, so the real app can be driven and
 * screenshotted from a shell, where synthetic input is unavailable.
 *
 * `curl -d '<json>' localhost:<vite port>/__pm_remote`, one command object
 * per POST (fields apply in the order below):
 * - `{"select": 3}`, `{"requestTab": "Body"}`, `{"responseTab": "TLS"}`,
 *   `{"viewer": {"kind": "json", "viewer": "tree"}}`, `{"theme": "light"}`
 * - M10: `{"appearance": "light"}` (as Settings › Appearance would),
 *   `{"palette": true}`, `{"filter": "body:token"}`, `{"reload": true}`
 * - `{"contextMenu": 3}`: right-clicks flow 3's row (it must be on screen),
 *   opening the flow menu for a screenshot. `{"contextMenu": null}` closes it.
 * - `{"preset": {"flow": 3, "kind": "map_local"}}` (or `breakpoint`,
 *   `rewrite`): exactly what the menu's "Map Local…" etc. do.
 * - M13: `{"rewrite": {"flow": 3}}` opens the Rewrite dialog on the active
 *   tab's flow 3; `{"rewrite": {"rule": "<id>"}}` reopens a saved rule on a
 *   flow it matches. `{"rewriteEdit": {"side": "response", "tab": "Body",
 *   "json": [{"set": "/name", "raw": "\"x\""}, {"rename": "/a", "key": "b"},
 *   {"add": "", "key": "k", "raw": "1"}, {"remove": "/c"}], "headers": [{"name":
 *   "X-A", "value": "1"}], "whole": false, "save": true}}` edits it like a user.
 * - `{"rulesEditor": true}` / `false`: opens or closes the Rules Editor.
 * - `{"testRule": true}`: presses "Test against selected flow".
 * - `{"saveRule": true}`: presses Save in the open Rules Editor.
 * - `{"throttle": "3G"}` (a preset name from `lib/rules.ts`, or "Off").
 * - `{"breakpoints": true}` / `false`: opens or closes the Breakpoint Editor.
 * - `{"breakpointAction": {"execute": {"url": "…", "headers": [...], "body": "…"}}}`
 *   presses Execute on the selected paused flow, merging these edits over
 *   any made in the form (`{"execute": {}}` sends the form as is);
 *   `{"breakpointAction": {"abort": true}}` presses Abort.
 * - `{"store": 1}`: switches to that document tab (0 is the live capture).
 * - `{"replay": 3}`, `{"editReplay": 3}`: the flow menu's Replay and Edit &
 *   Replay, on the active tab's flow 3.
 * - `{"compose": {"method": "POST", "url": "…", "headers": [], "body": "…"}}`
 *   opens Compose with that request (`{"compose": null}`: a new one);
 *   `{"composeSend": true}` presses Send.
 * - `{"curl": 3}`: posts Copy as cURL's command for flow 3 to the mailbox's
 *   outbox, `curl localhost:<vite port>/__pm_remote/out`.
 * - `{"openDocument": "/path/x.har"}`, `{"closeDocument": 1}`.
 * - `{"saveSession": "/path/x.proxymate"}` saves the active tab;
 *   `{"exportHar": "/path/x.har"}` exports what the table shows.
 */
interface Command {
  select?: number;
  requestTab?: RequestTab;
  responseTab?: ResponseTab;
  viewer?: { kind: ContentKind; viewer: ViewerId };
  theme?: "dark" | "light";
  /** Settings › Appearance, as the user would set it (the native window follows). */
  appearance?: "system" | "dark" | "light";
  palette?: boolean;
  /** Sets the search box's text (the filter DSL). */
  filter?: string;
  /** Reloads the page (with `VITE_PM_PERF_PROBE`, the probe runs again). */
  reload?: boolean;
  contextMenu?: number | null;
  preset?: { flow: number; kind: PresetKind };
  /** Opens the Rewrite dialog on a flow, or reopens a saved rule (by id) visually; `null` closes it. */
  rewrite?: { flow?: number; rule?: string } | null;
  /** Edits in the open Rewrite dialog, then optionally saves (see `bridge.rewrite`). */
  rewriteEdit?: {
    side?: "request" | "response";
    tab?: "Headers" | "Query" | "Body" | "Status";
    json?: PointerEdit[];
    headers?: { name: string; value: string | null }[];
    query?: { name: string; value: string | null }[];
    form?: { name: string; value: string | null }[];
    status?: number;
    whole?: boolean;
    panel?: "preview" | "actions";
    save?: boolean;
  };
  rulesEditor?: boolean;
  testRule?: boolean;
  saveRule?: boolean;
  throttle?: string;
  breakpoints?: boolean;
  breakpointAction?: { execute?: MessageEdits_Deserialize; abort?: boolean };
  /** Opens device setup on this tab; `null` closes it. */
  deviceTab?: DeviceTab | null;
  settings?: SettingsPage | null;
  scrollTo?: string;
  /** Clicks the element with this `data-testid`, e.g. `simulator-install`. */
  click?: string;
  store?: number;
  replay?: number;
  editReplay?: number;
  compose?: Partial<RequestSpec_Serialize> | null;
  composeSend?: boolean;
  curl?: number;
  openDocument?: string;
  closeDocument?: number;
  saveSession?: string;
  exportHar?: string;
}

async function apply(cmd: Command) {
  const ui = useUi.getState();
  if (cmd.select !== undefined) ui.select(cmd.select);
  if (cmd.requestTab) ui.setRequestTab(cmd.requestTab);
  if (cmd.responseTab) ui.setResponseTab(cmd.responseTab);
  if (cmd.viewer) ui.setViewer(cmd.viewer.kind, cmd.viewer.viewer);
  if (cmd.deviceTab !== undefined) {
    if (cmd.deviceTab) ui.setDeviceTab(cmd.deviceTab);
    ui.setDeviceSetupOpen(cmd.deviceTab !== null);
  }
  if (cmd.settings !== undefined) {
    if (cmd.settings) ui.openSettings(cmd.settings);
    else ui.closeSettings();
  }
  if (cmd.scrollTo) {
    const id = cmd.scrollTo;
    setTimeout(() => document.querySelector(`[data-testid="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "end" }), 100);
  }
  if (cmd.click) {
    // After this tick's state changes have rendered.
    const id = cmd.click;
    setTimeout(() => document.querySelector<HTMLElement>(`[data-testid="${CSS.escape(id)}"]`)?.click(), 100);
  }
  if (cmd.appearance) usePrefs.getState().setAppearance(cmd.appearance);
  if (cmd.palette !== undefined) useUi.getState().setPaletteOpen(cmd.palette);
  if (cmd.filter !== undefined) useUi.getState().setText(cmd.filter);
  if (cmd.reload) window.location.reload();
  if (cmd.theme) {
    forceTheme(cmd.theme);
  }
  if (cmd.contextMenu !== undefined) openContextMenu(cmd.contextMenu);
  if (cmd.preset) await openPreset(useUi.getState().store, cmd.preset.flow, cmd.preset.kind);
  if (cmd.rewrite !== undefined) {
    if (cmd.rewrite === null) ui.closeRewrite();
    else await openRewrite(useUi.getState().store, cmd.rewrite.flow ?? null, cmd.rewrite.rule ? await savedRule(cmd.rewrite.rule) : null);
  }
  const rw = bridge.rewrite;
  if (cmd.rewriteEdit && rw) {
    const e = cmd.rewriteEdit;
    if (e.side) rw.side(e.side);
    if (e.tab) rw.tab(e.tab);
    const side = e.side ?? "response";
    for (const op of e.json ?? []) rw.edit(side, op);
    for (const h of e.headers ?? []) rw.header(side, h);
    for (const q of e.query ?? []) rw.query(q);
    for (const f of e.form ?? []) rw.form(side, f);
    if (e.status !== undefined) rw.status(e.status);
    if (e.whole !== undefined) rw.whole(side, e.whole);
    if (e.panel) rw.panel(e.panel);
    if (e.save) await rw.save();
  }
  if (cmd.rulesEditor === true) ui.openRulesEditor();
  if (cmd.rulesEditor === false) ui.closeRulesEditor();
  if (cmd.testRule) bridge.testRule?.();
  if (cmd.saveRule) await bridge.saveRule?.();
  if (cmd.throttle !== undefined) await setThrottle(THROTTLE_PRESETS.find((p) => p.name === cmd.throttle) ?? null);
  if (cmd.breakpoints !== undefined) ui.setBreakpointsOpen(cmd.breakpoints);
  if (cmd.breakpointAction?.execute) await bridge.executeBreakpoint?.(cmd.breakpointAction.execute);
  if (cmd.breakpointAction?.abort) await bridge.abortBreakpoint?.();
  if (cmd.store !== undefined) ui.setStore(cmd.store);
  if (cmd.openDocument) await openDocument(cmd.openDocument);
  if (cmd.closeDocument !== undefined) await closeDocument(cmd.closeDocument);
  const store = useUi.getState().store;
  if (cmd.replay !== undefined) await replayFlow(store, cmd.replay);
  if (cmd.editReplay !== undefined) await editAndReplay(store, cmd.editReplay);
  if (cmd.compose !== undefined) {
    const spec = cmd.compose && { method: "GET", url: "", headers: [], body: null, body_base64: false, ...cmd.compose };
    ui.openCompose(spec);
  }
  if (cmd.composeSend) await bridge.sendCompose?.();
  if (cmd.curl !== undefined) await post("/__pm_remote/out", await backend.getCurl(store, cmd.curl));
  if (cmd.saveSession) await saveSession({ kind: "all" }, cmd.saveSession);
  if (cmd.exportHar) await exportHar(undefined, cmd.exportHar);
}

async function savedRule(id: string) {
  const rule = (await backend.listRules()).find((r) => r.id === id);
  if (!rule) throw new Error(`no rule ${id}`);
  return normalizeRule(rule);
}

async function post(url: string, body: string) {
  await fetch(url, { method: "POST", body });
}

/** Right-clicks a row the way a user would (Radix opens on `contextmenu`). */
function openContextMenu(flow: number | null) {
  if (flow === null) {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    return;
  }
  const row = document.querySelector<HTMLElement>(`[data-flow-id="${flow}"]`);
  if (!row) return;
  const box = row.getBoundingClientRect();
  const at = { clientX: box.left + Math.min(320, box.width / 3), clientY: box.top + box.height / 2 };
  row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, ...at }));
}

export function runRemote() {
  const poll = async () => {
    try {
      const res = await fetch("/__pm_remote");
      if (res.status === 200) await apply((await res.json()) as Command);
    } catch (e) {
      // The dev server restarted, or a command failed; try again on the next tick.
      console.warn("pm remote:", e);
    }
    setTimeout(() => void poll(), 300);
  };
  void poll();
}
