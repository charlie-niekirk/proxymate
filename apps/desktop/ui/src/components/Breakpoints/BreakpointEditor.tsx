import { useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, CirclePlay, OctagonPause, Plus, Reply, Trash2, X, XCircle } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactElement } from "react";

import type { BreakpointInfo_Serialize, Header, MessageEdits_Deserialize } from "@/bindings";
import { CodeEditor } from "@/components/Compose/CodeEditor";
import { keepEscapeInEditor } from "@/components/JsonEditor/escape";
import { JsonEditor } from "@/components/JsonEditor/JsonEditor";
import { MethodPill } from "@/components/FlowTable/cells";
import { NumberInput } from "@/components/Rules/fields";
import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { bridge } from "@/dev/bridge";
import { reportError, resolveBreakpoint } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { languageFor } from "@/lib/compose";
import { base64, hexBytes } from "@/lib/copyFormats";
import { bodyText, headerValue } from "@/lib/http";
import { parseHex } from "@/lib/stream";
import { cleanHeaders } from "@/lib/rules";
import { cn } from "@/lib/utils";
import { useUndoable } from "@/hooks/useUndoable";
import { align, parseDoc, toJson, type JNode } from "@/lib/json/doc";
import { useBreakpoints } from "@/store/breakpoints";
import { useUi } from "@/store/ui";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The Breakpoint Editor: the queue of paused flows and the selected one's
 * message, editable before it goes on. Execute sends it (with any edits),
 * Abort closes the connection, Respond answers the client directly. A held
 * WebSocket message (M19) is edited as text, a JSON tree or hex, then sent
 * on or dropped. The first resolution wins, whether from here or from an
 * agent.
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
        onEscapeKeyDown={keepEscapeInEditor}
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
            {selected?.message ? (
              <MessageForm key={selected.id} bp={selected} />
            ) : selected ? (
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

/**
 * A paused flow's menu: copy its URL, show its flow or rule, or let it go
 * without the form (continue unmodified, abort).
 */
function QueueMenu({ bp, children }: { bp: BreakpointInfo_Serialize; children: ReactElement }) {
  const select = useUi((s) => s.select);
  const openRules = useUi((s) => s.openRulesEditor);
  const resolve = (resolution: Parameters<typeof resolveBreakpoint>[1], what: string) =>
    void resolveBreakpoint(bp.id, resolution).catch((e: unknown) => reportError(what, e));
  return (
    <CopyMenu
      testId="breakpoint-menu"
      items={[
        { label: "Copy URL", text: bp.request.url, what: "URL" },
        { label: "Copy method", text: bp.request.method, what: "method" },
      ]}
      extra={
        <>
          {bp.flow_id !== null && <ContextMenuItem onSelect={() => select(bp.flow_id)}>Show flow #{bp.flow_id + 1}</ContextMenuItem>}
          <ContextMenuItem onSelect={() => openRules({ ruleId: bp.rule_id })}>Open rule “{bp.rule_name}”</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => resolve({ action: "continue", edits: {} }, "Couldn't continue the flow")}>
            Continue unmodified
          </ContextMenuItem>
          <ContextMenuItem variant="destructive" onSelect={() => resolve({ action: "abort" }, "Couldn't abort the flow")}>
            {bp.message ? "Drop the message" : "Abort"}
          </ContextMenuItem>
        </>
      }
    >
      {children}
    </CopyMenu>
  );
}

function QueueItem({ bp, selected, onSelect }: { bp: BreakpointInfo_Serialize; selected: boolean; onSelect: () => void }) {
  return (
    <QueueMenu bp={bp}>
      <button
        type="button"
        onClick={onSelect}
        data-selected={selected}
        className="hover:bg-accent data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground flex w-full min-w-0 flex-col gap-0.5 rounded-sm px-2 py-1 text-left text-xs"
      >
        <div className="flex items-center gap-1.5">
          <PhasePill phase={bp.phase} />
          {bp.message ? <MessagePill direction={bp.message.direction} /> : <MethodPill method={bp.request.method} />}
          <span className="ml-auto font-mono text-2xs opacity-70">{bp.flow_id !== null ? `#${bp.flow_id + 1}` : ""}</span>
        </div>
        <span className="truncate font-mono text-2xs">{bp.request.url}</span>
        <span className="flex items-center gap-1 text-2xs opacity-70">
          <span className="truncate">{bp.rule_name}</span>
          <Countdown deadline={bp.deadline_ms} />
        </span>
      </button>
    </QueueMenu>
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

function MessagePill({ direction }: { direction: "sent" | "received" }) {
  return (
    <span className="text-muted-foreground inline-flex items-center gap-0.5 text-2xs font-semibold">
      {direction === "sent" ? (
        <ArrowUp className="size-3 text-[var(--ws-sent)]" />
      ) : (
        <ArrowDown className="size-3 text-[var(--ws-received)]" />
      )}
      WebSocket
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
  const { data: originalBody = "", isPending: bodyPending } = useQuery({
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

  const resolve = async (resolution: Parameters<typeof resolveBreakpoint>[1]) => {
    setError(null);
    try {
      await resolveBreakpoint(bp.id, resolution);
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
            <BodyEditor
              // The editor is seeded once, so it waits for the paused body.
              loading={bodyPending}
              value={bodyText}
              original={originalBody}
              headers={headers}
              readOnly={!bp.body.editable}
              onChange={setBody}
              className="h-60"
              testId="bp-body"
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
              <BodyEditor
                value={answer.body}
                headers={answer.headers}
                onChange={(text) => setAnswer((a) => ({ ...a, body: text }))}
                className="h-32"
                testId="bp-respond-body"
              />
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

type MessageMode = "text" | "tree" | "hex";

/**
 * A held WebSocket message: its payload as text, a JSON tree or hex. Send
 * goes on with any edits (hex edits as bytes), Drop never forwards it.
 * Later messages in its direction wait until it is resolved.
 */
function MessageForm({ bp }: { bp: BreakpointInfo_Serialize }) {
  const held = bp.message!;
  const { data: original, isPending } = useQuery({
    queryKey: ["breakpoint-body", bp.id],
    queryFn: async () => new Uint8Array(await backend.getBreakpointBody(bp.id)),
    staleTime: Infinity,
    gcTime: 0,
  });
  const originalText = useMemo(() => (original ? bodyText(original) : null), [original]);
  const isJson = originalText !== null && parsed(originalText) !== null;
  const [mode, setMode] = useState<MessageMode | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [hex, setHex] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Remounts the text editors from the current text after a mode switch.
  const [revision, setRevision] = useState(0);
  // Binary that isn't UTF-8 can only be edited as hex.
  const shown: MessageMode = mode ?? (originalText === null || held.opcode === "binary" ? "hex" : isJson ? "tree" : "text");
  const currentText = text ?? originalText ?? "";
  const currentHex = hex ?? hexBytes(original ?? new Uint8Array());

  const switchTo = (next: MessageMode) => {
    setError(null);
    let nextText = text ?? originalText ?? "";
    if (next === "hex" && shown !== "hex") {
      setHex(hexBytes(text === null ? (original ?? new Uint8Array()) : new TextEncoder().encode(text)));
    }
    if (next !== "hex" && shown === "hex" && hex !== null) {
      const bytes = parseHex(hex);
      const decoded = bytes ? bodyText(bytes) : null;
      if (decoded === null) return setError("The bytes aren't UTF-8 text: keep editing them as hex.");
      nextText = decoded;
      setText(decoded);
    }
    if (next === "tree" && parsed(nextText) === null) return setError("The text isn't valid JSON.");
    setRevision((r) => r + 1);
    setMode(next);
  };

  const edits = (): MessageEdits_Deserialize | string => {
    if (!bp.body.editable) return {};
    if (shown === "hex") {
      if (hex === null) return {};
      const bytes = parseHex(hex);
      if (bytes === null) return "That isn't hex: use pairs of digits like 0a ff 3c.";
      return original && hexBytes(bytes) === hexBytes(original) ? {} : { body_base64: base64(bytes) };
    }
    return text === null || text === originalText ? {} : { body: text };
  };
  const resolve = async (resolution: Parameters<typeof resolveBreakpoint>[1]) => {
    setError(null);
    try {
      await resolveBreakpoint(bp.id, resolution);
    } catch (e) {
      setError(errorText(e));
    }
  };
  // `extra` (dev/remote.ts) is merged over the form's edits.
  const send = async (extra: MessageEdits_Deserialize | null = null) => {
    const e = edits();
    if (typeof e === "string") return setError(e);
    await resolve({ action: "continue", edits: extra ? { ...e, ...extra } : e });
  };
  const drop = () => resolve({ action: "abort" });

  useEffect(() => {
    bridge.executeBreakpoint = send;
    bridge.abortBreakpoint = drop;
    return () => {
      bridge.executeBreakpoint = undefined;
      bridge.abortBreakpoint = undefined;
    };
  });

  const modes: MessageMode[] = originalText === null ? ["hex"] : isJson ? ["text", "tree", "hex"] : ["text", "hex"];
  return (
    <div className="flex h-full flex-col" data-testid="message-form">
      <div className="flex items-center gap-2 border-b px-3 py-2 text-xs">
        <PhasePill phase={bp.phase} />
        <MessagePill direction={held.direction} />
        <span className="truncate font-medium">{bp.rule_name}</span>
        <span className="text-muted-foreground ml-auto flex items-center gap-1">
          continues in <Countdown deadline={bp.deadline_ms} />
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-2 p-3 text-xs">
        <div className="text-muted-foreground truncate font-mono" title={bp.request.url}>
          {bp.request.url}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-muted-foreground font-semibold">
            {held.direction === "sent" ? "Sent by the client" : "From the server"} · {held.opcode === "text" ? "Text" : "Binary"} ·{" "}
            {bp.body.size.toLocaleString()} bytes
          </span>
          <span className="text-muted-foreground">Later messages this way wait behind it.</span>
          <div className="ml-auto flex items-center gap-1 text-[11px]" data-testid="bp-message-mode">
            {modes.map((m) => (
              <button
                key={m}
                type="button"
                data-active={shown === m}
                onClick={() => switchTo(m)}
                className="text-muted-foreground data-[active=true]:bg-accent data-[active=true]:text-foreground h-5 rounded-sm px-1.5"
                data-testid={`bp-message-${m}`}
              >
                {m === "text" ? "Text" : m === "tree" ? "Tree" : "Hex"}
              </button>
            ))}
          </div>
        </div>
        {bp.body.note && <span className="text-muted-foreground">{bp.body.note}</span>}
        <div className="min-h-0 flex-1 overflow-hidden rounded-md border">
          {isPending ? null : shown === "hex" ? (
            <textarea
              value={currentHex}
              onChange={(e) => setHex(e.target.value)}
              readOnly={!bp.body.editable}
              spellCheck={false}
              className="bg-background h-full w-full resize-none p-2 font-mono text-xs outline-none"
              data-testid="bp-message-hex-input"
            />
          ) : shown === "tree" && originalText !== null && parsed(currentText) ? (
            <TreeBody key={revision} initial={parsed(currentText)!} original={originalText} onChange={setText} testId="bp-message-json" />
          ) : (
            <CodeEditor
              key={revision}
              value={currentText}
              language={isJson ? "json" : "text"}
              onChange={setText}
              readOnly={!bp.body.editable}
              testId="bp-message-text"
            />
          )}
        </div>
      </div>
      <div className="flex items-center gap-2 border-t px-3 py-2">
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--s-failed)]">{error}</span>
        <Button variant="ghost" size="sm" onClick={() => void drop()} data-testid="bp-drop">
          <Trash2 />
          Drop
        </Button>
        <Button size="sm" onClick={() => void send()} data-testid="bp-send">
          <CirclePlay />
          Send
        </Button>
      </div>
    </div>
  );
}

/** A JSON document, or null if `text` isn't JSON. */
function parsed(text: string): JNode | null {
  try {
    return parseDoc(text);
  } catch {
    return null;
  }
}

/**
 * A message body in CodeMirror, highlighted for its (live) Content-Type.
 * A JSON body can also be edited as a tree (`original` tints the changes);
 * the text follows every tree edit.
 */
function BodyEditor({
  value,
  original,
  headers,
  onChange,
  loading = false,
  readOnly = false,
  className,
  testId,
}: {
  value: string;
  original?: string;
  headers: Header[];
  onChange: (text: string) => void;
  loading?: boolean;
  readOnly?: boolean;
  className: string;
  testId: string;
}) {
  const language = languageFor(headerValue(headers, "content-type")?.trim() ?? null);
  // The tree starts from the text as it is when it opens.
  const [tree, setTree] = useState<JNode | null>(null);
  const [invalid, setInvalid] = useState(false);
  const canTree = useMemo(() => original !== undefined && !readOnly && parsed(original) !== null, [original, readOnly]);
  const mode = tree && canTree ? "tree" : "text";
  const setMode = (m: "text" | "tree") => {
    if (m === "text") return setTree(null);
    const doc = parsed(value);
    setInvalid(doc === null);
    setTree(doc);
  };
  return (
    <div className="flex flex-col gap-1">
      {canTree && (
        <div className="flex items-center gap-1 self-end text-[11px]" data-testid={`${testId}-mode`}>
          {(["text", "tree"] as const).map((m) => (
            <button
              key={m}
              type="button"
              data-active={mode === m}
              onClick={() => setMode(m)}
              className="text-muted-foreground data-[active=true]:bg-accent data-[active=true]:text-foreground h-5 rounded-sm px-1.5"
              title={m === "tree" && invalid ? "The text isn't valid JSON" : undefined}
              data-testid={`${testId}-${m}`}
            >
              {m === "text" ? "Text" : "Tree"}
            </button>
          ))}
        </div>
      )}
      <div className={cn("overflow-hidden rounded-md border", className)}>
        {loading ? null : mode === "tree" && tree && original !== undefined ? (
          <TreeBody initial={tree} original={original} onChange={onChange} testId={`${testId}-json`} />
        ) : (
          // Remounted from the current text when coming back from the tree.
          <CodeEditor key={mode} value={value} language={language} onChange={onChange} readOnly={readOnly} testId={testId} />
        )}
      </div>
    </div>
  );
}

function TreeBody({ initial, original, onChange, testId }: { initial: JNode; original: string; onChange: (text: string) => void; testId: string }) {
  const reference = useMemo(() => parsed(original), [original]);
  // Shares the original's ids where they line up, so only real edits are tinted.
  const tree = useUndoable(() => (reference ? align(reference, initial) : initial));
  const pretty = /\n/.test(original.trim());
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    onChange(toJson(tree.value, pretty ? 2 : 0));
  }, [tree.value, pretty, onChange]);
  return <JsonEditor doc={tree} original={reference} testId={testId} />;
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
