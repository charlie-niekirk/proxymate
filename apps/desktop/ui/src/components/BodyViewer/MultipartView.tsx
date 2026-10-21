import { useEffect, useMemo } from "react";

import { sniff } from "@/lib/body/content";
import { boundaryOf, parseMultipart, type MultipartPart } from "@/lib/body/multipart";
import { formatSize } from "@/lib/flow";
import { displayHeaderName } from "@/lib/http";

import { Centered, Note } from "./parts";
import { VIEWERS, viewersFor } from "./registry";
import { HexView } from "./HexView";
import { Viewer } from "./Viewer";

/** Each part with its headers, shown in the sub-viewer that fits it; images inline. */
export function MultipartView({ bytes, contentType, bodyId }: { bytes: Uint8Array; contentType: string | null; bodyId: string }) {
  const boundary = boundaryOf(contentType);
  const parts = useMemo(() => (boundary ? parseMultipart(bytes, boundary) : []), [bytes, boundary]);
  if (!boundary) {
    return (
      <div className="flex h-full flex-col">
        <Note tone="warn">The Content-Type has no boundary, so the parts can&apos;t be split.</Note>
        <div className="min-h-0 flex-1">
          <HexView bytes={bytes} />
        </div>
      </div>
    );
  }
  if (parts.length === 0) return <Centered>No parts found for boundary {boundary}</Centered>;
  return (
    <div className="h-full overflow-auto p-2" data-testid="multipart-view">
      <div className="text-muted-foreground mb-2 text-[11px]">
        {parts.length} part{parts.length === 1 ? "" : "s"} · boundary <span className="font-mono">{boundary}</span>
      </div>
      <div className="flex flex-col gap-2">
        {parts.map((p, i) => (
          <PartCard key={i} part={p} bodyId={`${bodyId}-part${i}`} />
        ))}
      </div>
    </div>
  );
}

function PartCard({ part, bodyId }: { part: MultipartPart; bodyId: string }) {
  const sniffed = useMemo(() => sniff(part.body, part.contentType), [part]);
  const viewer = viewersFor(sniffed.kind)[0]!;
  const short = sniffed.kind === "text" && part.body.length <= 200 && !part.body.includes(10);
  return (
    <div className="bg-card overflow-hidden rounded-md border" data-testid="multipart-part">
      <div className="flex items-center gap-2 border-b bg-[var(--pane-header)] px-2 py-1 text-[11px]">
        <span className="font-mono font-semibold text-[var(--syntax-key)]">{part.name ?? "(unnamed)"}</span>
        {part.filename && <span className="font-mono">{part.filename}</span>}
        <span className="text-muted-foreground ml-auto">
          {part.contentType ?? sniffed.mime} · {formatSize(part.body.length) === "—" ? "0 B" : formatSize(part.body.length)}
        </span>
      </div>
      <table className="w-full font-mono text-[10.5px]" data-selectable>
        <tbody>
          {part.headers.map(([k, v], i) => (
            <tr key={i}>
              <td className="text-muted-foreground w-40 px-2 align-top">{displayHeaderName(k)}</td>
              <td className="px-2 break-all">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {part.body.length === 0 ? null : short ? (
        <div className="border-t px-2 py-1 font-mono text-xs whitespace-pre-wrap" data-selectable>
          {new TextDecoder().decode(part.body)}
        </div>
      ) : (
        <div className="h-56 border-t">
          <PartBody part={part} viewer={viewer} mime={sniffed.mime} bodyId={bodyId} />
        </div>
      )}
    </div>
  );
}

function PartBody({ part, viewer, mime, bodyId }: { part: MultipartPart; viewer: ReturnType<typeof viewersFor>[number]; mime: string; bodyId: string }) {
  const byUrl = VIEWERS[viewer].source === "url";
  const url = useMemo(
    () => (byUrl ? URL.createObjectURL(new Blob([part.body.slice()], { type: mime })) : null),
    [byUrl, part.body, mime],
  );
  useEffect(() => () => (url ? URL.revokeObjectURL(url) : undefined), [url]);
  return (
    <Viewer
      viewer={viewer}
      bytes={part.body}
      url={url}
      mime={mime}
      size={part.body.length}
      contentType={part.contentType}
      highlight
      bodyId={bodyId}
    />
  );
}
