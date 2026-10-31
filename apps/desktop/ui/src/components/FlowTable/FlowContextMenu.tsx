import {
  ClipboardList,
  Code2,
  Columns2,
  Copy,
  MessageSquareText,
  Repeat,
  Save,
  Tag,
  Trash2,
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

import type { BodyPart, FlowLabel } from "@/bindings";
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
  addFilterTerm,
  addSslBypass,
  compareSelection,
  copyFlowCode,
  copyFlowCurl,
  copyFlowUrl,
  copySelectionCurl,
  copySelectionUrls,
  currentSelection,
  deleteSelection,
  editAndReplay,
  exportHar,
  labelSelection,
  newScriptForFlow,
  openPreset,
  openRewrite,
  pinFlows,
  pinSelection,
  replayFlow,
  replaySelection,
  reportError,
  saveSession,
} from "@/lib/actions";
import { backend } from "@/lib/backend";
import { shortcutLabel } from "@/lib/commands";
import { headersBlock } from "@/lib/copyFormats";
import { CODE_LANGUAGES, FLOW_LABELS, formatDuration, formatSize, formatTime, type Row } from "@/lib/flow";
import { isSelected, selectionCount } from "@/lib/selection";
import { bodyText, displayHeaderName, rawMessage } from "@/lib/http";
import { useUi } from "@/store/ui";

import type { PlacedColumn } from "./columns";

/**
 * The flow row menu. Replay sends the request again as recorded; Edit &
 * Replay opens it in Compose. The presets pre-fill a rule from the flow (in
 * Rust) and open it, unsaved, in the Rules Editor; Rewrite… opens the
 * Rewrite dialog. They work on saved sessions and HARs too. The first item
 * copies the cell that was right-clicked; "Copy" has the flow's parts.
 */
export function FlowContextMenu({ row, column }: { row: Row | null; column: PlacedColumn | null }) {
  const store = useUi((s) => s.store);
  const mark = useUi((s) => s.compareMark);
  const selection = useUi((s) => s.selection);
  const len = useUi((s) => s.stats.len);
  if (!row) return null;
  const count = selectionCount(selection, len);
  if (count > 1 && isSelected(selection, row.id)) return <BulkMenu count={count} row={row} />;
  const connect = row.method === "CONNECT";
  const websocket = row.scheme === "ws" || row.scheme === "wss";
  const https = row.scheme === "https" || row.scheme === "wss";
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
        <ContextMenuSubTrigger disabled={connect || websocket} data-testid="menu-copy-code">
          <Code2 />
          Copy as Code
        </ContextMenuSubTrigger>
        <ContextMenuSubContent className="w-48">
          {CODE_LANGUAGES.map((l) => (
            <ContextMenuItem key={l.id} onSelect={() => void copyFlowCode(store, row.id, l.id)} data-testid={`menu-code-${l.id}`}>
              {l.label}
            </ContextMenuItem>
          ))}
        </ContextMenuSubContent>
      </ContextMenuSub>
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
      {column?.header && row.extra[column.header.index] != null && (
        <ContextMenuItem
          onSelect={() =>
            addFilterTerm(
              `${column.header!.part === "request" ? "reqheader" : "resheader"}:"${column.header!.name}: ${row.extra[column.header!.index]!.replace(/"/g, "")}"`,
            )
          }
          data-testid="menu-filter-header"
        >
          <ListFilter />
          <span className="truncate">Filter by {column.header.name}</span>
        </ContextMenuItem>
      )}
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
      <ContextMenuItem
        disabled={!sendable}
        onSelect={() => useUi.getState().openRepeat(store, row.id)}
        data-testid="menu-repeat"
      >
        <Repeat />
        Repeat…
        <ContextMenuShortcut>{shortcutLabel("r", { alt: true })}</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => void pinFlows([row.id], !row.pinned, store)} data-testid="menu-pin">
        {row.pinned ? <PinOff /> : <Pin />}
        {row.pinned ? "Unpin" : "Pin"}
      </ContextMenuItem>
      <LabelSub current={row.label} />
      <ContextMenuItem onSelect={() => useUi.getState().markForCompare({ store, id: row.id })} data-testid="menu-compare-mark">
        <Columns2 />
        Mark for Compare
      </ContextMenuItem>
      {mark && !(mark.store === store && mark.id === row.id) && (
        <ContextMenuItem
          onSelect={() => {
            useUi.getState().openCompare(mark, { store, id: row.id });
            useUi.getState().markForCompare(null);
          }}
          data-testid="menu-compare-with"
        >
          <Columns2 />
          Compare with Marked (#{mark.id + 1}
          {mark.store !== store ? ", other tab" : ""})
        </ContextMenuItem>
      )}
      <ContextMenuItem onSelect={() => useUi.getState().openComment(store, row.id)} data-testid="menu-comment">
        <MessageSquareText />
        {row.comment ? "Edit Comment…" : "Comment…"}
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
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        disabled={row.state === "active" || row.state === "paused"}
        onSelect={() => void deleteSelection({ kind: "ids", ids: [row.id] }, store).catch((e: unknown) => reportError("Delete failed", e))}
        data-testid="menu-delete"
      >
        <Trash2 />
        Delete
        <ContextMenuShortcut>⌫</ContextMenuShortcut>
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

