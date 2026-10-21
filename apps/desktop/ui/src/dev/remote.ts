import type { ContentKind, MessageEdits_Deserialize, PresetKind } from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import { bridge } from "@/dev/bridge";
import { openPreset, setThrottle } from "@/lib/actions";
import { THROTTLE_PRESETS } from "@/lib/rules";
import { useUi, type RequestTab, type ResponseTab } from "@/store/ui";

/**
 * Dev only (`VITE_PM_REMOTE=1`): polls the Vite server's `/__pm_remote`
 * mailbox and applies each command, so the real app can be driven and
 * screenshotted from a shell, where synthetic input is unavailable.
 *
 * `curl -d '<json>' localhost:<vite port>/__pm_remote`, one command object
 * per POST (fields apply in the order below):
 * - `{"select": 3}`, `{"requestTab": "Body"}`, `{"responseTab": "TLS"}`,
 *   `{"viewer": {"kind": "json", "viewer": "tree"}}`, `{"theme": "light"}`
 * - `{"contextMenu": 3}`: right-clicks flow 3's row (it must be on screen),
 *   opening the flow menu for a screenshot. `{"contextMenu": null}` closes it.
 * - `{"preset": {"flow": 3, "kind": "map_local"}}` (or `breakpoint`,
 *   `rewrite`): exactly what the menu's "Map Local…" etc. do.
 * - `{"rulesEditor": true}` / `false`: opens or closes the Rules Editor.
 * - `{"testRule": true}`: presses "Test against selected flow".
 * - `{"saveRule": true}`: presses Save in the open Rules Editor.
 * - `{"throttle": "3G"}` (a preset name from `lib/rules.ts`, or "Off").
 * - `{"breakpoints": true}` / `false`: opens or closes the Breakpoint Editor.
 * - `{"breakpointAction": {"execute": {"url": "…", "headers": [...], "body": "…"}}}`
 *   presses Execute on the selected paused flow, merging these edits over
 *   any made in the form (`{"execute": {}}` sends the form as is);
 *   `{"breakpointAction": {"abort": true}}` presses Abort.
 */
interface Command {
  select?: number;
  requestTab?: RequestTab;
  responseTab?: ResponseTab;
  viewer?: { kind: ContentKind; viewer: ViewerId };
  theme?: "dark" | "light";
  contextMenu?: number | null;
  preset?: { flow: number; kind: PresetKind };
  rulesEditor?: boolean;
  testRule?: boolean;
  saveRule?: boolean;
  throttle?: string;
  breakpoints?: boolean;
  breakpointAction?: { execute?: MessageEdits_Deserialize; abort?: boolean };
}

async function apply(cmd: Command) {
  const ui = useUi.getState();
  if (cmd.select !== undefined) ui.select(cmd.select);
  if (cmd.requestTab) ui.setRequestTab(cmd.requestTab);
  if (cmd.responseTab) ui.setResponseTab(cmd.responseTab);
  if (cmd.viewer) ui.setViewer(cmd.viewer.kind, cmd.viewer.viewer);
  if (cmd.theme) {
    document.documentElement.dataset.theme = cmd.theme;
    document.documentElement.classList.toggle("dark", cmd.theme === "dark");
  }
  if (cmd.contextMenu !== undefined) openContextMenu(cmd.contextMenu);
  if (cmd.preset) await openPreset(cmd.preset.flow, cmd.preset.kind);
  if (cmd.rulesEditor === true) ui.openRulesEditor();
  if (cmd.rulesEditor === false) ui.closeRulesEditor();
  if (cmd.testRule) bridge.testRule?.();
  if (cmd.saveRule) await bridge.saveRule?.();
  if (cmd.throttle !== undefined) await setThrottle(THROTTLE_PRESETS.find((p) => p.name === cmd.throttle) ?? null);
  if (cmd.breakpoints !== undefined) ui.setBreakpointsOpen(cmd.breakpoints);
  if (cmd.breakpointAction?.execute) await bridge.executeBreakpoint?.(cmd.breakpointAction.execute);
  if (cmd.breakpointAction?.abort) await bridge.abortBreakpoint?.();
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
