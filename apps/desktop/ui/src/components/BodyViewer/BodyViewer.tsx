import { Download } from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";

import type { BodyInfo, BodyPart, ContentKind, Header } from "@/bindings";
import { useBody, useBodyUrl } from "@/components/Inspector/useBody";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { backend } from "@/lib/backend";
import { essence, kindFromContentType } from "@/lib/body/content";
import { parseGraphQlQueryString } from "@/lib/body/graphql";
import { formatSize } from "@/lib/flow";
import { headerValue } from "@/lib/http";
import { useUi } from "@/store/ui";

import { loadViewers, preloadViewer } from "./code/load";
import { Bar, Centered, Note } from "./parts";
import { pickViewer, VIEWERS, viewersFor, type ViewerId } from "./registry";
import { HIGHLIGHT_LIMIT, LOAD_LIMIT, suggestedFileName } from "./util";
import { Viewer } from "./Viewer";

const Operations = lazy(() => loadViewers().then((m) => ({ default: m.Operations })));

/**
 * The Body tab: picks the best viewer for the body's detected kind (the
 * dropdown overrides it, and the choice sticks per kind), decodes on
 * demand and keeps huge bodies cheap.
 */
export function BodyViewer({
  id,
  part,
  info,
  finished,
  headers,
  url,
}: {
  id: number;
  part: BodyPart;
  info: BodyInfo;
  finished: boolean;
  headers: Header[];
  /** The request URL, for file names and GET GraphQL operations. */
  url: string;
}) {
  const contentType = headerValue(headers, "content-type") ?? null;
  // Flows recorded before detection ran (or still streaming) fall back to the header.
  const kind: ContentKind = info.kind !== "none" ? info.kind : info.size > 0 ? fallbackKind(contentType) : "none";
  const preferred = useUi((s) => s.viewerByKind[kind]);
  const store = useUi((s) => s.store);
  const setViewer = useUi((s) => s.setViewer);
  const viewer = pickViewer(kind, preferred);
  const def = VIEWERS[viewer];
  const [loadAll, setLoadAll] = useState(false);
  const [highlightAnyway, setHighlightAnyway] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const capped = info.captured > LOAD_LIMIT && !loadAll;
  const body = useBody(id, part, info, true, finished, capped ? LOAD_LIMIT : null, def.source === "bytes");
  const bodyUrl = useBodyUrl(id, part, info, finished, def.source === "url");
  const mime = info.mime ?? (essence(contentType) || "application/octet-stream");
  // Fetch the viewer's code alongside the body rather than after it.
  useEffect(() => preloadViewer(viewer, def.code), [viewer, def.code]);

  if (info.size === 0 && kind === "graphql") {
    const op = parseGraphQlQueryString(url);
    if (op) {
      return (
        <Suspense fallback={<Centered>Loading…</Centered>}>
          <Operations ops={[op]} />
        </Suspense>
      );
    }
  }
  if (info.size === 0) return <Centered>{finished ? "No body" : "Waiting for body…"}</Centered>;
  if (!finished) return <Centered>Body streaming · {formatSize(info.size)} so far</Centered>;

  const save = () => {
    setSaveError(null);
    backend.saveBody(store, id, part, true, suggestedFileName(url, mime, part)).catch((e: unknown) => setSaveError(String(e)));
  };
  const loaded = def.source === "bytes" ? body : bodyUrl;
  const bytes = body.data?.bytes ?? null;
  const tooBig = bytes !== null && bytes.length > HIGHLIGHT_LIMIT;
  const highlight = !tooBig || highlightAnyway;

  return (
    <div className="flex h-full flex-col" data-testid="body-viewer" data-viewer={viewer} data-kind={kind}>
      <Bar>
        <Select value={viewer} onValueChange={(v) => setViewer(kind, v as ViewerId)}>
          <SelectTrigger className="h-5 min-w-24 px-1.5 text-[11px]" aria-label="Viewer" data-testid="viewer-select">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {viewersFor(kind).map((v) => (
              <SelectItem key={v} value={v} className="text-xs">
                {VIEWERS[v].label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="text-muted-foreground truncate font-mono">
          {mime} · {formatSize(info.size)}
        </span>
        <Button variant="ghost" size="xs" className="ml-auto" onClick={save} data-testid="save-body">
          <Download /> Save…
        </Button>
      </Bar>
      {body.data?.decodedFrom && (
        <Note>
          Decoded from {body.data.decodedFrom} · {formatSize(info.captured)} → {capped ? `${formatSize(bytes!.length)}+` : formatSize(bytes!.length)}
        </Note>
      )}
      {body.data?.decodeError && <Note tone="warn">Couldn&apos;t decode: {body.data.decodeError}. Showing the raw bytes.</Note>}
      {info.truncated && (
        <Note>
          Recorded the first {formatSize(info.captured)} of {formatSize(info.size)}
        </Note>
      )}
      {capped && (
        <Note tone="warn" testId="large-body-note">
          Showing the first {formatSize(LOAD_LIMIT)} of {formatSize(info.captured)}.
          <Button variant="outline" size="xs" className="h-5" onClick={() => setLoadAll(true)}>
            Load all
          </Button>
          <Button variant="outline" size="xs" className="h-5" onClick={save}>
            Save body to file…
          </Button>
        </Note>
      )}
      {def.code && tooBig && !highlightAnyway && (
        <Note testId="highlight-note">
          Highlighting is off for bodies over {formatSize(HIGHLIGHT_LIMIT)}.
          <Button variant="outline" size="xs" className="h-5" onClick={() => setHighlightAnyway(true)}>
            Highlight anyway
          </Button>
        </Note>
      )}
      {saveError && <Note tone="warn">Couldn&apos;t save: {saveError}</Note>}
      <div className="min-h-0 flex-1">
        {loaded.isError ? (
          <Centered>Couldn&apos;t load body: {String(loaded.error)}</Centered>
        ) : !loaded.data ? (
          <Centered>Loading…</Centered>
        ) : (
          <Viewer
            viewer={viewer}
            bytes={bytes}
            url={bodyUrl.data ?? null}
            mime={mime}
            size={info.size}
            contentType={contentType}
            highlight={highlight}
            bodyId={`${id}-${part}`}
            onSave={save}
          />
        )}
      </div>
    </div>
  );
}

function fallbackKind(contentType: string | null): ContentKind {
  const kind = kindFromContentType(contentType);
  return kind === "none" ? "text" : kind;
}
