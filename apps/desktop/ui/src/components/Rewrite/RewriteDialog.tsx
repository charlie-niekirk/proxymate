import { useQuery } from "@tanstack/react-query";
import { PencilLine, Save } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";

import type { Phase, RuleTestResult_Serialize } from "@/bindings";
import { languageFor } from "@/lib/compose";
import { Centered } from "@/components/BodyViewer/parts";
import { docInWorker, WORKER_THRESHOLD } from "@/components/BodyViewer/workers/client";
import { CodeEditor } from "@/components/Compose/CodeEditor";
import { MethodPill } from "@/components/FlowTable/cells";
import { keepEscapeInEditor } from "@/components/JsonEditor/escape";
import { JsonEditor } from "@/components/JsonEditor/JsonEditor";
import { NumberInput } from "@/components/Rules/fields";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Switch } from "@/components/ui/switch";
import { bridge } from "@/dev/bridge";
import { useUndoable, type Undoable } from "@/hooks/useUndoable";
import { saveRule } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { children, insert, newId, nodeAt, parseDoc, remove, rename, setValue, type JNode, type Path } from "@/lib/json/doc";
import { isVisual, parsePairs, READ_ONLY_HEADERS, type KvRow, type Pins } from "@/lib/rewrite/diff";
import { matcherFor, modeOf, type FlowTarget } from "@/lib/rewrite/matcher";
import {
  bodySource,
  buildActions,
  headerRows,
  initialEdits,
  MANY_ACTIONS,
  PREVIEW_LIMIT,
  TREE_MAX,
  rawQuery,
  rebuild,
  type BodyEdit,
  type BodySource,
  type Edits,
  type Source,
} from "@/lib/rewrite/session";
import { actionLabel, actionSummary, normalizeRule, type Action, type Rule } from "@/lib/rules";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

import { KvEditor } from "./KvEditor";
import { MatcherChips, type MatchChoice } from "./MatcherChips";
import { Preview } from "./Preview";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The first byte that isn't JSON whitespace. */
function firstByte(bytes: Uint8Array): number {
  for (const b of bytes) if (b !== 0x20 && b !== 0x0a && b !== 0x0d && b !== 0x09) return b;
  return 0;
}

interface Loaded {
  source: Source;
  /** The rule being edited: a saved one, or the Rewrite preset's draft. */
  base: Rule;
  initial: Edits;
  pins: Pins;
  kept: Action[];
  choice: MatchChoice;
  /** Why a saved rule's edits couldn't be rebuilt on this flow, if they couldn't. */
  note: string | null;
}

async function load(store: number, flow: number, rule: Rule | null): Promise<Loaded> {
  const [summary, detail, draft] = await Promise.all([
    backend.getFlow(store, flow),
    backend.getFlowDetail(store, flow),
    backend.draftRule(store, flow, "rewrite"),
  ]);
  if (!summary || !detail) throw new Error(`flow ${flow + 1} has no recorded messages`);
  const [reqBytes, resBytes] = await Promise.all(
    (["request", "response"] as const).map(async (part) => new Uint8Array(await backend.getBody(store, flow, part, true))),
  );
  // Large JSON bodies become documents off the main thread.
  const preparse = async (bytes: Uint8Array) => {
    if (bytes.length < WORKER_THRESHOLD || bytes.length > TREE_MAX || ![0x7b, 0x5b].includes(firstByte(bytes))) return null;
    return docInWorker(bytes).catch(() => null);
  };
  const [reqDoc, resDoc] = [await preparse(reqBytes!), await preparse(resBytes!)];
  const query = parsePairs(rawQuery(detail.request.url), newId);
  const contentType = (headers: { name: string; value: string }[]) => headers.find((h) => h.name.toLowerCase() === "content-type")?.value ?? null;
  const target: FlowTarget = {
    method: summary.method,
    scheme: summary.scheme,
    host: summary.host,
    port: summary.port,
    path: summary.path,
    query: query.map((r) => [r.name, r.value]),
  };
  const reqType = contentType(detail.request.headers);
  const resType = detail.response ? contentType(detail.response.headers) : null;
  const source: Source = {
    store,
    flow,
    target,
    url: detail.request.url,
    request: {
      headers: headerRows(detail.request.headers),
      query,
      body: bodySource(reqBytes!, reqType, detail.request_body.kind, "request", query, reqDoc),
      contentType: reqType,
    },
    response: detail.response && {
      status: detail.response.status,
      headers: headerRows(detail.response.headers),
      body: bodySource(resBytes!, resType, detail.response_body.kind, "response", [], resDoc),
      contentType: resType,
    },
  };
  if (!rule) {
    return {
      source,
      base: { ...normalizeRule(draft), matcher: matcherFor(target, "path", false) },
      initial: initialEdits(source),
      pins: new Set(),
      kept: [],
      choice: { mode: "path", anyMethod: false, matcher: matcherFor(target, "path", false) },
      note: null,
    };
  }
  const visual = { ...rule, actions: rule.actions.filter(isVisual) };
  const dry = await backend.testRule(store, visual, flow, PREVIEW_LIMIT);
  const chip = modeOf(target, rule.matcher);
  const choice: MatchChoice = chip ? { ...chip, matcher: rule.matcher } : { mode: "advanced", anyMethod: false, matcher: rule.matcher };
  if (!dry.matched) {
    return { source, base: rule, initial: initialEdits(source), pins: new Set(), kept: rule.actions, choice, note: `The rule doesn't match this flow (${dry.mismatch ?? "no match"}): its actions are kept as they are.` };
  }
  const rebuilt = rebuild(source, rule, dry);
  return { source, base: rule, initial: rebuilt.edits, pins: rebuilt.pins, kept: rebuilt.kept, choice, note: null };
}

