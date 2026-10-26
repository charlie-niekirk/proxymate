import { useQuery } from "@tanstack/react-query";
import { CircleAlert, PencilLine, ShieldAlert, ShieldOff } from "lucide-react";
import { useState, type ReactElement, type ReactNode } from "react";

import type { FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";
import { MethodPill, StatusPill, UrlText } from "@/components/FlowTable/cells";
import { BodyViewer } from "@/components/BodyViewer/BodyViewer";
import { KvTable, Placeholder, type Kv } from "@/components/Inspector/KvTable";
import { RawView } from "@/components/Inspector/RawView";
import { RuleBadges } from "@/components/Inspector/RuleBadges";
import { SsePanel } from "@/components/Inspector/SsePanel";
import { TimingView } from "@/components/Inspector/TimingView";
import { TlsView } from "@/components/Inspector/TlsView";
import { WebSocketPanel } from "@/components/Inspector/WebSocketPanel";
import { FilterBySub } from "@/components/menus/filter";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { addSslBypass, openRewrite, setInsecureUpstream } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { explainFailure, type FailureHelp } from "@/lib/errors";
import { flowUrl, formatDuration, formatSize } from "@/lib/flow";
import { displayHeaderName, queryPairs } from "@/lib/http";
import { flowKey, sslSettingsKey } from "@/lib/query";
import { streamKind } from "@/lib/stream";
import { useUi } from "@/store/ui";

interface Inspected {
  flow: FlowSummary_Serialize;
  /** Null for synthetic flows, which have no recorded messages. */
  detail: FlowDetail_Serialize | null;
}

/** Inspector for the selected flow: URL bar over a Request | Response split. */
export function DetailPane() {
  const id = useUi((s) => s.selectedId);
  const store = useUi((s) => s.store);
  const { data } = useQuery({
    queryKey: flowKey(store, id),
    enabled: id !== null,
    queryFn: async (): Promise<Inspected | null> => {
      const [flow, detail] = await Promise.all([backend.getFlow(store, id!), backend.getFlowDetail(store, id!)]);
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
  const stream = streamKind(flow, detail);
  const messages = (
    <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
      <ResizablePanel defaultSize="50" minSize={240}>
        {detail ? <RequestPane flow={flow} detail={detail} /> : <NoDetail />}
      </ResizablePanel>
      <ResizableHandle />
      <ResizablePanel defaultSize="50" minSize={240}>
        {detail ? <ResponsePane flow={flow} detail={detail} /> : <NoDetail />}
      </ResizablePanel>
    </ResizablePanelGroup>
  );
  return (
    <div className="bg-background flex h-full flex-col" data-testid="inspector">
      <UrlBarMenu flow={flow} detail={detail}>
        <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2" data-selectable data-testid="url-bar">
          <MethodPill method={flow.method} />
          <UrlText row={flow} operation={flow.graphql_op} sentBy={flow.sent_by} className="min-w-0 flex-1 text-xs" />
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
          {detail && flow.method !== "CONNECT" && stream !== "websocket" && (
            <Button
              variant="ghost"
              size="xs"
              title="Edit this flow and save the changes as a rule"
              onClick={() => void openRewrite(store, flow.id)}
              data-testid="inspector-rewrite"
            >
              <PencilLine />
              Rewrite…
            </Button>
          )}
          {(flow.tls === "intercepted" || flow.tls === "likely_pinned") && <BypassButton host={flow.host} compact />}
        </div>
      </UrlBarMenu>
      {detail && <RuleBadges rules={detail.rules} />}
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
          <CertWizardButton />
          <BypassButton host={flow.host} />
        </div>
      )}
      {stream === null ? (
        messages
      ) : (
        // WebSocket frames and SSE events get the lower part of the inspector.
        <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
          <ResizablePanel defaultSize="40" minSize={96} className="flex flex-col">
            {messages}
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel defaultSize="60" minSize={120}>
            {stream === "websocket" ? <WebSocketPanel key={flow.id} id={flow.id} /> : <SsePanel key={flow.id} id={flow.id} />}
          </ResizablePanel>
        </ResizablePanelGroup>
      )}
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
      <ScrollTab value="Header">
        <KvTable rows={headers} kind="header" side="request" testId="request-headers" />
      </ScrollTab>
      <ScrollTab value="Query">
        {query.length ? <KvTable rows={query} kind="query" testId="query-params" /> : <Placeholder text="No query parameters" />}
      </ScrollTab>
      <FillTab value="Body">
        <BodyViewer
          key={`${flow.id}-request`}
          id={flow.id}
          part="request"
          info={info}
          finished={finished}
          headers={request.headers}
          url={request.url}
        />
      </FillTab>
      <FillTab value="Raw">
        <RawView
          id={flow.id}
          part="request"
          startLine={`${request.method} ${request.url} ${request.version}`}
          headers={request.headers}
          info={info}
          finished={finished}
        />
      </FillTab>
    </MessageTabs>
  );
}

function ResponsePane({ flow, detail }: Inspected & { detail: FlowDetail_Serialize }) {
  const tab = useUi((s) => s.responseTab);
  const setTab = useUi((s) => s.setResponseTab);
  const { response, response_body: info, error } = detail;
  // A 101 has no body: the connection carries WebSocket messages instead.
  const finished = flow.state !== "active" || response?.status === 101;
  // A pinning client gets its own banner above both panes.
  const help = flow.tls === "likely_pinned" ? null : explainFailure(flow, detail);
  const noResponse = <Placeholder text={error ? "No response" : "Waiting for the response…"} />;
  return (
    <MessageTabs title="Response" tabs={["Header", "Body", "Raw", "TLS", "Timing"]} value={tab} onChange={setTab}>
      {help && <FailureBanner help={help} host={flow.host} />}
      {flow.tls === "passthrough" && (
        <div className="text-muted-foreground border-b px-2 py-1.5 text-xs" data-testid="tunnel-note">
          Not decrypted: {flow.host} is in the SSL exclude or bypass list. {formatSize(flow.req_size)} sent,{" "}
          {formatSize(flow.resp_size)} received through the tunnel.
        </div>
      )}
      <ScrollTab value="Header">
        {response ? (
          <KvTable
            rows={response.headers.map((h) => [displayHeaderName(h.name), h.value])}
            kind="header"
            side="response"
            testId="response-headers"
          />
        ) : (
          noResponse
        )}
      </ScrollTab>
      <FillTab value="Body">
        {response ? (
          <BodyViewer
            key={`${flow.id}-response`}
            id={flow.id}
            part="response"
            info={info}
            finished={finished}
            headers={response.headers}
            url={detail.request.url}
          />
        ) : (
          noResponse
        )}
      </FillTab>
      <FillTab value="Raw">
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
      </FillTab>
      <ScrollTab value="TLS">
        <TlsView flow={flow} detail={detail} />
      </ScrollTab>
      <ScrollTab value="Timing">
        <TimingView flow={flow} timing={detail.timing} />
      </ScrollTab>
    </MessageTabs>
  );
}

/**
 * The URL bar's menu: the URL and its parts, method, status, cURL, and
 * filters for this flow's host, method, status and app.
 */
function UrlBarMenu({ flow, detail, children }: Inspected & { children: ReactElement }) {
  const store = useUi((s) => s.store);
  const status = detail?.response ? `${detail.response.status} ${detail.response.reason}`.trim() : flow.status !== null ? String(flow.status) : null;
  const pathQuery = `${flow.path}${flow.query ? `?${flow.query}` : ""}`;
  return (
    <CopyMenu
      testId="url-bar-menu"
      items={[
        { label: "Copy URL", text: detail?.request.url ?? flowUrl(flow), what: "URL", testId: "url-copy-url" },
        { label: "Copy host", text: flow.host, what: "host", testId: "url-copy-host" },
        { label: "Copy path", text: flow.path, what: "path" },
        flow.query !== "" && { label: "Copy path and query", text: pathQuery, what: "path and query" },
        { label: "Copy method", text: flow.method, what: "method" },
        status !== null && { label: "Copy status", text: status, what: "status", testId: "url-copy-status" },
        flow.method !== "CONNECT" && { label: "Copy as cURL", text: () => backend.getCurl(store, flow.id), what: "cURL command" },
      ]}
      extra={<FilterBySub facts={{ host: flow.host, method: flow.method, status: flow.status, client: flow.client }} />}
    >
      {children}
    </CopyMenu>
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

/** Why the flow failed, in a line, with the fixes that apply. */
function FailureBanner({ help, host }: { help: FailureHelp; host: string }) {
  const openSettings = useUi((s) => s.openSettings);
  return (
    <div className="flex items-start gap-2 border-b bg-[var(--s-5xx-bg)] px-2 py-1.5 text-xs" role="alert" data-testid="failure-banner">
      <CircleAlert className="mt-px size-3.5 shrink-0 text-[var(--s-failed)]" />
      <CopyMenu
        items={[
          { label: "Copy error", text: help.detail, what: "error" },
          { label: "Copy title and error", text: `${help.title}\n${help.detail}`, what: "error" },
        ]}
      >
        <div className="min-w-0 flex-1" data-selectable>
          <div className="font-medium text-[var(--s-failed)]">{help.title}</div>
          <div className="text-muted-foreground leading-4">{help.detail}</div>
        </div>
      </CopyMenu>
      <div className="flex shrink-0 flex-wrap justify-end gap-1">
        {help.actions.map((action) => {
          switch (action) {
            case "cert-wizard":
              return <CertWizardButton key={action} />;
            case "bypass":
              return <BypassButton key={action} host={host} compact />;
            case "insecure-upstream":
              return <InsecureUpstreamButton key={action} host={host} />;
            case "timeouts":
              return (
                <Button key={action} variant="outline" size="xs" className="text-foreground" onClick={() => openSettings("advanced")}>
                  Timeouts…
                </Button>
              );
            case "tls-settings":
              return (
                <Button key={action} variant="outline" size="xs" className="text-foreground" onClick={() => openSettings("ssl")}>
                  SSL Proxying…
                </Button>
              );
          }
        })}
      </div>
    </div>
  );
}

/** The device may simply not trust the CA yet: the wizard checks and installs it. */
function CertWizardButton() {
  const openWizard = useUi((s) => s.setCertWizardOpen);
  return (
    <Button variant="outline" size="xs" className="text-foreground shrink-0" onClick={() => openWizard(true)}>
      Certificate wizard…
    </Button>
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
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
    </Tabs>
  );
}

/** A tab whose content scrolls as a whole (headers, TLS, timing). */
function ScrollTab({ value, children }: { value: string; children: ReactNode }) {
  return (
    <TabsContent value={value} className="min-h-0 flex-1">
      <ScrollArea className="h-full">{children}</ScrollArea>
    </TabsContent>
  );
}

/** A tab that fills the pane and scrolls itself (body viewers, Raw). */
function FillTab({ value, children }: { value: string; children: ReactNode }) {
  return (
    <TabsContent value={value} className="min-h-0 flex-1">
      {children}
    </TabsContent>
  );
}

function NoDetail() {
  return <Placeholder text="Synthetic flow: no recorded messages" />;
}
