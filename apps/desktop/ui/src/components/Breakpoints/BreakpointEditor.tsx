import { useQuery } from "@tanstack/react-query";
import { CirclePlay, OctagonPause, Plus, Reply, X, XCircle } from "lucide-react";
import { useEffect, useState } from "react";

import type { BreakpointInfo_Serialize, Header, MessageEdits_Deserialize } from "@/bindings";
import { MethodPill } from "@/components/FlowTable/cells";
import { NumberInput, TextArea } from "@/components/Rules/fields";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { bridge } from "@/dev/bridge";
import { backend } from "@/lib/backend";
import { cleanHeaders } from "@/lib/rules";
import { cn } from "@/lib/utils";
import { useBreakpoints } from "@/store/breakpoints";
import { useUi } from "@/store/ui";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The Breakpoint Editor: the queue of paused flows and the selected one's
 * message, editable before it goes on. Execute sends it (with any edits),
 * Abort closes the connection, Respond answers the client directly. The
 * first resolution wins, whether from here or from an agent.
 */
export function BreakpointEditor() {
  const open = useUi((s) => s.breakpointsOpen);
  const setOpen = useUi((s) => s.setBreakpointsOpen);
  const queue = useBreakpoints((s) => s.queue);
  const selectedId = useUi((s) => s.selectedBreakpoint);
  const select = useUi((s) => s.selectBreakpoint);
  const selected = queue.find((b) => b.id === selectedId) ?? queue[0] ?? null;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        className="flex h-[80vh] flex-col gap-0 p-0 sm:max-w-[1080px]"
        onOpenAutoFocus={(e) => e.preventDefault()}
        data-testid="breakpoint-editor"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <OctagonPause className="size-4 text-[var(--s-paused)]" />
            Breakpoints
          </DialogTitle>
          <DialogDescription>
            Paused flows wait here until you execute, abort or answer them, or until they time out and continue
            unmodified.
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          {/* A plain scroller: ScrollArea's table layout defeats truncation. */}
          <div className="bg-muted/20 w-64 shrink-0 overflow-y-auto border-r">
            <div className="flex flex-col p-1" data-testid="breakpoint-queue">
              {queue.length === 0 && <div className="text-muted-foreground p-2 text-xs">Nothing is paused.</div>}
              {queue.map((bp) => (
                <QueueItem key={bp.id} bp={bp} selected={bp.id === selected?.id} onSelect={() => select(bp.id)} />
              ))}
            </div>
          </div>
          <div className="min-w-0 flex-1">
            {selected ? (
              <BreakpointForm key={selected.id} bp={selected} />
            ) : (
              <div className="text-muted-foreground flex h-full items-center justify-center text-xs">
                Add a Breakpoint action to a rule (or use “Breakpoint…” on a flow) to pause matching flows.
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function QueueItem({ bp, selected, onSelect }: { bp: BreakpointInfo_Serialize; selected: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={selected}
      className="hover:bg-accent data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground flex w-full min-w-0 flex-col gap-0.5 rounded-sm px-2 py-1 text-left text-xs"
    >
      <div className="flex items-center gap-1.5">
        <PhasePill phase={bp.phase} />
        <MethodPill method={bp.request.method} />
        <span className="ml-auto font-mono text-2xs opacity-70">{bp.flow_id !== null ? `#${bp.flow_id + 1}` : ""}</span>
      </div>
      <span className="truncate font-mono text-2xs">{bp.request.url}</span>
      <span className="flex items-center gap-1 text-2xs opacity-70">
        <span className="truncate">{bp.rule_name}</span>
        <Countdown deadline={bp.deadline_ms} />
      </span>
    </button>
  );
}

function PhasePill({ phase }: { phase: "request" | "response" }) {
  return (
    <span
      className="inline-flex h-4 items-center rounded-sm px-1 text-2xs font-semibold"
      style={{ color: "var(--s-paused)", backgroundColor: "var(--s-paused-bg)" }}
    >
      {phase === "request" ? "Request" : "Response"}
    </span>
  );
}

/** Seconds until the flow continues on its own. */
function Countdown({ deadline }: { deadline: number | null }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [deadline]);
  if (deadline === null) return <span className="ml-auto shrink-0">no timeout</span>;
  const left = Math.max(0, Math.round((deadline - now) / 1000));
  return (
    <span className="ml-auto shrink-0 font-mono tabular-nums" title="Continues unmodified when this runs out">
      {Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")}
    </span>
  );
}

function BreakpointForm({ bp }: { bp: BreakpointInfo_Serialize }) {
  const isRequest = bp.phase === "request";
  const original = isRequest ? bp.request.headers : (bp.response?.headers ?? []);
  const [method, setMethod] = useState(bp.request.method);
  const [url, setUrl] = useState(bp.request.url);
  const [status, setStatus] = useState<number | null>(bp.response?.status ?? 200);
  const [headers, setHeaders] = useState<Header[]>(original);
  const [body, setBody] = useState<string | null>(null);
  const [responding, setResponding] = useState(false);
  const [answer, setAnswer] = useState({ status: 200 as number | null, headers: [{ name: "content-type", value: "application/json" }] as Header[], body: "" });
  const [error, setError] = useState<string | null>(null);
  const { data: originalBody = "" } = useQuery({
    queryKey: ["breakpoint-body", bp.id],
    queryFn: async () => new TextDecoder().decode(await backend.getBreakpointBody(bp.id)),
    staleTime: Infinity,
    gcTime: 0,
  });
  const bodyText = body ?? originalBody;

  const edits = (): MessageEdits_Deserialize => {
    const out: MessageEdits_Deserialize = {};
    if (isRequest && method !== bp.request.method) out.method = method;
    if (isRequest && url !== bp.request.url) out.url = url;
    if (!isRequest && status !== null && status !== bp.response?.status) out.status = status;
    if (JSON.stringify(cleanHeaders(headers)) !== JSON.stringify(original)) out.headers = cleanHeaders(headers);
    if (bp.body.editable && body !== null && body !== originalBody) out.body = body;
    return out;
  };

  const resolve = async (resolution: Parameters<typeof backend.resolveBreakpoint>[1]) => {
    setError(null);
    try {
      await backend.resolveBreakpoint(bp.id, resolution);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const execute = (extra: MessageEdits_Deserialize | null = null) =>
    resolve({ action: "continue", edits: extra ? { ...edits(), ...extra } : edits() });
  const abort = () => resolve({ action: "abort" });
  const respond = () =>
    resolve({ action: "respond", status: answer.status ?? 200, headers: cleanHeaders(answer.headers), body: answer.body });

  // Lets dev/remote.ts press Execute and Abort in the real app.
  useEffect(() => {
    bridge.executeBreakpoint = execute;
    bridge.abortBreakpoint = abort;
    return () => {
      bridge.executeBreakpoint = undefined;
      bridge.abortBreakpoint = undefined;
    };
  });

  return (
    <div className="flex h-full flex-col" data-testid="breakpoint-form">
      <div className="flex items-center gap-2 border-b px-3 py-2 text-xs">
        <PhasePill phase={bp.phase} />
        <span className="truncate font-medium">{bp.rule_name}</span>
        <span className="text-muted-foreground ml-auto flex items-center gap-1">
          continues in <Countdown deadline={bp.deadline_ms} />
        </span>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-3 p-3 text-xs">
          {isRequest ? (
            <div className="flex items-center gap-1.5">
              <Input value={method} onChange={(e) => setMethod(e.target.value.toUpperCase())} className="h-6 w-24 font-mono text-xs" aria-label="Method" />
              <Input value={url} onChange={(e) => setUrl(e.target.value)} className="h-6 font-mono text-xs" aria-label="URL" data-testid="bp-url" />
            </div>
          ) : (
            <>
              <div className="text-muted-foreground truncate font-mono">
                {bp.request.method} {bp.request.url}
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Status</span>
                <NumberInput value={status} onChange={setStatus} min={100} max={999} className="w-20" />
              </div>
            </>
          )}
          <HeaderTable rows={headers} onChange={setHeaders} />
          <div className="flex flex-col gap-1">
            <span className="text-muted-foreground font-semibold">
              Body{bp.body.size ? ` · ${bp.body.size.toLocaleString()} bytes` : ""}
            </span>
            {bp.body.note && <span className="text-muted-foreground">{bp.body.note}</span>}
            <TextArea
              value={bodyText}
              readOnly={!bp.body.editable}
              onChange={(e) => setBody(e.target.value)}
              rows={12}
              className={cn(!bp.body.editable && "opacity-70")}
              data-testid="bp-body"
            />
          </div>
          {responding && (
            <div className="flex flex-col gap-1.5 rounded-md border p-2" data-testid="bp-respond-form">
              <span className="font-semibold">Answer the client with</span>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Status</span>
                <NumberInput value={answer.status} onChange={(s) => setAnswer({ ...answer, status: s })} min={100} max={999} className="w-20" />
              </div>
              <HeaderTable rows={answer.headers} onChange={(h) => setAnswer({ ...answer, headers: h })} />
              <TextArea value={answer.body} rows={6} onChange={(e) => setAnswer({ ...answer, body: e.target.value })} />
            </div>
          )}
        </div>
      </ScrollArea>
      <div className="flex items-center gap-2 border-t px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--s-failed)]">{error}</span>
        <Button variant="ghost" size="sm" onClick={() => void abort()} data-testid="bp-abort">
          <XCircle />
          Abort
        </Button>
        {responding ? (
          <Button variant="outline" size="sm" onClick={() => void respond()} data-testid="bp-respond-send">
            <Reply />
            Send response
          </Button>
        ) : (
          <Button variant="outline" size="sm" onClick={() => setResponding(true)} data-testid="bp-respond">
            <Reply />
            Respond…
          </Button>
        )}
        <Button size="sm" onClick={() => void execute()} data-testid="bp-execute">
          <CirclePlay />
          Execute
        </Button>
      </div>
    </div>
  );
}

function HeaderTable({ rows, onChange }: { rows: Header[]; onChange: (rows: Header[]) => void }) {
  const update = (i: number, patch: Partial<Header>) => onChange(rows.map((r, j) => (i === j ? { ...r, ...patch } : r)));
  return (
    <div className="flex flex-col gap-0.5" data-testid="header-table">
      <span className="text-muted-foreground font-semibold">Headers</span>
      {rows.map((h, i) => (
        <div key={i} className="flex items-center gap-1">
          <Input value={h.name} onChange={(e) => update(i, { name: e.target.value })} className="h-6 w-56 shrink-0 font-mono text-xs" />
          <Input value={h.value} onChange={(e) => update(i, { value: e.target.value })} className="h-6 font-mono text-xs" />
          <Button variant="ghost" size="icon-xs" aria-label="Remove header" onClick={() => onChange(rows.filter((_, j) => j !== i))}>
            <X />
          </Button>
        </div>
      ))}
      <Button variant="ghost" size="xs" className="text-muted-foreground self-start" onClick={() => onChange([...rows, { name: "", value: "" }])}>
        <Plus />
        Header
      </Button>
    </div>
  );
}
