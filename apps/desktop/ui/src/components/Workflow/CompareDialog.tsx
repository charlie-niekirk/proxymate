import { useQuery } from "@tanstack/react-query";
import { Columns2 } from "lucide-react";
import { useMemo, useState } from "react";

import type { BodyPart, FlowDetail_Serialize, FlowSummary_Serialize } from "@/bindings";
import { MethodPill, StatusPill } from "@/components/FlowTable/cells";
import { Lines } from "@/components/Rewrite/Preview";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { backend } from "@/lib/backend";
import { changedCount, diffHeaders, type KvDiffRow } from "@/lib/compare";
import { flowUrl, formatMs, formatSize } from "@/lib/flow";
import { bodyText } from "@/lib/http";
import { flowKey } from "@/lib/query";
import { lineDiff, prettyForDiff } from "@/lib/rewrite/lines";
import { cn } from "@/lib/utils";
import { useUi, type FlowRef } from "@/store/ui";

/** Bodies are compared up to this size. */
const BODY_MAX = 2 * 1024 * 1024;

/**
 * Compare (M17): two flows, from any tabs, side by side: the request line,
 * status, headers as a key/value diff (volatile ones optionally hidden),
 * bodies as a line diff of their pretty-printed text, and timing.
 */
export function CompareDialog() {
  const { open, a, b } = useUi((s) => s.compare);
  const close = useUi((s) => s.closeCompare);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="flex h-[86vh] flex-col gap-0 p-0 sm:max-w-[1180px]" data-testid="compare">
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Columns2 className="size-4" />
            Compare
          </DialogTitle>
          <DialogDescription>Left is the older flow. Headers are compared by name, bodies line by line.</DialogDescription>
        </DialogHeader>
        {open && a && b && <Comparison a={a} b={b} />}
      </DialogContent>
    </Dialog>
  );
}

interface Side {
  summary: FlowSummary_Serialize | null | undefined;
  detail: FlowDetail_Serialize | null | undefined;
}

function useSide(ref: FlowRef): Side {
  const { data: summary } = useQuery({ queryKey: [...flowKey(ref.store, ref.id), "summary"], queryFn: () => backend.getFlow(ref.store, ref.id) });
  const { data: detail } = useQuery({ queryKey: [...flowKey(ref.store, ref.id), "detail"], queryFn: () => backend.getFlowDetail(ref.store, ref.id) });
  return { summary, detail };
}

function useBody(ref: FlowRef, part: BodyPart, enabled: boolean) {
  return useQuery({
    queryKey: ["body", ref.store, ref.id, part, "compare"],
    queryFn: async () => {
      const bytes = new Uint8Array(await backend.getBody(ref.store, ref.id, part, true, BODY_MAX).catch(() => new ArrayBuffer(0)));
      return { size: bytes.length, text: bodyText(bytes) };
    },
    enabled,
  });
}

