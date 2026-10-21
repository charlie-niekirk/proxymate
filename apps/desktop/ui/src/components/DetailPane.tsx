import { useQuery } from "@tanstack/react-query";
import { ShieldAlert, ShieldOff } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";
import { MethodPill, StatusPill, UrlText } from "@/components/FlowTable/cells";
import { BodyView } from "@/components/Inspector/BodyView";
import { KvTable, Placeholder, type Kv } from "@/components/Inspector/KvTable";
import { RawView } from "@/components/Inspector/RawView";
import { TimingView } from "@/components/Inspector/TimingView";
import { TlsView } from "@/components/Inspector/TlsView";
import { Button } from "@/components/ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { addSslBypass, setInsecureUpstream } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatDuration, formatSize } from "@/lib/flow";
import { displayHeaderName, queryPairs } from "@/lib/http";
import { flowKey, sslSettingsKey } from "@/lib/query";
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
        <span
          className="text-muted-foreground rounded-sm border px-1 font-mono text-2xs leading-4"
          data-testid="http-version"
        >
          {flow.http_version}
        </span>
        <span className="text-muted-foreground font-mono text-xs tabular-nums">
          {formatDuration(flow.duration_ms)} · {formatSize(flow.resp_size)}
        </span>
        {(flow.tls === "intercepted" || flow.tls === "likely_pinned") && <BypassButton host={flow.host} compact />}
      </div>
      {flow.tls === "likely_pinned" && (
        <div
          className="flex shrink-0 items-center gap-2 border-b bg-[var(--ssl-pinned-bg)] px-2 py-1.5 text-xs"
          data-testid="pinned-banner"
        >
          <ShieldAlert className="size-3.5 shrink-0 text-[var(--ssl-pinned)]" />
          <span className="min-w-0 flex-1">
            The client rejected Proxymate&apos;s certificate: it pins its certificates, or doesn&apos;t trust the
            Proxymate CA yet.
          </span>
          <BypassButton host={flow.host} />
        </div>
      )}
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
  const { response, response_body: info, error, tls } = detail;
  const finished = flow.state !== "active";
  // The upstream presented a certificate but the handshake failed: offer to
  // accept it unverified (self-signed dev servers and the like).
  const certRejected = flow.state === "failed" && flow.tls === "intercepted" && !!tls && tls.version === "" && tls.chain.length > 0;
  const noResponse = <Placeholder text={error ? "No response" : "Waiting for the response…"} />;
  return (
    <MessageTabs title="Response" tabs={["Header", "Body", "Raw", "TLS", "Timing"]} value={tab} onChange={setTab}>
      {error && (
        <div className="flex items-start gap-2 border-b px-2 py-1.5 text-xs text-[var(--s-failed)]" data-selectable>
          <span className="min-w-0 flex-1">{error}</span>
          {certRejected && <InsecureUpstreamButton host={flow.host} />}
        </div>
      )}
      {flow.tls === "passthrough" && (
        <div className="text-muted-foreground border-b px-2 py-1.5 text-xs" data-testid="tunnel-note">
          Not decrypted: {flow.host} is in the SSL exclude or bypass list. {formatSize(flow.req_size)} sent,{" "}
          {formatSize(flow.resp_size)} received through the tunnel.
        </div>
      )}
      <TabsContent value="Header">
        {response ? <KvTable rows={response.headers.map((h) => [displayHeaderName(h.name), h.value])} /> : noResponse}
      </TabsContent>
      <TabsContent value="Body">
        {response ? <BodyView id={flow.id} part="response" info={info} finished={finished} /> : noResponse}
      </TabsContent>
      <TabsContent value="Raw">
        {response ? (
          <RawView
            id={flow.id}
            part="response"
            startLine={`${response.version} ${response.status} ${response.reason}`}
            headers={response.headers}
            info={info}
            finished={finished}
          />
        ) : (
          noResponse
        )}
      </TabsContent>
      <TabsContent value="TLS">
        <TlsView flow={flow} detail={detail} />
      </TabsContent>
      <TabsContent value="Timing">
        <TimingView flow={flow} timing={detail.timing} />
      </TabsContent>
    </MessageTabs>
  );
}

/** "Add to SSL bypass": stop decrypting the host from its next connection. */
function BypassButton({ host, compact = false }: { host: string; compact?: boolean }) {
  const { data: settings } = useQuery({ queryKey: sslSettingsKey, queryFn: () => backend.getSslSettings() });
  const [error, setError] = useState<string | null>(null);
  const bypassed = settings?.bypass.includes(host.toLowerCase()) ?? false;
  const label = bypassed ? "In SSL bypass" : compact ? "Bypass SSL" : `Add ${host} to SSL bypass`;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant={compact ? "ghost" : "outline"}
          size="xs"
          disabled={bypassed}
          data-testid={compact ? "bypass-ssl" : "bypass-ssl-banner"}
          onClick={() => addSslBypass(host).catch((e: unknown) => setError(String(e)))}
        >
          <ShieldOff />
          {label}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {error ?? (bypassed ? `${host} is no longer decrypted` : `Stop decrypting ${host}: new connections are tunnelled as is`)}
      </TooltipContent>
    </Tooltip>
  );
}

function InsecureUpstreamButton({ host }: { host: string }) {
  const { data: settings } = useQuery({ queryKey: sslSettingsKey, queryFn: () => backend.getSslSettings() });
  const allowed = settings?.insecure_upstream.includes(host.toLowerCase()) ?? false;
  return (
    <Button
      variant="outline"
      size="xs"
      className="text-foreground shrink-0"
      onClick={() => void setInsecureUpstream(host, !allowed)}
    >
      {allowed ? "Verify upstream again" : "Allow insecure upstream"}
    </Button>
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
