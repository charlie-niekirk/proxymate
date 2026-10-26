import { createContext } from "react";

export interface BodyCopy {
  /** The decoded bytes, once loaded. */
  bytes: Uint8Array | null;
  /** "body", "payload", "data": names it in the items and toasts. */
  what: string;
  onSave?: () => void;
}

/**
 * The body a viewer shows, for the copy items every viewer's menu ends
 * with (`BodyItems`). Provided by the Body tab and the WebSocket/SSE
 * payload view.
 */
export const BodyCopyContext = createContext<BodyCopy | null>(null);
