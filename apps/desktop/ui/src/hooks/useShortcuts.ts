import { useEffect } from "react";

import { focusFlowTable } from "@/lib/focus";
import { commandForKey, isEditable } from "@/lib/commands";
import { useUi } from "@/store/ui";

/**
 * The app's keyboard shortcuts (see `lib/commands.ts`): ⌘K opens the
 * command palette, the rest run their command. Escape in the inspector
 * goes back to the flow table.
 */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        useUi.getState().setPaletteOpen(!useUi.getState().paletteOpen);
        return;
      }
      if (e.key === "Escape" && !e.defaultPrevented && !isEditable(e.target)) {
        const target = e.target instanceof HTMLElement ? e.target : null;
        if (target?.closest('[data-testid="inspector"]') && !document.querySelector('[role="dialog"]')) {
          e.preventDefault();
          focusFlowTable();
        }
        return;
      }
      const command = commandForKey(e);
      if (!command) return;
      e.preventDefault();
      void command.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
