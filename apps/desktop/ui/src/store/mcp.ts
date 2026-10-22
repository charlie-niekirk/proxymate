import { create } from "zustand";

import type { McpStatus_Serialize } from "@/bindings";
import { backend } from "@/lib/backend";
import { queryClient, rulesKey } from "@/lib/query";

interface McpState {
  /** As the engine last pushed it; null until the first push. */
  status: McpStatus_Serialize | null;
}

export const useMcp = create<McpState>()(() => ({ status: null }));

let started = false;

/**
 * Subscribes to the MCP status (clients, settings) and to the rule list,
 * once per page. Rule pushes land in the rules query, so an open Rules
 * Editor and the throttle menu follow edits an agent makes.
 */
export function startMcp() {
  if (started) return;
  started = true;
  void backend.subscribeMcp((status) => useMcp.setState({ status }));
  void backend.subscribeRules((rules) => queryClient.setQueryData(rulesKey, rules));
}
