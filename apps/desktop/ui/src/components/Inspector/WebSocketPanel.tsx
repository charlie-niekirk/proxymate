import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, RotateCcw, Send, Unplug } from "lucide-react";
import { useState, type KeyboardEvent } from "react";

import type { WsDirection, WsMessage_Serialize, WsQuery_Deserialize } from "@/bindings";
import { CopyItem } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { usePagedList } from "@/hooks/usePagedList";
import { reportError } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatSize, formatTime } from "@/lib/flow";
import { queryClient, streamKey } from "@/lib/query";
import { LIVE_STORE, useUi } from "@/store/ui";
import {
  OPCODE_LABEL,
  canResend,
  closeCodeLabel,
  describeClose,
  messageFlags,
  resendOf,
  wsSendOf,
  type ComposeMode,
} from "@/lib/stream";
import { base64, hexBytes } from "@/lib/copyFormats";
import { bodyText } from "@/lib/http";

import { Placeholder } from "./KvTable";
import { PayloadView } from "./PayloadView";
import { LiveStatus, MessageList, SearchBox, StreamShell } from "./StreamShell";
import { listHeaderClass, rowClass } from "./streamStyles";

const GRID = "grid-cols-[14px_40px_84px_48px_60px_minmax(0,1fr)]";

type DirectionFilter = "all" | WsDirection;

/**
 * The frames of a WebSocket flow, live while it is open: a virtualised
 * list with a direction filter and payload search (both run in Rust), and
 * the selected message's payload. While a live connection is open, the
 * composer sends messages either way and Resend repeats a recorded one
 * (M19); rows show when a rule edited or dropped a message, or Proxymate
 * sent it.
 */
export function WebSocketPanel({ id }: { id: number }) {
  const [direction, setDirection] = useState<DirectionFilter>("all");
  const [text, setText] = useState("");
  const [selected, setSelected] = useState<WsMessage_Serialize | null>(null);
  const query: WsQuery_Deserialize = { direction: direction === "all" ? null : direction, text };
  const store = useUi((st) => st.store);

  const { data: head } = useQuery({
    queryKey: [...streamKey(store, id), "ws", query],
    queryFn: () => backend.getWebsocketMessages(store, id, query, 0, 0),
    placeholderData: keepPreviousData,
  });
  const len = head?.len ?? 0;
  const [[first, last], setRange] = useState<[number, number]>([0, -1]);
  const message = usePagedList(["ws-page", store, id, query], len, first, last, async (start, count) =>
    (await backend.getWebsocketMessages(store, id, query, start, count)).messages,
  );

  // With Auto Select on, the newest message is inspected until one is clicked.
  const autoSelect = useUi((st) => st.autoScroll);
  const shown = selected ?? (autoSelect && len > 0 ? (message(len - 1) ?? null) : null);
  const total = head?.total ?? 0;
  const count = len === total ? `${total} messages` : `${len} of ${total} messages`;
  const status = head?.open ? "Open" : head?.close ? describeClose(head.close) : "Closed";
  // Only live connections take messages.
  const sendable = store === LIVE_STORE && (head?.open ?? false);

  return (
    <StreamShell
      title="WebSocket"
      testId="websocket-panel"
      status={
        <>
          <LiveStatus live={head?.open ?? false} label={status} />
          <span className="shrink-0 tabular-nums" data-testid="message-count">
            · {count}
          </span>
        </>
      }
      controls={
        <>
          <ToggleGroup
            type="single"
            value={direction}
            onValueChange={(v) => v && setDirection(v as DirectionFilter)}
            className="gap-0.5"
          >
            <ToggleGroupItem value="all" className={chip}>
              All
            </ToggleGroupItem>
            <ToggleGroupItem value="sent" className={chip} aria-label="Sent by the client">
              <ArrowUp className="size-3 text-[var(--ws-sent)]" />
              Sent
            </ToggleGroupItem>
            <ToggleGroupItem value="received" className={chip} aria-label="Received from the server">
              <ArrowDown className="size-3 text-[var(--ws-received)]" />
              Received
            </ToggleGroupItem>
          </ToggleGroup>
          <SearchBox value={text} onChange={setText} placeholder="Search payloads" />
          {sendable && (
            <Button
              variant="ghost"
              size="xs"
              title="Close the connection: a normal close frame to each side"
              onClick={() => void backend.wsClose(id).then(() => refresh(store, id), (e: unknown) => reportError("Couldn't close the connection", e))}
              data-testid="ws-close"
            >
              <Unplug />
              Close
            </Button>
          )}
        </>
      }
      list={
        <div className="flex h-full flex-col">
          <div className="min-h-0 flex-1">
            <MessageList
          count={len}
          onRange={setRange}
          menuTestId="ws-menu"
          menu={(index) => {
            const m = message(index);
            return m ? <MessageItems id={id} message={m} sendable={sendable} /> : null;
          }}
          emptyText={total === 0 ? "No messages yet" : "No messages match"}
          header={
            <div className={`${listHeaderClass} ${GRID}`}>
              <span />
              <span className="text-right">#</span>
              <span>Time</span>
              <span>Type</span>
              <span className="text-right">Size</span>
              <span>Payload</span>
            </div>
          }
          row={(index) => {
            const m = message(index);
            if (!m) return null;
            return (
              <div
                className={`${rowClass(shown?.index === m.index, index)} ${GRID}`}
                onClick={() => setSelected(m)}
                data-testid="ws-row"
              >
                <DirectionArrow direction={m.direction} />
                <span className="text-muted-foreground text-right tabular-nums">{m.index + 1}</span>
                <span className="tabular-nums">{formatTime(m.time_ms)}</span>
                <span>{OPCODE_LABEL[m.opcode]}</span>
                <span className="text-right tabular-nums">{formatSize(m.len) === "—" ? "0 B" : formatSize(m.len)}</span>
                <span className={`flex min-w-0 items-center gap-1 ${m.dropped ? "text-muted-foreground line-through" : ""}`}>
                  <Flags message={m} />
                  <span className="truncate">{rowPreview(m)}</span>
                </span>
              </div>
            );
          }}
            />
          </div>
          {sendable && <Composer id={id} />}
        </div>
      }
      detail={shown ? <MessageDetail id={id} message={shown} sendable={sendable} /> : <Placeholder text="Select a message" />}
    />
  );
}