/**
 * The Rewrite dialog: a flow's request and response as editable tables and
 * trees. Saving turns the edits into a rule that patches only what changed
 * (`lib/rewrite/diff.ts`), previewed live with the engine's own dry run.
 * Opens from "Rewrite…" on a flow (live or a document), the inspector's
 * URL bar, and "Edit visually" in the Rules Editor.
 */
export function RewriteDialog() {
  const { open, store, flow, rule, seq } = useUi((s) => s.rewrite);
  const close = useUi((s) => s.closeRewrite);
  const loaded = useQuery({
    queryKey: ["rewrite", seq],
    enabled: open && flow !== null,
    queryFn: () => load(store, flow!, rule),
    gcTime: 0,
    staleTime: Infinity,
    retry: false,
  });
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[88vh] flex-col gap-0 p-0 sm:max-w-[1320px]"
        // Focus the dialog itself, not its first field: focus left outside
        // (on the flow table) would close it on the next focus change.
        onOpenAutoFocus={(e) => {
          e.preventDefault();
          (e.currentTarget as HTMLElement | null)?.focus();
        }}
        onEscapeKeyDown={keepEscapeInEditor}
        data-testid="rewrite-dialog"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <PencilLine className="size-4" />
            {rule?.id ? "Edit rewrite" : "Rewrite"}
          </DialogTitle>
          <DialogDescription>
            Edit the flow as it should be. Saving makes a rule that changes only what you changed, on every request it
            matches.
          </DialogDescription>
        </DialogHeader>
        {loaded.isPending ? (
          <Centered>Loading the flow…</Centered>
        ) : loaded.isError ? (
          <Centered>Can&apos;t edit this flow: {errorText(loaded.error)}</Centered>
        ) : (
          <RewriteEditor key={seq} loaded={loaded.data} onDone={close} />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** One side's body: every kind's state (hooks can't be conditional), and the edit it adds up to. */
function useBody(initial: BodyEdit) {
  const tree = useUndoable<JNode>(() => (initial.kind === "json" ? initial.doc : initial.kind === "graphql" ? initial.vars : { t: "null" }));
  const [whole, setWhole] = useState(initial.kind === "json" || initial.kind === "graphql" || initial.kind === "form" ? initial.whole : false);
  const [rows, setRows] = useState<KvRow[]>(initial.kind === "form" ? initial.rows : []);
  const [text, setText] = useState(initial.kind === "text" ? initial.text : "");
  const [mode, setMode] = useState<"replace" | "find">(initial.kind === "text" ? initial.mode : "find");
  const [pairs, setPairs] = useState(initial.kind === "text" ? initial.pairs : []);
  const edit = useMemo((): BodyEdit => {
    switch (initial.kind) {
      case "json":
        return { kind: "json", doc: tree.value, whole };
      case "graphql":
        return { kind: "graphql", vars: tree.value, whole };
      case "form":
        return { kind: "form", rows, whole };
      case "text":
        return { kind: "text", text, mode, pairs };
      case "none":
        return initial;
    }
  }, [initial, tree.value, whole, rows, text, mode, pairs]);
  return { tree, whole, setWhole, rows, setRows, text, setText, mode, setMode, pairs, setPairs, edit };
}

type BodyState = ReturnType<typeof useBody>;
type Tab = "Headers" | "Query" | "Body" | "Status";

function RewriteEditor({ loaded, onDone }: { loaded: Loaded; onDone: () => void }) {
  const { source, base, pins, kept, initial } = loaded;
  const [name, setName] = useState(base.name);
  const [choice, setChoice] = useState(loaded.choice);
  const [side, setSide] = useState<Phase>(source.response && source.response.body.kind !== "none" ? "response" : "request");
  const [tabs, setTabs] = useState<Record<Phase, Tab>>({ request: "Body", response: "Body" });
  const [panel, setPanel] = useState<"preview" | "actions">("preview");
  const [reqHeaders, setReqHeaders] = useState(initial.request.headers);
  const [query, setQuery] = useState(initial.request.query);
  const [status, setStatus] = useState<number | null>(initial.response?.status ?? null);
  const [resHeaders, setResHeaders] = useState(initial.response?.headers ?? []);
  const reqBody = useBody(initial.request.body);
  const resBody = useBody(initial.response?.body ?? { kind: "none" });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const edits = useMemo(
    (): Edits => ({
      request: { headers: reqHeaders, query, body: reqBody.edit },
      response: source.response && { status: status ?? source.response.status, headers: resHeaders, body: resBody.edit },
    }),
    [source, reqHeaders, query, reqBody.edit, status, resHeaders, resBody.edit],
  );
  const built = useMemo(() => buildActions(source, edits, pins, kept), [source, edits, pins, kept]);
  const rule = useMemo((): Rule => ({ ...base, name, matcher: choice.matcher, actions: built.actions }), [base, name, choice, built]);
  const ruleKey = JSON.stringify(rule);

  // The live preview: the engine's dry run of the rule as it stands.
  const [preview, setPreview] = useState<{ result: RuleTestResult_Serialize | null; busy: boolean; error: string | null }>({
    result: null,
    busy: true,
    error: null,
  });
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      setPreview((p) => ({ ...p, busy: true }));
      backend.testRule(source.store, JSON.parse(ruleKey) as Rule, source.flow, PREVIEW_LIMIT).then(
        (result) => !cancelled && setPreview({ result, busy: false, error: null }),
        (e: unknown) => !cancelled && setPreview({ result: null, busy: false, error: errorText(e) }),
      );
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [ruleKey, source]);

  const changes = built.actions.length - kept.length;
  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await saveRule(rule);
      toast.success(`Saved “${saved.name}”`, { description: `${saved.actions.length} action${saved.actions.length === 1 ? "" : "s"}` });
      // An open Rules Editor shows the saved rule.
      if (useUi.getState().rulesEditor.open) useUi.getState().openRulesEditor({ ruleId: saved.id });
      onDone();
    } catch (e) {
      setSaveError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const body = side === "request" ? reqBody : resBody;
  // Lets dev/remote.ts make edits and save in the real app.
  useEffect(() => {
    bridge.rewrite = {
      side: (s) => setSide(s),
      tab: (s, t) => setTabs((prev) => ({ ...prev, [s]: t })),
      edit: (s, op) => editByPointer(s === "request" ? reqBody.tree : resBody.tree, op),
      whole: (s, on) => (s === "request" ? reqBody : resBody).setWhole(on),
      header: (s, header) => {
        const [rows, set] = s === "request" ? [reqHeaders, setReqHeaders] : [resHeaders, setResHeaders];
        const rest = rows.filter((r) => r.name.toLowerCase() !== header.name.toLowerCase());
        set(header.value === null ? rest : [...rest, { id: newId(), name: header.name, value: header.value }]);
      },
      query: (q) => setQuery((rows) => (q.value === null ? rows.filter((r) => r.name !== q.name) : rows.some((r) => r.name === q.name) ? rows.map((r) => (r.name === q.name ? { ...r, value: q.value! } : r)) : [...rows, { id: newId(), name: q.name, value: q.value }])),
      form: (s, f) =>
        (s === "request" ? reqBody : resBody).setRows((rows) =>
          f.value === null ? rows.filter((r) => r.name !== f.name) : rows.some((r) => r.name === f.name) ? rows.map((r) => (r.name === f.name ? { ...r, value: f.value! } : r)) : [...rows, { id: newId(), name: f.name, value: f.value }],
        ),
      status: (s) => setStatus(s),
      panel: (p) => setPanel(p),
      save,
    };
    return () => {
      bridge.rewrite = undefined;
    };
  });

  const sideTabs: Tab[] = side === "request" ? ["Headers", "Query", "Body"] : ["Status", "Headers", "Body"];
  const tab = tabs[side];
  const sideSource = side === "request" ? source.request.body : (source.response?.body ?? { kind: "none" as const, note: null });
  const contentType = side === "request" ? source.request.contentType : (source.response?.contentType ?? null);
  const before = {
    status: side === "response" ? (source.response?.status ?? null) : null,
    headers: (side === "request" ? source.request.headers : (source.response?.headers ?? [])).map(({ name, value }) => ({ name, value })),
    body: sideSource.kind === "none" ? "" : sideSource.text,
  };
  const actionsOf = (phase: Phase) => built.actions.slice(0, changes).filter((a) => ("phase" in a ? a.phase === phase : phase === (a.type === "SetStatus" ? "response" : "request"))).length;

  return (
    <>
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <MethodPill method={source.target.method} />
        <span className="text-muted-foreground min-w-0 flex-1 truncate font-mono text-xs" title={source.url} data-testid="rewrite-url">
          {source.url}
        </span>
        <Input value={name} onChange={(e) => setName(e.target.value)} className="h-7 w-96 text-sm font-medium" aria-label="Rule name" data-testid="rewrite-name" />
      </div>
      <MatcherChips target={source.target} choice={choice} onChange={setChoice} />
      {loaded.note && <div className="border-b bg-[var(--s-4xx-bg)] px-3 py-1 text-xs">{loaded.note}</div>}
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel defaultSize="60" minSize={360}>
          <div className="flex h-full min-h-0 flex-col">
            <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2">
              <Segmented
                value={side}
                onChange={(s) => setSide(s)}
                options={[
                  { value: "request", label: `Request${count(actionsOf("request"))}`, testId: "rewrite-side-request" },
                  ...(source.response ? [{ value: "response" as const, label: `Response${count(actionsOf("response"))}`, testId: "rewrite-side-response" }] : []),
                ]}
              />
              <div className="bg-border h-4 w-px" />
              <Segmented
                value={tab}
                onChange={(t) => setTabs((prev) => ({ ...prev, [side]: t }))}
                options={sideTabs.map((t) => ({ value: t, label: t, testId: `rewrite-tab-${t.toLowerCase()}` }))}
              />
            </div>
            <div className="min-h-0 flex-1 overflow-hidden" data-testid={`rewrite-pane-${side}-${tab.toLowerCase()}`}>
              {tab === "Headers" && (
                <Scroll>
                  <KvEditor
                    key={side}
                    rows={side === "request" ? reqHeaders : resHeaders}
                    original={side === "request" ? source.request.headers : (source.response?.headers ?? [])}
                    onChange={side === "request" ? setReqHeaders : setResHeaders}
                    fold
                    readOnlyName={(n) => (READ_ONLY_HEADERS.has(n.toLowerCase()) ? "Set by the proxy when the body changes" : null)}
                    addLabel="Header"
                    testId={`rewrite-headers-${side}`}
                  />
                </Scroll>
              )}
              {tab === "Query" && (
                <Scroll>
                  <KvEditor rows={query} original={source.request.query} onChange={setQuery} repeatedReadOnly addLabel="Parameter" testId="rewrite-query" />
                </Scroll>
              )}
              {tab === "Status" && source.response && (
                <div className="flex items-center gap-2 p-3 text-xs" data-testid="rewrite-status">
                  <span className="text-muted-foreground">Status</span>
                  <NumberInput value={status} min={100} max={999} onChange={setStatus} className="w-20" />
                  {status !== source.response.status && <span className="text-muted-foreground">was {source.response.status}</span>}
                </div>
              )}
              {tab === "Body" && <BodyPane key={side} source={sideSource} body={body} contentType={contentType} replaced={built.replaced[side]} many={built.actions.length > MANY_ACTIONS} />}
            </div>
          </div>
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="40" minSize={280}>
          <div className="flex h-full min-h-0 flex-col">
            <div className="flex h-8 shrink-0 items-center gap-2 border-b px-2">
              <Segmented
                value={panel}
                onChange={setPanel}
                options={[
                  { value: "preview", label: "Preview", testId: "rewrite-panel-preview" },
                  { value: "actions", label: `Actions${count(built.actions.length)}`, testId: "rewrite-panel-actions" },
                ]}
              />
            </div>
            <div className="min-h-0 flex-1">
              {panel === "preview" ? (
                <Preview side={side} before={before} {...preview} />
              ) : (
                <ActionList actions={built.actions.slice(0, changes)} kept={kept} problems={built.problems} />
              )}
            </div>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
      <div className="flex items-center gap-2 border-t px-3 py-2">
        <div className="min-w-0 flex-1 truncate text-xs" data-testid="rewrite-footer">
          {saveError ? (
            <span className="text-[var(--s-failed)]">{saveError}</span>
          ) : (
            <span className="text-muted-foreground">
              {changes === 0 ? "No changes yet." : `${changes} change${changes === 1 ? "" : "s"}`}
              {kept.length > 0 && ` · ${kept.length} kept from the saved rule`}
              {built.problems.length > 0 && ` · ${built.problems.join("; ")}`}
            </span>
          )}
        </div>
        <Button variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" disabled={saving || built.actions.length === 0 || name.trim() === ""} onClick={() => void save()} data-testid="rewrite-save">
          <Save />
          {base.id ? "Save rule" : "Create rule"}
        </Button>
      </div>
    </>
  );
}

const count = (n: number) => (n > 0 ? ` · ${n}` : "");

function Scroll({ children }: { children: ReactNode }) {
  return <div className="h-full overflow-y-auto">{children}</div>;
}

function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange: (v: T) => void; options: { value: T; label: string; testId: string }[] }) {
  return (
    <div className="bg-muted/50 flex items-center gap-0.5 rounded-md p-0.5 text-xs">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          data-active={o.value === value}
          onClick={() => onChange(o.value)}
          className="text-muted-foreground data-[active=true]:bg-background data-[active=true]:text-foreground h-6 rounded-sm px-2 data-[active=true]:shadow-xs"
          data-testid={o.testId}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function WholeToggle({ body }: { body: BodyState }) {
  return (
    <label className="flex items-center gap-1.5 text-[11px]" title="Send the edited body instead of patching only what changed">
      <Switch size="sm" checked={body.whole} onCheckedChange={body.setWhole} data-testid="rewrite-whole-body" />
      Replace whole body
    </label>
  );
}

function BodyPane({
  source,
  body,
  contentType,
  replaced,
  many,
}: {
  source: BodySource;
  body: BodyState;
  contentType: string | null;
  replaced: ReadonlySet<number>;
  many: boolean;
}) {
  const hint = many && !body.whole && (
    <div className="border-b bg-[var(--s-4xx-bg)] px-2 py-1 text-xs" data-testid="rewrite-many">
      Over {MANY_ACTIONS} changes: “Replace whole body” sends one body instead.
    </div>
  );
  switch (source.kind) {
    case "none":
      return <Centered>{source.note ?? "No body."}</Centered>;
    case "json":
    case "graphql":
      return (
        <div className="flex h-full flex-col">
          {source.kind === "graphql" && (
            <div className="text-muted-foreground border-b px-2 py-1 text-xs" data-testid="rewrite-graphql">
              GraphQL variables{source.via === "query" ? " (the variables query parameter)" : ""}
            </div>
          )}
          {hint}
          <div className="min-h-0 flex-1">
            <JsonEditor
              doc={body.tree}
              original={source.kind === "json" ? source.doc : source.vars}
              replaced={replaced}
              toolbar={source.kind === "json" || source.via === "body" ? <WholeToggle body={body} /> : null}
              testId="rewrite-tree"
            />
          </div>
        </div>
      );
    case "form":
      return (
        <div className="flex h-full flex-col">
          <div className="flex h-7 shrink-0 items-center border-b px-2">
            <span className="text-muted-foreground text-xs">Form fields</span>
            <div className="ml-auto">
              <WholeToggle body={body} />
            </div>
          </div>
          <Scroll>
            <KvEditor rows={body.rows} original={source.rows} onChange={body.setRows} repeatedReadOnly addLabel="Field" testId="rewrite-form" />
          </Scroll>
        </div>
      );
    case "text":
      return (
        <div className="flex h-full flex-col">
          {source.note && <div className="text-muted-foreground border-b px-2 py-1 text-xs">{source.note}</div>}
          <div className="flex h-7 shrink-0 items-center border-b px-2">
            <Segmented
              value={body.mode}
              onChange={body.setMode}
              options={[
                { value: "find", label: "Find & replace", testId: "rewrite-text-find" },
                { value: "replace", label: "Replace whole body", testId: "rewrite-text-replace" },
              ]}
            />
          </div>
          {body.mode === "replace" ? (
            <div className="min-h-0 flex-1">
              <CodeEditor value={body.text} language={languageFor(contentType)} onChange={body.setText} testId="rewrite-text" />
            </div>
          ) : (
            <Scroll>
              <FindReplace pairs={body.pairs} onChange={body.setPairs} />
            </Scroll>
          )}
        </div>
      );
  }
}

function FindReplace({ pairs, onChange }: { pairs: { find: string; replace: string }[]; onChange: (p: { find: string; replace: string }[]) => void }) {
  const rows = [...pairs, { find: "", replace: "" }];
  const update = (i: number, patch: Partial<{ find: string; replace: string }>) => {
    const next = rows.map((p, j) => (i === j ? { ...p, ...patch } : p)).filter((p) => p.find !== "" || p.replace !== "");
    onChange(next);
  };
  return (
    <div className="flex flex-col gap-1 p-2 text-xs" data-testid="rewrite-find-replace">
      <span className="text-muted-foreground">Each text found in the body is replaced, every time it occurs.</span>
      {rows.map((p, i) => (
        <div key={i} className="flex items-center gap-1">
          <Input value={p.find} placeholder="find" onChange={(e) => update(i, { find: e.target.value })} className="h-6 font-mono text-xs" />
          <span className="text-muted-foreground">→</span>
          <Input value={p.replace} placeholder="replace with" onChange={(e) => update(i, { replace: e.target.value })} className="h-6 font-mono text-xs" />
        </div>
      ))}
    </div>
  );
}

function ActionList({ actions, kept, problems }: { actions: Action[]; kept: Action[]; problems: string[] }) {
  return (
    <div className="h-full overflow-y-auto p-2 text-xs" data-testid="rewrite-actions">
      {actions.length === 0 && <div className="text-muted-foreground">Edit the flow to add actions.</div>}
      <ol className="flex flex-col gap-0.5">
        {actions.map((a, i) => (
          <li key={i} className="flex gap-2 font-mono" data-testid="rewrite-action">
            <span className="text-muted-foreground w-28 shrink-0 font-sans">{actionLabel(a.type)}</span>
            <span className="min-w-0 break-all">{actionSummary(a)}</span>
          </li>
        ))}
      </ol>
      {kept.length > 0 && (
        <>
          <div className="text-muted-foreground mt-3 mb-1 text-2xs font-semibold tracking-wide uppercase">Kept from the saved rule</div>
          <ol className={cn("flex flex-col gap-0.5 opacity-80")}>
            {kept.map((a, i) => (
              <li key={i} className="flex gap-2 font-mono" data-testid="rewrite-kept">
                <span className="text-muted-foreground w-28 shrink-0 font-sans">{actionLabel(a.type)}</span>
                <span className="min-w-0 break-all">{actionSummary(a)}</span>
              </li>
            ))}
          </ol>
        </>
      )}
      {problems.length > 0 && (
        <ul className="mt-3 list-disc pl-4 text-[var(--s-failed)]">
          {problems.map((p) => (
            <li key={p}>{p}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** A tree edit addressed by JSON pointer, for dev/remote.ts. */
export type PointerEdit =
  | { set: string; raw: string }
  | { rename: string; key: string }
  | { add: string; key?: string; raw: string }
  | { remove: string };

function pathOf(root: JNode, pointer: string): Path | null {
  const tokens = pointer === "" ? [] : pointer.slice(1).split("/").map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
  const path: number[] = [];
  let node = root;
  for (const t of tokens) {
    const kids = children(node);
    const child = node.t === "obj" ? [...node.entries].reverse().find((e) => e.key === t) : kids[Number(t)];
    if (!child) return null;
    path.push(child.id);
    node = child.v;
  }
  return path;
}

function editByPointer(tree: Undoable<JNode>, op: PointerEdit) {
  tree.set((root) => {
    if ("set" in op) {
      const path = pathOf(root, op.set);
      return path ? setValue(root, path, parseDoc(op.raw)) : root;
    }
    if ("rename" in op) {
      const path = pathOf(root, op.rename);
      return path ? rename(root, path, op.key) : root;
    }
    if ("remove" in op) {
      const path = pathOf(root, op.remove);
      return path ? remove(root, path) : root;
    }
    const parent = pathOf(root, op.add);
    if (!parent || !nodeAt(root, parent)) return root;
    return insert(root, parent, null, parseDoc(op.raw), op.key).root;
  });
}
