import { useEffect } from "react";

import type { MessageEdits_Deserialize, Phase } from "@/bindings";
import type { PointerEdit } from "@/components/Rewrite/pointer";

/**
 * Handles the open editors register so `dev/remote.ts` can press their
 * buttons from a shell (the real app takes no synthetic input). Each runs
 * exactly what the button does. Unused outside dev.
 */
export interface Bridge {
  saveRule?: () => Promise<void>;
  testRule?: () => void;
  executeBreakpoint?: (edits: MessageEdits_Deserialize | null) => Promise<void>;
  abortBreakpoint?: () => Promise<void>;
  sendCompose?: () => Promise<void>;
  /** The open Rewrite dialog. `null` values remove a header, parameter or field. */
  rewrite?: {
    side: (side: Phase) => void;
    tab: (side: Phase, tab: "Headers" | "Query" | "Body" | "Status") => void;
    edit: (side: Phase, op: PointerEdit) => void;
    whole: (side: Phase, on: boolean) => void;
    header: (side: Phase, header: { name: string; value: string | null }) => void;
    query: (param: { name: string; value: string | null }) => void;
    form: (side: Phase, field: { name: string; value: string | null }) => void;
    status: (status: number) => void;
    panel: (panel: "preview" | "actions") => void;
    save: () => Promise<void>;
  };
  saveScript?: () => Promise<void>;
  testScript?: (part?: "request" | "response") => void;
}

export const bridge: Bridge = {};

/**
 * The one way an editor registers its handles (R4): while it is mounted,
 * re-registered on every render so each handle calls the latest closure, and
 * removed when it unmounts. Release builds register nothing.
 */
export function useBridge(handles: Bridge) {
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    Object.assign(bridge, handles);
    return () => {
      for (const name of Object.keys(handles) as (keyof Bridge)[]) {
        if (bridge[name] === handles[name]) delete bridge[name];
      }
    };
  });
}
