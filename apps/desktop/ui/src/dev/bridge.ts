import type { MessageEdits_Deserialize } from "@/bindings";

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
} = {};