function Comparison({ a, b }: { a: FlowRef; b: FlowRef }) {
  const left = useSide(a);
  const right = useSide(b);
  const [tab, setTab] = useState("response-headers");
  const [ignoreVolatile, setIgnoreVolatile] = useState(true);
  const reqHeaders = useMemo(
    () => diffHeaders(left.detail?.request.headers ?? [], right.detail?.request.headers ?? [], ignoreVolatile),
    [left.detail, right.detail, ignoreVolatile],
  );
  const respHeaders = useMemo(
    () => diffHeaders(left.detail?.response?.headers ?? [], right.detail?.response?.headers ?? [], ignoreVolatile),
    [left.detail, right.detail, ignoreVolatile],
  );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="grid grid-cols-2 gap-px border-b bg-[var(--border)]">
        <FlowHeader side={left} label="A" testId="compare-a" />
        <FlowHeader side={right} label="B" testId="compare-b" />
      </div>
      <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col gap-0">
        <div className="flex items-center gap-2 border-b px-3 py-1.5">
          <TabsList className="h-7">
            <TabsTrigger value="request-headers" className="text-xs" data-testid="compare-tab-request-headers">
              Request Headers <Count n={changedCount(reqHeaders)} />
            </TabsTrigger>
            <TabsTrigger value="response-headers" className="text-xs" data-testid="compare-tab-response-headers">
              Response Headers <Count n={changedCount(respHeaders)} />
            </TabsTrigger>
            <TabsTrigger value="request-body" className="text-xs" data-testid="compare-tab-request-body">
              Request Body
            </TabsTrigger>
            <TabsTrigger value="response-body" className="text-xs" data-testid="compare-tab-response-body">
              Response Body
            </TabsTrigger>
            <TabsTrigger value="timing" className="text-xs" data-testid="compare-tab-timing">
              Timing
            </TabsTrigger>
          </TabsList>
          {tab.endsWith("headers") && (
            <label className="text-muted-foreground ml-auto flex items-center gap-2 text-xs">
              <Switch checked={ignoreVolatile} onCheckedChange={setIgnoreVolatile} data-testid="compare-ignore-volatile" />
              Ignore volatile headers (date, request ids, traces)
            </label>
          )}
        </div>
        <TabsContent value="request-headers" className="min-h-0 flex-1 overflow-auto">
          <HeaderDiff rows={reqHeaders} />
        </TabsContent>
        <TabsContent value="response-headers" className="min-h-0 flex-1 overflow-auto">
          <HeaderDiff rows={respHeaders} />
        </TabsContent>
        <TabsContent value="request-body" className="min-h-0 flex-1">
          <BodyDiff a={a} b={b} part="request" enabled={tab === "request-body"} />
        </TabsContent>
        <TabsContent value="response-body" className="min-h-0 flex-1">
          <BodyDiff a={a} b={b} part="response" enabled={tab === "response-body"} />
        </TabsContent>
        <TabsContent value="timing" className="min-h-0 flex-1 overflow-auto">
          <TimingDiff left={left} right={right} />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function Count({ n }: { n: number }) {
  if (n === 0) return null;
  return <span className="bg-[var(--diff-change-bg)] ml-1 rounded px-1 text-[10px] tabular-nums">{n}</span>;
}

function FlowHeader({ side, label, testId }: { side: Side; label: string; testId: string }) {
  const s = side.summary;
  return (
    <div className="bg-background flex min-w-0 items-center gap-2 px-3 py-2 text-xs" data-testid={testId}>
      <span className="text-muted-foreground font-semibold">{label}</span>
      {s ? (
        <>
          <MethodPill method={s.method} />
          <span className="min-w-0 flex-1 truncate font-mono" title={side.detail?.request.url ?? flowUrl(s)} data-selectable>
            {side.detail?.request.url ?? flowUrl(s)}
          </span>
          <StatusPill status={s.status} state={s.state} grpc={s.grpc_status} />
          <span className="text-muted-foreground tabular-nums">{formatMs(s.duration_ms)}</span>
        </>
      ) : (
        <span className="text-muted-foreground">{s === null ? "This flow is gone" : "Loading…"}</span>
      )}
    </div>
  );
}

function HeaderDiff({ rows }: { rows: KvDiffRow[] }) {
  if (rows.length === 0) return <p className="text-muted-foreground p-4 text-xs">No headers to compare.</p>;
  return (
    <table className="w-full table-fixed border-collapse font-mono text-[11.5px]" data-testid="compare-headers" data-selectable>
      <colgroup>
        <col className="w-56" />
        <col />
        <col />
      </colgroup>
      <tbody>
        {rows.map((r) => (
          <tr
            key={r.name.toLowerCase()}
            data-kind={r.kind}
            className={cn(
              "border-b border-border/50 align-top",
              "data-[kind=added]:bg-[var(--diff-add-bg)] data-[kind=removed]:bg-[var(--diff-del-bg)] data-[kind=changed]:bg-[var(--diff-change-bg)]",
            )}
          >
            <td className="truncate px-3 py-1 font-semibold" title={r.name}>
              {r.name}
            </td>
            <td className={cn("px-3 py-1 break-all", r.left === null && "text-muted-foreground")}>{r.left ?? "—"}</td>
            <td className={cn("px-3 py-1 break-all", r.right === null && "text-muted-foreground")}>{r.right ?? "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function BodyDiff({ a, b, part, enabled }: { a: FlowRef; b: FlowRef; part: BodyPart; enabled: boolean }) {
  const left = useBody(a, part, enabled);
  const right = useBody(b, part, enabled);
  const lines = useMemo(() => {
    if (!left.data || !right.data || left.data.text === null || right.data.text === null) return null;
    return lineDiff(prettyForDiff(left.data.text), prettyForDiff(right.data.text));
  }, [left.data, right.data]);
  if (!left.data || !right.data) return <p className="text-muted-foreground p-4 text-xs">Loading…</p>;
  if (left.data.size === 0 && right.data.size === 0) return <p className="text-muted-foreground p-4 text-xs">Neither flow has a {part} body.</p>;
  if (lines === null) {
    return (
      <p className="text-muted-foreground p-4 text-xs" data-testid="compare-binary">
        Binary bodies ({formatSize(left.data.size)} and {formatSize(right.data.size)}): compare them in the Hex view.
      </p>
    );
  }
  if (lines.every((l) => l.kind === "same")) {
    return (
      <p className="text-muted-foreground p-4 text-xs" data-testid="compare-same-body">
        The {part} bodies are the same ({formatSize(left.data.size)}).
      </p>
    );
  }
  return <Lines lines={lines} testId="compare-body" />;
}

function TimingDiff({ left, right }: { left: Side; right: Side }) {
  const rows: [string, (s: Side) => number | null | undefined][] = [
    ["DNS", (s) => s.detail?.timing.dns_ms],
    ["Connect", (s) => s.detail?.timing.connect_ms],
    ["TLS", (s) => s.detail?.timing.tls_ms],
    ["Waiting (TTFB)", (s) => s.detail?.timing.ttfb_ms],
    ["Receiving", (s) => s.detail?.timing.transfer_ms],
    ["Total", (s) => s.summary?.duration_ms],
  ];
  return (
    <table className="w-full max-w-2xl border-collapse text-xs" data-testid="compare-timing">
      <thead className="text-muted-foreground">
        <tr className="border-b">
          <th className="px-3 py-1.5 text-left font-medium">Phase</th>
          <th className="px-3 py-1.5 text-right font-medium">A</th>
          <th className="px-3 py-1.5 text-right font-medium">B</th>
          <th className="px-3 py-1.5 text-right font-medium">B − A</th>
        </tr>
      </thead>
      <tbody className="tabular-nums">
        {rows.map(([name, get]) => {
          const [x, y] = [get(left) ?? null, get(right) ?? null];
          const delta = x !== null && y !== null ? y - x : null;
          return (
            <tr key={name} className="border-b border-border/50">
              <td className="px-3 py-1">{name}</td>
              <td className="px-3 py-1 text-right">{formatMs(x)}</td>
              <td className="px-3 py-1 text-right">{formatMs(y)}</td>
              <td className="text-muted-foreground px-3 py-1 text-right">
                {delta === null ? "—" : `${delta > 0 ? "+" : delta < 0 ? "−" : ""}${formatMs(Math.abs(delta))}`}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