/** Refetches the list after a send or close, ahead of the next delta. */
function refresh(store: number, id: number) {
  void queryClient.invalidateQueries({ queryKey: streamKey(store, id) });
  void queryClient.invalidateQueries({ queryKey: ["ws-page", store, id] });
}

async function resend(id: number, m: WsMessage_Serialize) {
  const store = useUi.getState().store;
  try {
    const payload = new Uint8Array(await backend.getWebsocketPayload(store, id, m.index));
    await backend.wsSend(id, resendOf(m, payload));
    refresh(store, id);
  } catch (e) {
    reportError("Couldn't resend the message", e);
  }
}

const FLAG_STYLE: Record<ReturnType<typeof messageFlags>[number], { color: string; title: string }> = {
  Injected: { color: "var(--diff-add)", title: "Sent by Proxymate (the composer, Resend or an agent)" },
  Edited: { color: "var(--diff-change)", title: "A rule, script or breakpoint changed it; the payload is what was forwarded" },
  Dropped: { color: "var(--diff-del)", title: "A rule, script or breakpoint dropped it: it was never forwarded" },
};

function Flags({ message }: { message: WsMessage_Serialize }) {
  return messageFlags(message).map((flag) => (
    <span
      key={flag}
      title={FLAG_STYLE[flag].title}
      className="shrink-0 rounded-sm px-1 text-2xs leading-4 font-semibold no-underline"
      style={{ color: FLAG_STYLE[flag].color, backgroundColor: `color-mix(in oklab, ${FLAG_STYLE[flag].color} 14%, transparent)` }}
      data-testid={`ws-flag-${flag.toLowerCase()}`}
    >
      {flag}
    </span>
  ));
}

