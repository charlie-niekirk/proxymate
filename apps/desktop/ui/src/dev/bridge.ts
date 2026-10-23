import type { MessageEdits_Deserialize, Phase } from "@/bindings";
import type { PointerEdit } from "@/components/Rewrite/RewriteDialog";

/**
 * Handles the open editors register so `dev/remote.ts` can press their
 * buttons from a shell (the real app takes no synthetic input). Each runs
 * exactly what the button does. Unused outside dev.
 */
export const bridge: {
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
} = {};
