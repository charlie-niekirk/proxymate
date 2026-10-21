import type { BodyInfo, BodyPart, Header } from "@/bindings";
import { formatSize } from "@/lib/flow";
import { bodyText, rawMessage } from "@/lib/http";

import { useBody } from "./useBody";

/** The Raw tab: the message rebuilt from its parsed parts. */
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
  // Encoded bodies are binary on the wire; only plain ones are fetched.
  const plain = info.encoding === null;
  const body = useBody(id, part, info, false, finished && plain);
  let bodyPart: string | null = null;
  if (info.size > 0) {
    if (!plain) bodyPart = `[${formatSize(info.size)} ${info.encoding}-encoded body]`;
    else if (!finished) bodyPart = `[body streaming · ${formatSize(info.size)} so far]`;
    else if (body.data) bodyPart = bodyText(body.data.bytes) ?? `[${formatSize(info.size)} binary body]`;
  }
  return (
    <div className="flex flex-col">
      <div className="text-muted-foreground border-b px-2 py-1 text-[11px]">
        Rebuilt from the parsed message, not the exact bytes on the wire
      </div>
      <pre className="px-2 py-1.5 font-mono text-xs leading-[1.45] break-all whitespace-pre-wrap" data-selectable>
        {rawMessage(startLine, headers, bodyPart)}
      </pre>
    </div>
  );
}