/**
 * Sends a message into the open connection: to the server as if the
 * client sent it, or to the client as if the server did. Text goes as a
 * text message, hex as binary. ⌘↩ sends.
 */
function Composer({ id }: { id: number }) {
  const store = useUi((st) => st.store);
  const [direction, setDirection] = useState<WsDirection>("sent");
  const [mode, setMode] = useState<ComposeMode>("text");
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const send = async () => {
    const message = wsSendOf(direction, mode, input);
    if (typeof message === "string") return setError(message);
    setError(null);
    try {
      await backend.wsSend(id, message);
      refresh(store, id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void send();
    }
  };
  return (
    <div className="bg-pane-toolbar flex shrink-0 flex-col gap-1 border-t p-1.5" data-testid="ws-composer">
      <div className="flex items-center gap-1.5">
        <ToggleGroup type="single" value={direction} onValueChange={(v) => v && setDirection(v as WsDirection)} className="gap-0.5">
          <ToggleGroupItem value="sent" className={chip} aria-label="To the server" data-testid="ws-compose-to-server">
            <ArrowUp className="size-3 text-[var(--ws-sent)]" />
            To server
          </ToggleGroupItem>
          <ToggleGroupItem value="received" className={chip} aria-label="To the client" data-testid="ws-compose-to-client">
            <ArrowDown className="size-3 text-[var(--ws-received)]" />
            To client
          </ToggleGroupItem>
        </ToggleGroup>
        <ToggleGroup type="single" value={mode} onValueChange={(v) => v && setMode(v as ComposeMode)} className="gap-0.5">
          <ToggleGroupItem value="text" className={chip}>
            Text
          </ToggleGroupItem>
          <ToggleGroupItem value="hex" className={chip} data-testid="ws-compose-hex">
            Binary (hex)
          </ToggleGroupItem>
        </ToggleGroup>
        <span className="min-w-0 flex-1 truncate text-right text-[11px] text-[var(--s-failed)]" data-testid="ws-compose-error">
          {error}
        </span>
        <Button size="xs" onClick={() => void send()} title="Send (⌘↩)" data-testid="ws-compose-send">
          <Send />
          Send
        </Button>
      </div>
      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={onKeyDown}
        rows={2}
        spellCheck={false}
        placeholder={mode === "text" ? "Message text, e.g. {\"type\":\"ping\"}" : "Bytes in hex, e.g. 0a ff 3c"}
        className="bg-background focus-visible:ring-ring/50 min-h-10 w-full resize-y rounded-sm border px-2 py-1 font-mono text-xs outline-none focus-visible:ring-2"
        data-testid="ws-compose-input"
      />
    </div>
  );
}

/** A message row's copies; payloads are fetched whole (the row only has a preview). */
function MessageItems({ id, message: m, sendable }: { id: number; message: WsMessage_Serialize; sendable: boolean }) {
  const store = useUi((st) => st.store);
  const payload = async () => new Uint8Array(await backend.getWebsocketPayload(store, id, m.index));
  const text = async () => {
    const bytes = await payload();
    const t = bodyText(bytes);
    if (t === null) throw new Error("The payload isn't UTF-8 text: copy it as Base64 or hex instead");
    return t;
  };
  return (
    <>
      {m.opcode === "close" ? (
        <CopyItem text={[m.close_code === null ? "" : closeCodeLabel(m.close_code), m.preview].filter(Boolean).join(" · ")} what="close reason">
          Copy close code and reason
        </CopyItem>
      ) : (
        <CopyItem text={text} what="payload" disabled={m.opcode === "binary" || m.len === 0} testId="ws-copy-payload">
          Copy payload
        </CopyItem>
      )}
      <CopyItem text={async () => base64(await payload())} what="payload as Base64" disabled={m.len === 0} testId="ws-copy-base64">
        Copy payload as Base64
      </CopyItem>
      <CopyItem text={async () => hexBytes(await payload())} what="payload as hex" disabled={m.len === 0}>
        Copy payload as hex
      </CopyItem>
      {sendable && canResend(m) && (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => void resend(id, m)} data-testid="ws-resend">
            Resend {m.direction === "sent" ? "to the server" : "to the client"}
          </ContextMenuItem>
        </>
      )}
    </>
  );
}

