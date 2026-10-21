import { create } from "zustand";

import type { BreakpointInfo_Serialize } from "@/bindings";
import { backend } from "@/lib/backend";
import { useUi } from "@/store/ui";

interface BreakpointState {
  /** Paused flows, oldest first, as the engine last pushed them. */
  queue: BreakpointInfo_Serialize[];
}

export const useBreakpoints = create<BreakpointState>()(() => ({ queue: [] }));

let started = false;

/**
 * Subscribes to the engine's breakpoint queue, once per page. The editor
 * opens itself when the queue goes from empty to not (and closes once it
 * is empty again), and the selection
 * follows the oldest paused flow when the selected one is resolved.
 */
export function startBreakpoints() {
  if (started) return;
  started = true;
  void backend.subscribeBreakpoints((queue) => {
    const before = useBreakpoints.getState().queue;
    useBreakpoints.setState({ queue });
    const ui = useUi.getState();
    if (before.length === 0 && queue.length > 0) ui.setBreakpointsOpen(true);
    if (before.length > 0 && queue.length === 0) ui.setBreakpointsOpen(false);
    if (!queue.some((b) => b.id === ui.selectedBreakpoint)) ui.selectBreakpoint(queue[0]?.id ?? null);
  });
}