/** "Label ▸": the six colours, and None. Applies to the selection. */
function LabelSub({ current }: { current: FlowLabel | null }) {
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger data-testid="menu-label">
        <Tag />
        Label
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="w-40">
        {FLOW_LABELS.map((label) => (
          <ContextMenuItem
            key={label}
            onSelect={() => void labelSelection(label).catch((e: unknown) => reportError("Label failed", e))}
            data-testid={`menu-label-${label}`}
          >
            <span className="size-2.5 rounded-full" style={{ background: `var(--label-${label})` }} />
            <span className="capitalize">{label}</span>
            {current === label && <span className="text-muted-foreground ml-auto">✓</span>}
          </ContextMenuItem>
        ))}
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={current === null}
          onSelect={() => void labelSelection(null).catch((e: unknown) => reportError("Label failed", e))}
          data-testid="menu-label-none"
        >
          No label
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

/** The menu for a right-click inside a multi-selection: bulk actions on all of it. */
function BulkMenu({ count, row }: { count: number; row: Row }) {
  const n = count.toLocaleString();
  const run = (what: string, action: () => Promise<unknown>) => () => void action().catch((e: unknown) => reportError(what, e));
  return (
    <ContextMenuContent className="w-56" data-testid="bulk-menu">
      <div className="text-muted-foreground px-2 py-1 text-xs">{n} flows selected</div>
      <ContextMenuItem onSelect={run("Copy failed", () => copySelectionUrls())} data-testid="bulk-copy-urls">
        <Copy />
        Copy URLs
      </ContextMenuItem>
      <ContextMenuItem onSelect={run("Copy failed", () => copySelectionCurl())} data-testid="bulk-copy-curl">
        <Terminal />
        Copy as cURL
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={run("Replay failed", () => replaySelection())} data-testid="bulk-replay">
        <Repeat2 />
        Replay in Order
      </ContextMenuItem>
      <ContextMenuItem disabled={count !== 2} onSelect={run("Compare failed", () => compareSelection())} data-testid="bulk-compare">
        <Columns2 />
        Compare
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={run("Pin failed", () => pinSelection(true))} data-testid="bulk-pin">
        <Pin />
        Pin
      </ContextMenuItem>
      <ContextMenuItem onSelect={run("Unpin failed", () => pinSelection(false))} data-testid="bulk-unpin">
        <PinOff />
        Unpin
      </ContextMenuItem>
      <LabelSub current={null} />
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={run("Export failed", () => exportHar(currentSelection()))} data-testid="bulk-export-har">
        <FileDown />
        Export HAR…
      </ContextMenuItem>
      <ContextMenuItem onSelect={run("Save failed", () => saveSession(currentSelection()))} data-testid="bulk-save-session">
        <Save />
        Save Session…
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem variant="destructive" onSelect={run("Delete failed", () => deleteSelection())} data-testid="bulk-delete">
        <Trash2 />
        Delete {n} Flows
        <ContextMenuShortcut>⌫</ContextMenuShortcut>
      </ContextMenuItem>
      <span hidden data-row={row.id} />
    </ContextMenuContent>
  );
}

/** The text of a flow table cell, as shown, for "Copy <column>". */
function cellText(col: PlacedColumn, row: Row): { label: string; text: string } | null {
  if (col.header) {
    const value = row.extra[col.header.index];
    return value == null ? null : { label: col.header.name, text: value };
  }
  switch (col.id) {
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
    case "notes":
      return row.comment ? { label: "Comment", text: row.comment } : null;
    default:
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
