import { create } from "zustand";

import type { SidebarSnapshot_Deserialize } from "@/bindings";
import { backend } from "@/lib/backend";

interface SidebarState {
  snapshot: SidebarSnapshot_Deserialize;
  /** Names of apps `pmicon://` has an icon for. */
  icons: ReadonlySet<string>;
}

/**
 * The sidebar's Apps and Domains as the engine last pushed them (small: one
 * entry per app and host, not per flow).
 */
export const useSidebar = create<SidebarState>()(() => ({
  snapshot: { total: 0, apps: [], domains: [] },
  icons: new Set(),
}));

let started = false;

/** Subscribes to the engine's sidebar pushes, once per page. */
export function startSidebar() {
  if (started) return;
  started = true;
  void backend.subscribeSidebar((snapshot) => {
    const icons = new Set(snapshot.apps.filter((a) => a.has_icon).map((a) => a.name));
    useSidebar.setState({ snapshot, icons });
  });
}
