import { Copy, FileDown, FileInput, OctagonPause, PencilLine, Repeat2, Send, ShieldOff, Terminal } from "lucide-react";

import { ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import {
  addSslBypass,
  copyFlowCurl,
  copyFlowUrl,
  editAndReplay,
  exportHar,
  openPreset,
  replayFlow,
} from "@/lib/actions";
import type { Row } from "@/lib/flow";
import { LIVE_STORE, useUi } from "@/store/ui";

/**
 * The flow row menu. Replay sends the request again as recorded; Edit &
 * Replay opens it in Compose. The presets pre-fill a rule from the flow (in
 * Rust) and open it, unsaved, in the Rules Editor; they need a live flow.
 */
export function FlowContextMenu({ row }: { row: Row | null }) {
  const store = useUi((s) => s.store);
  if (!row) return null;
  const connect = row.method === "CONNECT";
  const websocket = row.scheme === "ws" || row.scheme === "wss";
  const https = row.scheme === "https" || row.scheme === "wss";
  const live = store === LIVE_STORE;
  const sendable = !connect && !websocket;
  return (
    <ContextMenuContent className="w-56" data-testid="flow-menu">
      <ContextMenuItem onSelect={() => void copyFlowUrl(store, row.id)}>
        <Copy />
        Copy URL
      </ContextMenuItem>
      <ContextMenuItem disabled={connect} onSelect={() => void copyFlowCurl(store, row.id)} data-testid="menu-curl">
        <Terminal />
        Copy as cURL
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={!sendable} onSelect={() => void replayFlow(store, row.id)} data-testid="menu-replay">
        <Repeat2 />
        Replay
      </ContextMenuItem>
      <ContextMenuItem disabled={!sendable} onSelect={() => void editAndReplay(store, row.id)} data-testid="menu-edit-replay">
        <Send />
        Edit &amp; Replay…
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        disabled={connect || !live}
        onSelect={() => void openPreset(row.id, "map_local")}
        data-testid="menu-map-local"
      >
        <FileInput />
        Map Local…
      </ContextMenuItem>
      <ContextMenuItem
        disabled={connect || !live}
        onSelect={() => void openPreset(row.id, "breakpoint")}
        data-testid="menu-breakpoint"
      >
        <OctagonPause />
        Breakpoint…
      </ContextMenuItem>
      <ContextMenuItem disabled={connect || !live} onSelect={() => void openPreset(row.id, "rewrite")} data-testid="menu-rewrite">
        <PencilLine />
        Rewrite…
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => void exportHar({ kind: "ids", ids: [row.id] })} data-testid="menu-export-har">
        <FileDown />
        Export HAR…
      </ContextMenuItem>
      <ContextMenuItem disabled={!https} onSelect={() => void addSslBypass(row.host)}>
        <ShieldOff />
        Add {row.host} to SSL bypass
      </ContextMenuItem>
    </ContextMenuContent>
  );
}
