import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import type { SseEvent_Serialize } from "@/bindings";
import { CopyItem } from "@/components/menus/items";
import { usePagedList } from "@/hooks/usePagedList";
import { backend } from "@/lib/backend";
import { sseWire } from "@/lib/copyFormats";
import { formatTime } from "@/lib/flow";
import { streamKey } from "@/lib/query";
import { useUi } from "@/store/ui";

import { KvTable, Placeholder, type Kv } from "./KvTable";
import { PayloadView } from "./PayloadView";
import { LiveStatus, MessageList, SearchBox, StreamShell } from "./StreamShell";
import { listHeaderClass, rowClass } from "./streamStyles";

const GRID = "grid-cols-[40px_84px_96px_64px_minmax(0,1fr)]";

/**
 * The events of a `text/event-stream` response, parsed in Rust as the body
 * streams, so the list grows while the response is still open.
 */
export function SsePanel({ id }: { id: number }) {
  const [text, setText] = useState("");
  const [selected, setSelected] = useState<SseEvent_Serialize | null>(null);
  const store = useUi((st) => st.store);

  const { data: head } = useQuery({
    queryKey: [...streamKey(store, id), "sse", text],
    queryFn: () => backend.getSseEvents(store, id, text, 0, 0),
    placeholderData: keepPreviousData,
  });
  const len = head?.len ?? 0;
  const [[first, last], setRange] = useState<[number, number]>([0, -1]);
  const event = usePagedList(["sse-page", store, id, text], len, first, last, async (start, count) =>
    (await backend.getSseEvents(store, id, text, start, count)).events,
  );

  // With Auto Select on, the newest event is inspected until one is clicked.
  const autoSelect = useUi((st) => st.autoScroll);
  const shown = selected ?? (autoSelect && len > 0 ? (event(len - 1) ?? null) : null);
  const total = head?.total ?? 0;
  const count = len === total ? `${total} events` : `${len} of ${total} events`;

  return (
    <StreamShell
      title="Events"
      testId="sse-panel"
      status={
        <>
          <LiveStatus live={head?.open ?? false} label={head?.open ? "Streaming" : "Stream ended"} />
          <span className="shrink-0 tabular-nums" data-testid="event-count">
            · {count}
          </span>
        </>
      }
      controls={<SearchBox value={text} onChange={setText} placeholder="Search events" />}
      list={
        <MessageList
          count={len}
          onRange={setRange}
          menuTestId="sse-menu"
          menu={(index) => {
            const e = event(index);
            return e ? <EventItems event={e} /> : null;
          }}
          emptyText={total === 0 ? (head?.open ? "Waiting for events…" : "No events") : "No events match"}
          header={
            <div className={`${listHeaderClass} ${GRID}`}>
              <span className="text-right">#</span>
              <span>Time</span>
              <span>Event</span>
              <span>Id</span>
              <span>Data</span>
            </div>
          }
          row={(index) => {
            const e = event(index);
            if (!e) return null;
            return (
              <div
                className={`${rowClass(shown?.index === e.index, index)} ${GRID}`}
                onClick={() => setSelected(e)}
                data-testid="sse-row"
              >
                <span className="text-muted-foreground text-right tabular-nums">{e.index + 1}</span>
                <span className="tabular-nums">{formatTime(e.time_ms)}</span>
                <span className="truncate">{e.event ?? "message"}</span>
                <span className="truncate">{e.id ?? ""}</span>
                <span className="truncate">{e.data.slice(0, 300)}</span>
              </div>
            );
          }}
        />
      }
      detail={shown ? <EventDetail event={shown} /> : <Placeholder text="Select an event" />}
    />
  );
}

function EventItems({ event: e }: { event: SseEvent_Serialize }) {
  return (
    <>
      <CopyItem text={e.data} what="event data" testId="sse-copy-data">
        Copy data
      </CopyItem>
      <CopyItem text={sseWire(e)} what="event" testId="sse-copy-event">
        Copy as event-stream text
      </CopyItem>
      <CopyItem text={e.event ?? "message"} what="event name">
        Copy event name
      </CopyItem>
      {e.id !== null && (
        <CopyItem text={e.id} what="event id">
          Copy id
        </CopyItem>
      )}
    </>
  );
}

function EventDetail({ event }: { event: SseEvent_Serialize }) {
  const data = useMemo(() => new TextEncoder().encode(event.data), [event.data]);
  const fields: Kv = [["event", event.event ?? "message (default)"]];
  if (event.id !== null) fields.push(["id", event.id]);
  if (event.retry !== null) fields.push(["retry", `${event.retry} ms`]);
  if (event.time_ms !== null) fields.push(["time", formatTime(event.time_ms)]);
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="sse-event-detail">
      <KvTable rows={fields} testId="sse-fields" />
      {event.truncated && (
        <div className="text-muted-foreground border-b px-2 py-1 text-[11px]">Data cut at 1 MB</div>
      )}
      <PayloadView key={event.index} id={`sse-${event.index}`} bytes={data} binary={false} what="data" />
    </div>
  );
}
