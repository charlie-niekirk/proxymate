import { useQuery } from "@tanstack/react-query";
import { Copy, Download, FileCode2 } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";

import type { BodyInfo, BodyPart, ContentKind, Header } from "@/bindings";
import { useBody, useBodyUrl } from "@/components/Inspector/useBody";
import { BodyFallbackMenu } from "@/components/menus/body";
import { BodyCopyContext } from "@/components/menus/bodyContext";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useViewScriptsVersion } from "@/hooks/useScripts";
import { backend } from "@/lib/backend";
import { essence, kindFromContentType } from "@/lib/body/content";
import { parseGraphQlQueryString } from "@/lib/body/graphql";
import { copyBodyText } from "@/lib/clipboard";
import { bodySearchTerms } from "@/lib/dsl";
import { formatSize } from "@/lib/flow";
import { headerValue } from "@/lib/http";
import { flowViewsKey } from "@/lib/query";
import { chooseViewer, isScriptValue, scriptErrors, scriptViewMime, scriptViewOptions } from "@/lib/scripts";
import { useUi } from "@/store/ui";

import { loadViewers, preloadViewer } from "./code/load";
import { Bar, Centered, Note } from "./parts";
import { ScriptViewBody } from "./ScriptViewBody";
import { BodySearchContext } from "./searchTerms";
import { pickViewer, VIEWERS, viewersFor, type ViewerId } from "./registry";
import { HIGHLIGHT_LIMIT, LOAD_LIMIT, suggestedFileName } from "./util";
import { Viewer } from "./Viewer";

const Operations = lazy(() => loadViewers().then((m) => ({ default: m.Operations })));
const DecodedView = lazy(() => loadViewers().then((m) => ({ default: m.DecodedView })));

/**
 * The Body tab: picks the best viewer for the body's detected kind (the
 * dropdown overrides it, and the choice sticks per kind), decodes on
 * demand and keeps huge bodies cheap. View scripts (M14) add a "Scripts"
 * group to the dropdown; their first view is shown unless the user picked
 * a built-in viewer instead.
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
  // A trailers-only gRPC response has no body but still has a status to show.
  const grpcHeader = kindFromContentType(contentType) === "grpc";
  const kind: ContentKind =
    info.kind !== "none" ? info.kind : info.size > 0 || (grpcHeader && part === "response") ? fallbackKind(contentType) : "none";
  const preferred = useUi((s) => s.viewerByKind[kind]);
  const store = useUi((s) => s.store);
  const filterText = useUi((s) => s.filter.text);
  const searchTerms = useMemo(() => bodySearchTerms(filterText, part), [filterText, part]);
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
  const scripts = useViewScriptsVersion();
  const views = useQuery({
    queryKey: flowViewsKey(store, id, part, scripts),
    queryFn: () => backend.getFlowViews(store, id, part),
    enabled: finished && scripts !== "",
  });
  const results = useMemo(() => (scripts === "" ? [] : (views.data ?? [])), [scripts, views.data]);
  const options = useMemo(() => scriptViewOptions(results), [results]);
  const errors = scriptErrors(results);
  const picked = useUi((s) => s.pickedScriptView);
  const scriptViewsOff = useUi((s) => s.scriptViewsOff);
  const pickScriptView = useUi((s) => s.pickScriptView);
  const openScripts = useUi((s) => s.openScripts);
  const selected = chooseViewer(viewer, options, picked, scriptViewsOff);
  const scriptView = options.find((o) => o.value === selected) ?? null;
  const choose = (value: string) => {
    if (isScriptValue(value)) {
      pickScriptView(value);
      return;
    }
    setViewer(kind, value as ViewerId);
    if (options.length > 0) pickScriptView(null);
  };

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
  if (info.size === 0 && options.length === 0 && !(kind === "grpc" && part === "response")) {
    return <Centered>{finished ? "No body" : "Waiting for body…"}</Centered>;
  }
  // Decoded viewers show a gRPC stream as it arrives.
  if (!finished && def.source !== "decoded") return <Centered>Body streaming · {formatSize(info.size)} so far</Centered>;

  const save = () => {
    setSaveError(null);
    backend.saveBody(store, id, part, true, suggestedFileName(url, mime, part)).catch((e: unknown) => setSaveError(String(e)));
  };
  const loaded = def.source === "url" ? bodyUrl : body;
  const bytes = body.data?.bytes ?? null;
  const tooBig = bytes !== null && bytes.length > HIGHLIGHT_LIMIT;
  const highlight = !tooBig || highlightAnyway;

  return (
    <div className="flex h-full flex-col" data-testid="body-viewer" data-viewer={viewer} data-kind={kind}>
      <Bar>
        <Select value={selected} onValueChange={choose}>
          <SelectTrigger className="h-5 min-w-24 px-1.5 text-[11px]" aria-label="Viewer" data-testid="viewer-select">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {viewersFor(kind).map((v) => (
              <SelectItem key={v} value={v} className="text-xs">
                {VIEWERS[v].label}
              </SelectItem>
            ))}
            {options.length > 0 && (
              <>
                <SelectSeparator />
                <SelectGroup data-testid="script-views">
                  <SelectLabel className="text-2xs">Scripts</SelectLabel>
                  {options.map((o) => (
                    <SelectItem key={o.value} value={o.value} className="text-xs" title={o.scriptName}>
                      <FileCode2 className="size-3 text-[var(--rule)]" />
                      {o.label}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </>
            )}
          </SelectContent>
        </Select>
        <span className="text-muted-foreground truncate font-mono">
          {scriptView ? `${scriptView.scriptName} · ${scriptViewMime(scriptView.view)}` : `${mime} · ${formatSize(info.size)}`}
        </span>
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto"
          disabled={!bytes || bytes.length === 0}
          title={bytes ? "Copy the body as text (Base64 from the right-click menu)" : "The viewer loads the body itself: save it instead"}
          onClick={() => bytes && void copyBodyText(bytes, "body")}
          data-testid="copy-body"
        >
          <Copy /> Copy
        </Button>
        <Button variant="ghost" size="xs" onClick={save} data-testid="save-body">
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
      {errors.map((e) => (
        <Note key={e.scriptId} tone="warn" testId="script-view-error">
          <span className="min-w-0 truncate" title={e.error}>
            View script “{e.scriptName}” failed: {e.error.split("\n")[0]}
          </span>
          <Button variant="outline" size="xs" className="h-5 shrink-0" onClick={() => openScripts({ scriptId: e.scriptId })}>
            Open script
          </Button>
        </Note>
      ))}
      {scriptView?.view.truncated && <Note tone="warn">The script&apos;s view was cut at 8 MB.</Note>}
      <BodyCopyContext.Provider value={{ bytes, what: "body", onSave: save }}>
        <BodySearchContext.Provider value={searchTerms}>
        <BodyFallbackMenu>
          <div className="min-h-0 flex-1">
            {scriptView ? (
              <ScriptViewBody option={scriptView} bodyId={`${id}-${part}`} />
            ) : def.source === "decoded" ? (
              <Suspense fallback={<Centered>Loading…</Centered>}>
                <DecodedView id={id} part={part} size={info.size} finished={finished} url={url} grpc={kind === "grpc"} />
              </Suspense>
            ) : loaded.isError ? (
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
        </BodyFallbackMenu>
        </BodySearchContext.Provider>
      </BodyCopyContext.Provider>
    </div>
  );
}

function fallbackKind(contentType: string | null): ContentKind {
  const kind = kindFromContentType(contentType);
  return kind === "none" ? "text" : kind;
}
