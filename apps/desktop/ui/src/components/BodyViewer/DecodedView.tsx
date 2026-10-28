import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import type { BodyPart, DecodedBody_Serialize, DecodedMessage_Serialize, GrpcStatus } from "@/bindings";
import { StatusPill } from "@/components/FlowTable/cells";
import { KvTable } from "@/components/Inspector/KvTable";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "@/components/ui/select";
import { reportError } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { parseLossless } from "@/lib/body/losslessJson";
import { formatSize } from "@/lib/flow";
import { badgeMap, mappingFor, preview, withMapping } from "@/lib/proto/decoded";
import { decodedKey, decodedRootKey, protoMappingsKey, protoTypesKey, queryClient } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

import { Tree } from "./JsonViews";
import { Bar, Centered, Note } from "./parts";

const AUTO = "__auto";
/** A live stream is decoded again this often while it lasts. */
const LIVE_REFRESH_MS = 500;

/**
 * gRPC, protobuf, MessagePack and CBOR bodies, decoded in Rust
 * (`Engine::decode_body`, which MCP shares). gRPC lists its messages
 * (index, size, compressed) above the selected one's tree, then the
 * status and trailers; a live server stream fills in as it arrives.
 * Protobuf can be decoded as a chosen message type ("Decode as…").
 */
export function DecodedView({
  id,
  part,
  size,
  finished,
  url,
  grpc,
}: {
  id: number;
  part: BodyPart;
  size: number;
  finished: boolean;
  url: string;
  grpc: boolean;
}) {
  const store = useUi((s) => s.store);
  const decoded = useQuery({
    queryKey: decodedKey(store, id, part, size, finished),
    queryFn: () => backend.decodeBody(store, id, part),
    refetchInterval: finished ? false : LIVE_REFRESH_MS,
    placeholderData: (previous) => previous,
  });
  if (decoded.isError) return <Centered>Couldn&apos;t decode: {String(decoded.error)}</Centered>;
  if (!decoded.data) return <Centered>Decoding…</Centered>;
  return <Decoded body={decoded.data} part={part} url={url} grpc={grpc} finished={finished} />;
}

function Decoded({
  body,
  part,
  url,
  grpc,
  finished,
}: {
  body: DecodedBody_Serialize;
  part: BodyPart;
  url: string;
  grpc: boolean;
  finished: boolean;
}) {
  const [picked, setPicked] = useState<number | null>(null);
  const messages = body.messages;
  // Follow a live stream to its newest message until one is picked.
  const index = picked !== null && picked < messages.length ? picked : finished ? 0 : Math.max(0, messages.length - 1);
  const message = messages[index];
  const list = grpc || messages.length > 1;
  const count = `${messages.length} message${messages.length === 1 ? "" : "s"}`;
  return (
    <div className="flex h-full flex-col" data-testid="decoded-view" data-format={body.format}>
      <Bar>
        <span className="text-muted-foreground truncate" data-testid="decoded-summary">
          {body.format === "grpcweb" ? "gRPC-web" : FORMAT_LABEL[body.format]} · {count}
          {body.message_type ? ` · ${body.message_type}` : message?.schema_less ? " · no schema" : ""}
        </span>
        {(body.format === "protobuf" || grpc) && <DecodeAs part={part} url={url} />}
        {body.status && (
          <span className="ml-auto shrink-0">
            <StatusPill status={200} state="completed" grpc={body.status.code} />
          </span>
        )}
      </Bar>
      {body.error && <Note tone="warn">{body.error}</Note>}
      {!finished && <Note testId="decoded-live">Streaming · {body.pending_bytes > 0 ? `${formatSize(body.pending_bytes)} of the next message so far` : "waiting for more"}</Note>}
      {finished && body.pending_bytes > 0 && <Note tone="warn">{formatSize(body.pending_bytes)} at the end aren&apos;t a whole message</Note>}
      {list && messages.length > 0 && <MessageList messages={messages} selected={index} onSelect={setPicked} />}
      {message?.error && (
        <Note tone="warn" testId="decoded-message-error">
          {message.schema_less ? `Shown schema-less: ${message.error}` : message.error}
        </Note>
      )}
      <div className="min-h-0 flex-1">
        {message ? <MessageTree key={`${index}-${message.json.length}`} message={message} /> : <Centered>{finished ? "No messages" : "No messages yet"}</Centered>}
      </div>
      {(body.status || body.trailers.length > 0) && <Trailers status={body.status} trailers={body.trailers} />}
    </div>
  );
}

