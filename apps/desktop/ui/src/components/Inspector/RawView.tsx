import { useMemo } from "react";

import type { BodyInfo, BodyPart, Header } from "@/bindings";
import { CodeView } from "@/components/BodyViewer/CodeView";
import { Note } from "@/components/BodyViewer/parts";
import { HIGHLIGHT_LIMIT } from "@/components/BodyViewer/util";
import { formatSize } from "@/lib/flow";
import { bodyText, rawMessage } from "@/lib/http";

import { useBody } from "./useBody";

/** Bodies past this are left out of the Raw tab; the Body tab handles them. */
const RAW_BODY_MAX = 1024 * 1024;

/** The Raw tab: the message rebuilt from its parsed parts, highlighted. */
export function RawView({
  id,
  part,
  startLine,
  headers,
  info,
  finished,
}: {
  id: number;
  part: BodyPart;
  startLine: string;
  headers: Header[];
  info: BodyInfo;
  finished: boolean;
}) {
  // Encoded bodies are binary on the wire; only plain, modest ones are fetched.
  const plain = info.encoding === null;
  const body = useBody(id, part, info, false, finished && plain && info.captured <= RAW_BODY_MAX);
  const text = useMemo(() => {
    let bodyPart: string | null = null;
    if (info.size > 0) {
      if (!plain) bodyPart = `[${formatSize(info.size)} ${info.encoding}-encoded body]`;
      else if (!finished) bodyPart = `[body streaming · ${formatSize(info.size)} so far]`;
      else if (info.captured > RAW_BODY_MAX) bodyPart = `[${formatSize(info.size)} body: see the Body tab]`;
      else if (body.data) bodyPart = bodyText(body.data.bytes) ?? `[${formatSize(info.size)} binary body]`;
    }
    return rawMessage(startLine, headers, bodyPart);
  }, [startLine, headers, info, plain, finished, body.data]);
  return (
    <div className="flex h-full flex-col">
      <Note>Rebuilt from the parsed message, not the exact bytes on the wire</Note>
      <div className="min-h-0 flex-1" data-testid="raw-view">
        <CodeView text={text} language="http" highlight={text.length <= HIGHLIGHT_LIMIT} wrap />
      </div>
    </div>
  );
}
