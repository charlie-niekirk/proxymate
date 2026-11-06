//! The Rewrite dialog's editor: both sides' tabs, the match chips, the
//! live preview and Save.

import { Save } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import type { Phase, RuleTestResult_Serialize } from "@/bindings";
import { MethodPill } from "@/components/FlowTable/cells";
import { NumberInput } from "@/components/Rules/fields";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { useBridge } from "@/dev/bridge";
import { saveRule } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { newId } from "@/lib/json/doc";
import { READ_ONLY_HEADERS } from "@/lib/rewrite/diff";
import { buildActions, MANY_ACTIONS, PREVIEW_LIMIT, type Edits } from "@/lib/rewrite/session";
import { type Rule } from "@/lib/rules";
import { useUi } from "@/store/ui";

import { KvEditor } from "./KvEditor";
import { MatcherChips } from "./MatcherChips";
import { Preview } from "./Preview";
import { BodyPane } from "./BodyPane";
import { useBody } from "./useBody";
import { errorText, type Loaded } from "./load";
import { ActionList, Scroll, Segmented } from "./parts";
import { editByPointer } from "./pointer";

export type Tab = "Headers" | "Query" | "Body" | "Status";

const count = (n: number) => (n > 0 ? ` · ${n}` : "");

export function RewriteEditor({ loaded, onDone }: { loaded: Loaded; onDone: () => void }) {
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
  useBridge({
    rewrite: {
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
    },
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
