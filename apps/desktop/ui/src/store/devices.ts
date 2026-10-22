import { create } from "zustand";

import type { DevicesSnapshot_Deserialize } from "@/bindings";
import { backend } from "@/lib/backend";

interface DevicesState {
  snapshot: DevicesSnapshot_Deserialize;
  /** Pending devices whose prompt was closed unanswered: the toolbar offers it again. */
  dismissed: ReadonlySet<string>;
  dismiss(ip: string): void;
  /** Shows the prompts again. */
  reopen(): void;
}

const idle = { flows: 0, https_decrypted: 0, https_refused: 0 };

/**
 * LAN clients (allow-list and pending prompts) and Simulator/emulator
 * activity, as the engine last pushed them.
 */
export const useDevices = create<DevicesState>()((set) => ({
  snapshot: { devices: [], simulator: idle, emulator: idle },
  dismissed: new Set(),
  dismiss: (ip) => set((s) => ({ dismissed: new Set([...s.dismissed, ip]) })),
  reopen: () => set({ dismissed: new Set() }),
}));

let started = false;

/** Subscribes to the engine's device pushes, once per page. */
export function startDevices() {
  if (started) return;
  started = true;
  void backend.subscribeDevices((snapshot) => useDevices.setState({ snapshot }));
}