const FORMAT_LABEL: Record<DecodedBody_Serialize["format"], string> = {
  grpc: "gRPC",
  grpcweb: "gRPC-web",
  protobuf: "Protobuf",
  messagepack: "MessagePack",
  cbor: "CBOR",
};

function MessageList({
  messages,
  selected,
  onSelect,
}: {
  messages: DecodedMessage_Serialize[];
  selected: number;
  onSelect: (index: number) => void;
}) {
  return (
    <div className="max-h-[35%] shrink-0 overflow-auto border-b font-mono text-[11px]" role="listbox" data-testid="decoded-messages">
      {messages.map((m) => (
        <button
          key={m.index}
          type="button"
          role="option"
          aria-selected={m.index === selected}
          onClick={() => onSelect(m.index)}
          className={cn(
            "flex h-5 w-full items-center gap-2 px-2 text-left hover:bg-[var(--code-row-hover)]",
            m.index === selected && "bg-accent",
          )}
          data-testid="decoded-message"
        >
          <span className="text-muted-foreground w-8 shrink-0 tabular-nums">#{m.index + 1}</span>
          <span className="text-muted-foreground w-14 shrink-0 text-right tabular-nums">{formatSize(m.size)}</span>
          <span className="flex w-40 shrink-0 gap-1 font-sans text-[10px] whitespace-nowrap">
            {m.compressed && <Tag>compressed</Tag>}
            {m.schema_less && <Tag>schema-less</Tag>}
            {m.error && !m.schema_less && <Tag warn>error</Tag>}
          </span>
          <span className="min-w-0 truncate">{preview(m.json)}</span>
        </button>
      ))}
    </div>
  );
}

function Tag({ children, warn }: { children: string; warn?: boolean }) {
  return (
    <span className={cn("rounded-sm border px-1 leading-[14px]", warn ? "text-[var(--s-failed)]" : "text-muted-foreground")}>
      {children}
    </span>
  );
}

function MessageTree({ message }: { message: DecodedMessage_Serialize }) {
  const root = useMemo(() => parseLossless(message.json), [message.json]);
  const badges = useMemo(() => badgeMap(message), [message]);
  return <Tree root={root} badges={badges} testId="decoded-tree" />;
}

function Trailers({ status, trailers }: { status: GrpcStatus | null; trailers: { name: string; value: string }[] }) {
  return (
    <div className="max-h-[40%] shrink-0 overflow-auto border-t" data-testid="grpc-trailers">
      {status && (
        <div className="flex items-center gap-2 px-2 py-1 text-xs" data-testid="grpc-status">
          <span className="font-mono font-semibold">
            {status.name} ({status.code})
          </span>
          {status.message && <span className="text-muted-foreground min-w-0 truncate" title={status.message}>{status.message}</span>}
        </div>
      )}
      {status?.details && (
        <pre className="text-muted-foreground border-t px-2 py-1 font-mono text-[11px] whitespace-pre-wrap" data-selectable data-testid="grpc-status-details">
          {status.details}
        </pre>
      )}
      {trailers.length > 0 && <KvTable rows={trailers.map((t) => [t.name, t.value])} kind="header" side="response" testId="grpc-trailer-list" />}
    </div>
  );
}

/** "Decode as…": picks a message type for this flow's body, saved as a mapping. */
function DecodeAs({ part, url }: { part: BodyPart; url: string }) {
  const types = useQuery({ queryKey: protoTypesKey, queryFn: () => backend.listProtoMessageTypes() });
  const mappings = useQuery({ queryKey: protoMappingsKey, queryFn: () => backend.listProtoMappings() });
  const current = mappings.data ? mappingFor(mappings.data, url, part) : undefined;
  if (!types.data?.length) return null;
  const choose = async (value: string) => {
    try {
      const next = withMapping(mappings.data ?? [], url, part, value === AUTO ? null : value, crypto.randomUUID());
      await backend.setProtoMappings(next);
      await queryClient.invalidateQueries({ queryKey: protoMappingsKey });
      await queryClient.invalidateQueries({ queryKey: decodedRootKey });
    } catch (e) {
      reportError("Couldn't set the message type", e);
    }
  };
  return (
    <Select value={current?.message_type ?? AUTO} onValueChange={(v) => void choose(v)}>
      <SelectTrigger className="h-5 max-w-56 px-1.5 text-[11px]" aria-label="Decode as" data-testid="decode-as">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={AUTO} className="text-xs">
          Decode as: automatic
        </SelectItem>
        <SelectSeparator />
        {types.data.map((t) => (
          <SelectItem key={t} value={t} className="font-mono text-xs">
            {t}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
