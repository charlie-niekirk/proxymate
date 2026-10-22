import { create } from "zustand";

import type { StorageStatus_Serialize } from "@/bindings";
import { backend } from "@/lib/backend";

/** The live capture's flows, disk use and write errors, as the engine pushes them. */
export const useStorage = create<{ status: StorageStatus_Serialize | null }>()(() => ({ status: null }));

let started = false;

/** Subscribes to the engine's storage status, once per page. */
export function startStorage() {
  if (started) return;
  started = true;
  void backend.subscribeStorage((status) => useStorage.setState({ status }));
}
