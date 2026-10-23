import { create } from "zustand";

import type { McpStatus_Serialize } from "@/bindings";
import { backend } from "@/lib/backend";
import { queryClient, rulesKey, scriptsKey } from "@/lib/query";

interface McpState {
  /** As the engine last pushed it; null until the first push. */
  status: McpStatus_Serialize | null;
}

export const useMcp = create<McpState>()(() => ({ status: null }));

let started = false;

/**
 * Subscribes to the MCP status (clients, settings) and to the rule and
 * script lists, once per page. Pushes land in the rules and scripts
 * queries, so an open editor and the throttle menu follow edits an agent
 * makes.
 */
export function startMcp() {
  if (started) return;
  started = true;
  void backend.subscribeMcp((status) => useMcp.setState({ status }));
  void backend.subscribeRules((rules) => queryClient.setQueryData(rulesKey, rules));
  void backend.subscribeScripts((scripts) => queryClient.setQueryData(scriptsKey, scripts));
}
