import type { ContentKind } from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import { useUi, type RequestTab, type ResponseTab } from "@/store/ui";

/**
 * Dev only (`VITE_PM_REMOTE=1`): polls the Vite server's `/__pm_remote`
 * mailbox and applies each command, so the real app can be driven and
 * screenshotted from a shell, where synthetic input is unavailable.
 */
interface Command {
  select?: number;
  requestTab?: RequestTab;
  responseTab?: ResponseTab;
  viewer?: { kind: ContentKind; viewer: ViewerId };
  theme?: "dark" | "light";
}

function apply(cmd: Command) {
  const ui = useUi.getState();
  if (cmd.select !== undefined) ui.select(cmd.select);
  if (cmd.requestTab) ui.setRequestTab(cmd.requestTab);
  if (cmd.responseTab) ui.setResponseTab(cmd.responseTab);
  if (cmd.viewer) ui.setViewer(cmd.viewer.kind, cmd.viewer.viewer);
  if (cmd.theme) {
    document.documentElement.dataset.theme = cmd.theme;
    document.documentElement.classList.toggle("dark", cmd.theme === "dark");
  }
}

export function runRemote() {
  const poll = async () => {
    try {
      const res = await fetch("/__pm_remote");
      if (res.status === 200) apply((await res.json()) as Command);
    } catch {
      // The dev server restarted; try again on the next tick.
    }
    setTimeout(() => void poll(), 300);
  };
  void poll();
}
