import { useMemo } from "react";

import type { BodyInfo, BodyPart } from "@/bindings";
import { formatSize } from "@/lib/flow";
import { bodyText, jsonTokens, prettyJson, type JsonTokenKind } from "@/lib/http";

import { Placeholder } from "./KvTable";
import { useBody } from "./useBody";

/** Text beyond this is cut; rich large-body handling arrives in M3. */
const MAX_TEXT = 1024 * 1024;
/** Pretty JSON beyond this is shown uncoloured. */
const MAX_HIGHLIGHT = 256 * 1024;

const TOKEN_COLOR: Record<JsonTokenKind, string | undefined> = {
  key: "var(--syntax-key)",
  string: "var(--syntax-string)",
  number: "var(--syntax-number)",
  literal: "var(--syntax-literal)",
  punct: undefined,
};

/**
 * The Body tab: decoded on demand, pretty-printed and coloured when it is
 * JSON, plain text otherwise. Hex, images and the other rich viewers are M3.
 */
export function BodyView({
  id,
  part,
  info,
  finished,
}: {
  id: number;
  part: BodyPart;
  info: BodyInfo;
  finished: boolean;
}) {
  const body = useBody(id, part, info, true, finished);
  const view = useMemo(() => (body.data ? renderable(body.data.bytes) : null), [body.data]);

  if (info.size === 0) return <Placeholder text={finished ? "No body" : "Waiting for body…"} />;
  if (!finished) return <Placeholder text={`Body streaming · ${formatSize(info.size)} so far`} />;
  if (body.isError) return <Placeholder text={`Couldn't load body: ${String(body.error)}`} />;
  if (!body.data || !view) return <Placeholder text="Loading…" />;

  const { decodedFrom, decodeError, bytes } = body.data;
  const notes: string[] = [];
  if (decodedFrom) notes.push(`Decoded from ${decodedFrom} · ${formatSize(info.captured)} → ${formatSize(bytes.length)}`);
  if (decodeError) notes.push(`Couldn't decode: ${decodeError}. Showing the raw bytes.`);
  if (info.truncated) notes.push(`Recorded the first ${formatSize(info.captured)} of ${formatSize(info.size)}`);
  if (view.cut) notes.push(`Showing the first ${formatSize(MAX_TEXT)}`);

  return (
    <div className="flex flex-col">
      {notes.map((note) => (
        <div key={note} className="text-muted-foreground border-b px-2 py-1 text-[11px]" data-testid="body-note">
          {note}
        </div>
      ))}
      {view.kind === "binary" ? (
        <Placeholder text={`Binary body · ${formatSize(bytes.length)} (hex view arrives in M3)`} />
      ) : (
        <pre className="px-2 py-1.5 font-mono text-xs leading-[1.45] break-all whitespace-pre-wrap" data-selectable>
          {view.kind === "json" && view.text.length <= MAX_HIGHLIGHT
            ? jsonTokens(view.text).map((t, i) => (
                <span key={i} style={{ color: TOKEN_COLOR[t.kind] }}>
                  {t.text}
                </span>
              ))
            : view.text}
        </pre>
      )}
    </div>
  );
}

type Renderable = { kind: "json" | "text"; text: string; cut: boolean } | { kind: "binary"; cut: false };

function renderable(bytes: Uint8Array): Renderable {
  const cut = bytes.length > MAX_TEXT;
  const text = bodyText(cut ? bytes.subarray(0, MAX_TEXT) : bytes);
  if (text === null) return { kind: "binary", cut: false };
  const pretty = cut ? null : prettyJson(text);
  return pretty === null ? { kind: "text", text, cut } : { kind: "json", text: pretty, cut };
}
