import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useState } from "react";

import type { WsDirection, WsMessage_Serialize, WsQuery_Deserialize } from "@/bindings";
import { CopyItem } from "@/components/menus/items";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { usePagedList } from "@/hooks/usePagedList";
import { backend } from "@/lib/backend";
import { formatSize, formatTime } from "@/lib/flow";
import { streamKey } from "@/lib/query";
import { useUi } from "@/store/ui";
import { OPCODE_LABEL, closeCodeLabel, describeClose } from "@/lib/stream";
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
 * the selected message's payload.
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
        </>
      }
      list={
        <MessageList
          count={len}
          onRange={setRange}
          menuTestId="ws-menu"
          menu={(index) => {
            const m = message(index);
            return m ? <MessageItems id={id} message={m} /> : null;
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
                <span className="truncate">{rowPreview(m)}</span>
              </div>
            );
          }}
        />
      }
      detail={shown ? <MessageDetail id={id} message={shown} /> : <Placeholder text="Select a message" />}
    />
  );
}

/** A message row's copies; payloads are fetched whole (the row only has a preview). */
function MessageItems({ id, message: m }: { id: number; message: WsMessage_Serialize }) {
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

function MessageDetail({ id, message }: { id: number; message: WsMessage_Serialize }) {
  const store = useUi((st) => st.store);
  const payload = useQuery({
    queryKey: ["ws-payload", store, id, message.index],
    queryFn: async () => new Uint8Array(await backend.getWebsocketPayload(store, id, message.index)),
  });
  const notes = [
    `${message.direction === "sent" ? "Sent by the client" : "Received from the server"} · ${OPCODE_LABEL[message.opcode]} · ${message.len} bytes · ${formatTime(message.time_ms)}`,
  ];
  if (message.close_code !== null) notes.push(`Close code ${closeCodeLabel(message.close_code)}`);
  if (message.truncated) notes.push(`Recorded the first ${formatSize(payload.data?.length ?? 0)} of ${formatSize(message.len)}`);
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="ws-message-detail">
      {notes.map((note) => (
        <div key={note} className="text-muted-foreground flex items-center gap-1.5 border-b px-2 py-1 text-[11px]">
          {note === notes[0] && <DirectionArrow direction={message.direction} />}
          {note}
        </div>
      ))}
      {payload.isError ? (
        <Placeholder text={`Couldn't load the payload: ${String(payload.error)}`} />
      ) : payload.data ? (
        <PayloadView key={message.index} id={`ws-${id}-${message.index}`} bytes={payload.data} binary={message.opcode === "binary"} />
      ) : (
        <Placeholder text="Loading…" />
      )}
    </div>
  );
}

const chip = "h-5 gap-1 rounded-sm px-2 text-xs data-[state=on]:bg-primary-chip data-[state=on]:text-primary-chip-foreground";
