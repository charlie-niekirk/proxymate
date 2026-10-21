import { create } from "zustand";

import type { DevicesSnapshot_Deserialize } from "@/bindings";
import { backend } from "@/lib/backend";

interface DevicesState {
  snapshot: DevicesSnapshot_Deserialize;
}

const idle = { flows: 0, https_decrypted: 0, https_refused: 0 };

/**
 * LAN clients (allow-list and pending prompts) and Simulator/emulator
 * activity, as the engine last pushed them.
 */
export const useDevices = create<DevicesState>()(() => ({
  snapshot: { devices: [], simulator: idle, emulator: idle },
}));

let started = false;

/** Subscribes to the engine's device pushes, once per page. */
export function startDevices() {
  if (started) return;
  started = true;
  void backend.subscribeDevices((snapshot) => useDevices.setState({ snapshot }));
}
