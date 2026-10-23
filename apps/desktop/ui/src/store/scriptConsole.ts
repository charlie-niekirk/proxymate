import { create } from "zustand";

import { backend } from "@/lib/backend";

interface ScriptConsoleState {
  /** The newest console entry's sequence number, as the engine last pushed it. */
  latest: number;
}

export const useScriptConsole = create<ScriptConsoleState>()(() => ({ latest: 0 }));

let started = false;

/** Follows the script console, once per page; entries are fetched on demand. */
export function startScriptConsole() {
  if (started) return;
  started = true;
  void backend.subscribeScriptConsole((latest) => useScriptConsole.setState({ latest }));
}
