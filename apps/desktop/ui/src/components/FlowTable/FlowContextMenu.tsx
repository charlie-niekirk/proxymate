import {
  ClipboardList,
  Copy,
  Eye,
  FileCode2,
  FileDown,
  FileInput,
  ListFilter,
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

import type { BodyPart } from "@/bindings";
import { FilterBySub } from "@/components/menus/filter";
import { CopyItem } from "@/components/menus/items";
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
import { backend } from "@/lib/backend";
import { shortcutLabel } from "@/lib/commands";
import { headersBlock } from "@/lib/copyFormats";
import { formatDuration, formatSize, formatTime, type Row } from "@/lib/flow";
import { bodyText, displayHeaderName, rawMessage } from "@/lib/http";
import { LIVE_STORE, useUi } from "@/store/ui";

import type { ColumnId } from "./columns";

/**
 * The flow row menu. Replay sends the request again as recorded; Edit &
 * Replay opens it in Compose. The presets pre-fill a rule from the flow (in
 * Rust) and open it, unsaved, in the Rules Editor; Rewrite… opens the
 * Rewrite dialog. They work on saved sessions and HARs too. The first item
 * copies the cell that was right-clicked; "Copy" has the flow's parts.
 */
export function FlowContextMenu({ row, column }: { row: Row | null; column: ColumnId | null }) {
  const store = useUi((s) => s.store);
  if (!row) return null;
  const connect = row.method === "CONNECT";
  const websocket = row.scheme === "ws" || row.scheme === "wss";
  const https = row.scheme === "https" || row.scheme === "wss";
  const live = store === LIVE_STORE;
  const sendable = !connect && !websocket;
  const cellCopy = column ? cellText(column, row) : null;
  return (
    <ContextMenuContent className="w-56" data-testid="flow-menu">
      {cellCopy && (
        <CopyItem text={cellCopy.text} what={cellCopy.label.toLowerCase()} testId="menu-copy-cell">
          <Copy />
          <span className="truncate">
            Copy {cellCopy.label} <span className="text-muted-foreground font-mono">{cellCopy.text}</span>
          </span>
        </CopyItem>
      )}
      <ContextMenuItem onSelect={() => void copyFlowUrl(store, row.id)} data-testid="menu-copy-url">
        <Copy />
        Copy URL
      </ContextMenuItem>
      <ContextMenuItem disabled={connect} onSelect={() => void copyFlowCurl(store, row.id)} data-testid="menu-curl">
        <Terminal />
        Copy as cURL
      </ContextMenuItem>
      <ContextMenuSub>
        <ContextMenuSubTrigger data-testid="menu-copy-more">
          <ClipboardList />
          Copy
        </ContextMenuSubTrigger>
        <ContextMenuSubContent className="w-52">
          <CopyItem text={row.host} what="host">
            Host
          </CopyItem>
          <CopyItem text={row.path} what="path">
            Path
          </CopyItem>
          <ContextMenuSeparator />
          <CopyItem text={() => headersOf(store, row.id, "request")} what="request headers" testId="menu-copy-req-headers">
            Request headers
          </CopyItem>
          <CopyItem text={() => headersOf(store, row.id, "response")} what="response headers" testId="menu-copy-resp-headers">
            Response headers
          </CopyItem>
          <CopyItem text={() => bodyOf(store, row.id, "request")} what="request body" disabled={connect}>
            Request body
          </CopyItem>
          <CopyItem text={() => bodyOf(store, row.id, "response")} what="response body" disabled={connect} testId="menu-copy-resp-body">
            Response body
          </CopyItem>
          <ContextMenuSeparator />
          <CopyItem text={() => rawOf(store, row.id, "request")} what="request" disabled={connect}>
            Raw request
          </CopyItem>
          <CopyItem text={() => rawOf(store, row.id, "response")} what="response" disabled={connect}>
            Raw response
          </CopyItem>
        </ContextMenuSubContent>
      </ContextMenuSub>
      <FilterBySub facts={{ host: row.host, method: row.method, status: row.status, client: row.client }} icon={<ListFilter />} />
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

/** The text of a flow table cell, as shown, for "Copy <column>". */
function cellText(column: ColumnId, row: Row): { label: string; text: string } | null {
  switch (column) {
    case "id":
      return { label: "Number", text: String(row.id + 1) };
    case "state":
      return { label: "State", text: row.state };
    case "client":
      return row.client ? { label: "App", text: row.client } : null;
    case "method":
      return { label: "Method", text: row.method };
    case "status":
      return row.status === null ? null : { label: "Status", text: String(row.status) };
    case "time":
      return row.startedAtMs === null ? null : { label: "Time", text: formatTime(row.startedAtMs) };
    case "duration":
      return row.durationMs === null ? null : { label: "Duration", text: formatDuration(row.durationMs) };
    case "req":
      return { label: "Request size", text: formatSize(row.reqSize) };
    case "resp":
      return { label: "Response size", text: formatSize(row.respSize) };
    case "proto":
      return { label: "Protocol", text: row.httpVersion };
    // The URL has its own item; the rest are icons.
    case "url":
    case "ssl":
    case "edited":
    case "tools":
      return null;
  }
}

async function detailOf(store: number, id: number) {
  const detail = await backend.getFlowDetail(store, id);
  if (!detail) throw new Error("This flow has no recorded messages");
  return detail;
}

/** A message's headers as `Name: value` lines. */
async function headersOf(store: number, id: number, part: BodyPart): Promise<string> {
  const detail = await detailOf(store, id);
  const message = part === "request" ? detail.request : detail.response;
  if (!message) throw new Error("No response yet");
  return headersBlock(message.headers.map((h) => [displayHeaderName(h.name), h.value]));
}

/** A body, decoded, as text: binary bodies explain themselves (the Body tab copies them as Base64). */
async function bodyOf(store: number, id: number, part: BodyPart): Promise<string> {
  const bytes = new Uint8Array(await backend.getBody(store, id, part, true));
  const text = bodyText(bytes);
  if (text === null) throw new Error(`The ${part} body isn't UTF-8 text: copy it as Base64 from the Body tab`);
  return text;
}

/** The message as the Raw tab rebuilds it (decoded body, or a note for a binary one). */
async function rawOf(store: number, id: number, part: BodyPart): Promise<string> {
  const detail = await detailOf(store, id);
  const bytes = new Uint8Array(await backend.getBody(store, id, part, true));
  const body = bytes.length === 0 ? null : (bodyText(bytes) ?? `[${formatSize(bytes.length)} binary body]`);
  if (part === "request") {
    const { method, url, version, headers } = detail.request;
    return rawMessage(`${method} ${url} ${version}`, headers, body);
  }
  const response = detail.response;
  if (!response) throw new Error("No response yet");
  return rawMessage(`${response.version} ${response.status} ${response.reason}`, response.headers, body);
}