function rowPreview(m: WsMessage_Serialize): string {
  if (m.opcode === "close") return [m.close_code === null ? "" : closeCodeLabel(m.close_code), m.preview].filter(Boolean).join(" · ");
  if (m.opcode === "binary") return `Binary data · ${m.len} bytes`;
  return m.preview;
}

function DirectionArrow({ direction }: { direction: WsDirection }) {
  return direction === "sent" ? (
    <ArrowUp className="size-3 text-[var(--ws-sent)]" aria-label="Sent" />
  ) : (
    <ArrowDown className="size-3 text-[var(--ws-received)]" aria-label="Received" />
  );
}

function MessageDetail({ id, message, sendable }: { id: number; message: WsMessage_Serialize; sendable: boolean }) {
  const store = useUi((st) => st.store);
  // An edited message shows what was forwarded, or what arrived.
  const [original, setOriginal] = useState(false);
  const showOriginal = original && message.edited;
  const payload = useQuery({
    queryKey: ["ws-payload", store, id, message.index, showOriginal],
    queryFn: async () =>
      new Uint8Array(
        await (showOriginal ? backend.getWebsocketOriginal(store, id, message.index) : backend.getWebsocketPayload(store, id, message.index)),
      ),
  });
  const who = message.injected
    ? message.direction === "sent"
      ? "Sent to the server by Proxymate"
      : "Sent to the client by Proxymate"
    : message.direction === "sent"
      ? "Sent by the client"
      : "Received from the server";
  const notes = [`${who} · ${OPCODE_LABEL[message.opcode]} · ${message.len} bytes · ${formatTime(message.time_ms)}`];
  if (message.close_code !== null) notes.push(`Close code ${closeCodeLabel(message.close_code)}`);
  if (message.truncated) notes.push(`Recorded the first ${formatSize(payload.data?.length ?? 0)} of ${formatSize(message.len)}`);
  if (message.dropped) notes.push("Dropped by a rule, script or breakpoint: it was never forwarded.");
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="ws-message-detail">
      {notes.map((note) => (
        <div key={note} className="text-muted-foreground flex items-center gap-1.5 border-b px-2 py-1 text-[11px]">
          {note === notes[0] && <DirectionArrow direction={message.direction} />}
          <span className="min-w-0 flex-1 truncate">{note}</span>
          {note === notes[0] && sendable && canResend(message) && (
            <Button variant="ghost" size="xs" className="h-5" onClick={() => void resend(id, message)} data-testid="ws-detail-resend">
              <RotateCcw />
              Resend
            </Button>
          )}
        </div>
      ))}
      {message.edited && (
        <div className="flex items-center gap-1.5 border-b px-2 py-1 text-[11px]" data-testid="ws-edited-bar">
          <span className="text-muted-foreground flex-1">A rule, script or breakpoint changed this message.</span>
          <ToggleGroup type="single" value={showOriginal ? "original" : "forwarded"} onValueChange={(v) => v && setOriginal(v === "original")} className="gap-0.5">
            <ToggleGroupItem value="forwarded" className={chip}>
              Forwarded
            </ToggleGroupItem>
            <ToggleGroupItem value="original" className={chip} data-testid="ws-show-original">
              Original
            </ToggleGroupItem>
          </ToggleGroup>
        </div>
      )}
      {payload.isError ? (
        <Placeholder text={`Couldn't load the payload: ${String(payload.error)}`} />
      ) : payload.data ? (
        <PayloadView
          key={`${message.index}-${showOriginal}`}
          id={`ws-${id}-${message.index}${showOriginal ? "-original" : ""}`}
          bytes={payload.data}
          binary={message.opcode === "binary"}
        />
      ) : (
        <Placeholder text="Loading…" />
      )}
    </div>
  );
}

const chip = "h-5 gap-1 rounded-sm px-2 text-xs data-[state=on]:bg-primary-chip data-[state=on]:text-primary-chip-foreground";
