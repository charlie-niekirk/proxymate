import {
  Copy,
  Eye,
  FileCode2,
  FileDown,
  FileInput,
  OctagonPause,
  PencilLine,
  Pin,
  PinOff,
  Repeat2,
  Send,
  ShieldOff,
  Terminal,
  Theater,
} from "lucide-react";

import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import {
  addSslBypass,
  copyFlowCurl,
  copyFlowUrl,
  editAndReplay,
  exportHar,
  newScriptForFlow,
  openPreset,
  openRewrite,
  pinFlows,
  replayFlow,
} from "@/lib/actions";
import { shortcutLabel } from "@/lib/commands";
import type { Row } from "@/lib/flow";
import { LIVE_STORE, useUi } from "@/store/ui";

/**
 * The flow row menu. Replay sends the request again as recorded; Edit &
 * Replay opens it in Compose. The presets pre-fill a rule from the flow (in
 * Rust) and open it, unsaved, in the Rules Editor; Rewrite… opens the
 * Rewrite dialog. They work on saved sessions and HARs too.
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
        <ContextMenuShortcut>{shortcutLabel("r")}</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuItem disabled={!sendable} onSelect={() => void editAndReplay(store, row.id)} data-testid="menu-edit-replay">
        <Send />
        Edit &amp; Replay…
      </ContextMenuItem>
      <ContextMenuItem disabled={!live} onSelect={() => void pinFlows([row.id], !row.pinned)} data-testid="menu-pin">
        {row.pinned ? <PinOff /> : <Pin />}
        {row.pinned ? "Unpin" : "Pin"}
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        disabled={!sendable}
        onSelect={() => void openPreset(store, row.id, "map_local")}
        data-testid="menu-map-local"
      >
        <FileInput />
        Map Local…
      </ContextMenuItem>
      <ContextMenuItem
        disabled={!sendable}
        onSelect={() => void openPreset(store, row.id, "breakpoint")}
        data-testid="menu-breakpoint"
      >
        <OctagonPause />
        Breakpoint…
      </ContextMenuItem>
      <ContextMenuItem disabled={!sendable} onSelect={() => void openRewrite(store, row.id)} data-testid="menu-rewrite">
        <PencilLine />
        Rewrite…
      </ContextMenuItem>
      <ContextMenuSub>
        <ContextMenuSubTrigger disabled={connect} data-testid="menu-script">
          <FileCode2 />
          Script
        </ContextMenuSubTrigger>
        <ContextMenuSubContent className="w-56">
          <ContextMenuItem onSelect={() => void newScriptForFlow(store, row.id, "view")} data-testid="menu-script-view">
            <Eye />
            New view script…
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => void newScriptForFlow(store, row.id, "rewrite")} data-testid="menu-script-rewrite">
            <PencilLine />
            New rewrite script…
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => void newScriptForFlow(store, row.id, "mock")} data-testid="menu-script-mock">
            <Theater />
            New mock script…
          </ContextMenuItem>
        </ContextMenuSubContent>
      </ContextMenuSub>
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
