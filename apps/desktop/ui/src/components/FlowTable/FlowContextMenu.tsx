import { Copy, FileInput, OctagonPause, PencilLine, ShieldOff, Terminal } from "lucide-react";

import { ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import { addSslBypass, copyFlowCurl, copyFlowUrl, openPreset } from "@/lib/actions";
import type { Row } from "@/lib/flow";

/**
 * The flow row menu. The presets pre-fill a rule from the flow (in Rust)
 * and open it, unsaved, in the Rules Editor.
 */
export function FlowContextMenu({ row }: { row: Row | null }) {
  if (!row) return null;
  const connect = row.method === "CONNECT";
  const https = row.scheme === "https" || row.scheme === "wss";
  return (
    <ContextMenuContent className="w-56" data-testid="flow-menu">
      <ContextMenuItem onSelect={() => void copyFlowUrl(row.id)}>
        <Copy />
        Copy URL
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => void copyFlowCurl(row.id)} title="Without the body; the full exporter arrives in M8">
        <Terminal />
        Copy as cURL (basic)
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={connect} onSelect={() => void openPreset(row.id, "map_local")} data-testid="menu-map-local">
        <FileInput />
        Map Local…
      </ContextMenuItem>
      <ContextMenuItem disabled={connect} onSelect={() => void openPreset(row.id, "breakpoint")} data-testid="menu-breakpoint">
        <OctagonPause />
        Breakpoint…
      </ContextMenuItem>
      <ContextMenuItem disabled={connect} onSelect={() => void openPreset(row.id, "rewrite")} data-testid="menu-rewrite">
        <PencilLine />
        Rewrite…
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={!https} onSelect={() => void addSslBypass(row.host)}>
        <ShieldOff />
        Add {row.host} to SSL bypass
      </ContextMenuItem>
    </ContextMenuContent>
  );
}
