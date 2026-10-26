import { useMemo, useState } from "react";

import { Bar, Centered } from "@/components/BodyViewer/parts";
import { VIEWERS, viewersFor, type ViewerId } from "@/components/BodyViewer/registry";
import { HIGHLIGHT_LIMIT } from "@/components/BodyViewer/util";
import { Viewer } from "@/components/BodyViewer/Viewer";
import { BodyFallbackMenu } from "@/components/menus/body";
import { BodyCopyContext } from "@/components/menus/bodyContext";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { sniff } from "@/lib/body/content";
import { formatSize } from "@/lib/flow";

/**
 * A WebSocket payload or SSE data, shown with the body viewers: detected
 * from its bytes (JSON, XML, text, …), with a dropdown to pick another.
 * Binary frames start as hex unless they hold JSON. Only viewers that take
 * bytes apply: a message has no URL to load media from.
 */
export function PayloadView({
  bytes,
  binary,
  id,
  what = "payload",
}: {
  bytes: Uint8Array;
  binary: boolean;
  id: string;
  /** Names it in the copy items: "payload" (WebSocket), "data" (SSE). */
  what?: string;
}) {
  const { kind, mime, options } = useMemo(() => {
    const sniffed = sniff(bytes);
    let options = viewersFor(sniffed.kind).filter((v) => VIEWERS[v].source === "bytes");
    if (binary && sniffed.kind !== "json") options = ["hex", ...options.filter((v) => v !== "hex")];
    if (!options.includes("text")) options.push("text");
    return { kind: sniffed.kind, mime: sniffed.mime, options };
  }, [bytes, binary]);
  const [chosen, setChosen] = useState<ViewerId | null>(null);
  const viewer = chosen && options.includes(chosen) ? chosen : options[0]!;

  if (bytes.length === 0) return <Centered>Empty payload</Centered>;
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="payload-view" data-viewer={viewer} data-kind={kind}>
      <Bar>
        <Select value={viewer} onValueChange={(v) => setChosen(v as ViewerId)}>
          <SelectTrigger className="h-5 min-w-24 px-1.5 text-[11px]" aria-label="Viewer">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {options.map((v) => (
              <SelectItem key={v} value={v} className="text-xs">
                {VIEWERS[v].label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="text-muted-foreground truncate font-mono">
          {mime} · {formatSize(bytes.length)}
        </span>
      </Bar>
      <BodyCopyContext.Provider value={{ bytes, what }}>
        <BodyFallbackMenu>
          <div className="min-h-0 flex-1">
            <Viewer
              viewer={viewer}
              bytes={bytes}
              url={null}
              mime={mime}
              size={bytes.length}
              contentType={null}
              highlight={bytes.length <= HIGHLIGHT_LIMIT}
              bodyId={id}
            />
          </div>
        </BodyFallbackMenu>
      </BodyCopyContext.Provider>
    </div>
  );
}
