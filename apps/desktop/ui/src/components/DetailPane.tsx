import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";
import { MethodPill, StatusPill, UrlText } from "@/components/FlowTable/cells";
import { BodyView } from "@/components/Inspector/BodyView";
import { KvTable, Placeholder, type Kv } from "@/components/Inspector/KvTable";
import { RawView } from "@/components/Inspector/RawView";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { backend } from "@/lib/backend";
import { formatDuration, formatSize } from "@/lib/flow";
import { displayHeaderName, queryPairs } from "@/lib/http";
import { flowKey } from "@/lib/query";
import { useUi } from "@/store/ui";

interface Inspected {
  flow: FlowSummary_Serialize;
  /** Null for synthetic flows, which have no recorded messages. */
  detail: FlowDetail_Serialize | null;
}

/** Inspector for the selected flow: URL bar over a Request | Response split. */
export function DetailPane() {
  const id = useUi((s) => s.selectedId);
  const { data } = useQuery({
    queryKey: flowKey(id),
    enabled: id !== null,
    queryFn: async (): Promise<Inspected | null> => {
      const [flow, detail] = await Promise.all([backend.getFlow(id!), backend.getFlowDetail(id!)]);
      return flow ? { flow, detail } : null;
    },
  });

  if (id === null || !data || data.flow.id !== id) {
    return (
      <div className="bg-background text-muted-foreground flex h-full items-center justify-center text-sm">
        Select a flow to inspect it
      </div>
    );
  }

  const { flow, detail } = data;
  return (
    <div className="bg-background flex h-full flex-col" data-testid="inspector">
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
          {detail ? <RequestPane flow={flow} detail={detail} /> : <NoDetail />}
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="50" minSize={240}>
          {detail ? <ResponsePane flow={flow} detail={detail} /> : <NoDetail />}
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}

function RequestPane({ flow, detail }: Inspected & { detail: FlowDetail_Serialize }) {
  const tab = useUi((s) => s.requestTab);
  const setTab = useUi((s) => s.setRequestTab);
  const { request, request_body: info } = detail;
  const finished = flow.state !== "active" || info.complete;
  const query = queryPairs(request.url);
  const headers: Kv = request.headers.map((h) => [displayHeaderName(h.name), h.value]);
  return (
    <MessageTabs title="Request" tabs={["Header", "Query", "Body", "Raw"]} value={tab} onChange={setTab}>
      <TabsContent value="Header">
        <KvTable rows={headers} />
      </TabsContent>
      <TabsContent value="Query">
        {query.length ? <KvTable rows={query} /> : <Placeholder text="No query parameters" />}
      </TabsContent>
      <TabsContent value="Body">
        <BodyView id={flow.id} part="request" info={info} finished={finished} />
      </TabsContent>
      <TabsContent value="Raw">
        <RawView
          id={flow.id}
          part="request"
          startLine={`${request.method} ${request.url} ${request.version}`}
          headers={request.headers}
          info={info}
          finished={finished}
        />
      </TabsContent>
    </MessageTabs>
  );
}

function ResponsePane({ flow, detail }: Inspected & { detail: FlowDetail_Serialize }) {
  const tab = useUi((s) => s.responseTab);
  const setTab = useUi((s) => s.setResponseTab);
  const { response, response_body: info, error } = detail;
  const finished = flow.state !== "active";
  return (
    <MessageTabs title="Response" tabs={["Header", "Body", "Raw"]} value={tab} onChange={setTab}>
      {error && (
        <div className="border-b px-2 py-1.5 text-xs text-[var(--s-failed)]" data-selectable>
          {error}
        </div>
      )}
      {response ? (
        <>
          <TabsContent value="Header">
            <KvTable rows={response.headers.map((h) => [displayHeaderName(h.name), h.value])} />
          </TabsContent>
          <TabsContent value="Body">
            <BodyView id={flow.id} part="response" info={info} finished={finished} />
          </TabsContent>
          <TabsContent value="Raw">
            <RawView
              id={flow.id}
              part="response"
              startLine={`${response.version} ${response.status} ${response.reason}`}
              headers={response.headers}
              info={info}
              finished={finished}
            />
          </TabsContent>
        </>
      ) : (
        !error && <Placeholder text="Waiting for the response…" />
      )}
    </MessageTabs>
  );
}

function MessageTabs<T extends string>({
  title,
  tabs,
  value,
  onChange,
  children,
}: {
  title: string;
  tabs: T[];
  value: T;
  onChange: (tab: T) => void;
  children: ReactNode;
}) {
  return (
    <Tabs value={value} onValueChange={(v) => onChange(v as T)} className="flex h-full flex-col gap-0">
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
      <ScrollArea className="min-h-0 flex-1">{children}</ScrollArea>
    </Tabs>
  );
}

function NoDetail() {
  return <Placeholder text="Synthetic flow: no recorded messages" />;
}
