import { useEffect, useState } from "react";

import type { FlowSummary_Serialize } from "@/bindings";
import { MethodPill, StatusPill, UrlText } from "@/components/FlowTable/cells";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { backend } from "@/lib/backend";
import { formatDuration, formatSize } from "@/lib/flow";
import { useUi } from "@/store/ui";

const CONTENT_TYPES: Record<string, string> = {
  json: "application/json; charset=utf-8",
  graphql: "application/json",
  xml: "application/xml",
  html: "text/html; charset=utf-8",
  formurlencoded: "application/x-www-form-urlencoded",
  multipart: "multipart/form-data; boundary=----pm",
  javascript: "text/javascript",
  css: "text/css",
  text: "text/plain",
  image: "image/png",
  svg: "image/svg+xml",
  video: "video/mp4",
  audio: "audio/mp4",
  font: "font/woff2",
  pdf: "application/pdf",
  binary: "application/octet-stream",
};

type Kv = [string, string][];

/** Inspector for the selected flow. Only summary fields are real until M1. */
export function DetailPane() {
  const selectedId = useUi((s) => s.selectedId);
  const total = useUi((s) => s.stats.total);
  const [flow, setFlow] = useState<FlowSummary_Serialize | null>(null);

  useEffect(() => {
    if (selectedId === null) return;
    let cancelled = false;
    void backend.getFlow(selectedId).then((f) => !cancelled && setFlow(f));
    return () => {
      cancelled = true;
    };
    // `total` changes on every tick with traffic, which refreshes a live flow.
  }, [selectedId, total]);

  if (selectedId === null || !flow) {
    return (
      <div className="bg-background text-muted-foreground flex h-full items-center justify-center text-sm">
        Select a flow to inspect it
      </div>
    );
  }

  const query: Kv = [...new URLSearchParams(flow.query)];
  const contentType = CONTENT_TYPES[flow.content_kind];
  const reqHeaders: Kv = [
    ["Host", flow.host],
    ["User-Agent", `${flow.client ?? "Unknown"}/1.0`],
    ["Accept", "*/*"],
    ["Accept-Encoding", "gzip, deflate, br"],
    ...(flow.req_size > 0 ? ([["Content-Length", String(flow.req_size)]] as Kv) : []),
  ];
  const respHeaders: Kv = [
    ...(contentType ? ([["Content-Type", contentType]] as Kv) : []),
    ["Content-Length", String(flow.resp_size)],
    ["Server", "synthetic"],
  ];

  return (
    <div className="bg-background flex h-full flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2" data-selectable>
        <MethodPill method={flow.method} />
        <UrlText row={flow} className="min-w-0 flex-1 text-xs" />
        <StatusPill status={flow.status} state={flow.state} />
        <span className="text-muted-foreground font-mono text-xs tabular-nums">
          {formatDuration(flow.duration_ms)} · {formatSize(flow.resp_size)}
        </span>
      </div>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel defaultSize="50" minSize={240}>
          <MessagePane title="Request" headers={reqHeaders} query={query} />
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="50" minSize={240}>
          <MessagePane title="Response" headers={respHeaders} />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}

function MessagePane({ title, headers, query }: { title: string; headers: Kv; query?: Kv }) {
  const tabs = query ? ["Header", "Query", "Cookies", "Body", "Raw", "Timing"] : ["Header", "Cookies", "Body", "Raw", "TLS"];
  return (
    <Tabs defaultValue="Header" className="flex h-full flex-col gap-0">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2">
        <span className="text-xs font-semibold">{title}</span>
        <TabsList className="h-6 bg-transparent p-0">
          {tabs.map((t) => (
            <TabsTrigger key={t} value={t} className="h-5 px-1.5 text-xs">
              {t}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <TabsContent value="Header">
          <KvTable rows={headers} />
        </TabsContent>
        {query && (
          <TabsContent value="Query">
            {query.length ? <KvTable rows={query} /> : <Placeholder text="No query parameters" />}
          </TabsContent>
        )}
        {tabs
          .filter((t) => t !== "Header" && t !== "Query")
          .map((t) => (
            <TabsContent key={t} value={t}>
              <Placeholder text={`${t} arrives with the proxy (M1–M3)`} />
            </TabsContent>
          ))}
      </ScrollArea>
    </Tabs>
  );
}

function KvTable({ rows }: { rows: Kv }) {
  return (
    <table className="w-full font-mono text-xs" data-selectable>
      <tbody>
        {rows.map(([k, v], i) => (
          <tr key={`${k}-${i}`} className="border-b border-border/50">
            <td className="text-[var(--m-get)] w-1/3 px-2 py-1 align-top font-medium">{k}</td>
            <td className="px-2 py-1 break-all">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Placeholder({ text }: { text: string }) {
  return <div className="text-muted-foreground p-4 text-center text-xs">{text}</div>;
}
