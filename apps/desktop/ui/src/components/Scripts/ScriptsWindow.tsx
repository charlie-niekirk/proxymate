import { useQuery } from "@tanstack/react-query";
import { Bot, ClipboardCopy, FileCode2, FileDown, FileUp, ListChecks, Plus, Save, Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import type { BodyPart, ContentKind, ScriptKind, ScriptTestResult_Serialize } from "@/bindings";
import { Checkbox, Choice } from "@/components/Rules/fields";
import { MatcherBuilder } from "@/components/Rules/MatcherBuilder";
import { RuleOriginBadge } from "@/components/Rules/RuleOriginBadge";
import { CopyItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { Button } from "@/components/ui/button";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { bridge } from "@/dev/bridge";
import { useScripts } from "@/hooks/useScripts";
import {
  copyScriptTypes,
  deleteAgentScripts,
  deleteScript,
  duplicateScript,
  errorText,
  exportScript,
  importScript,
  reportError,
  ruleForScript,
  saveScript,
  setScriptEnabled,
} from "@/lib/actions";
import { backend } from "@/lib/backend";
import { rulesKey } from "@/lib/query";
import { normalizeRule } from "@/lib/rules";
import {
  KIND_HELP,
  KIND_LABELS,
  KIND_ORDER,
  TEMPLATES,
  emptyViewConfig,
  newScript,
  type Script,
} from "@/lib/scripts";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

import { ScriptConsole } from "./ScriptConsole";
import { ScriptEditor } from "./ScriptEditor";
import { ScriptTestPanel } from "./ScriptTestPanel";

/**
 * The Scripts window (M14): scripts grouped by kind beside the selected
 * one's editor, where it runs, a dry run against the selected flow, and the
 * script console. Scripts run in QuickJS with no network or file access;
 * the API is `proxymate.d.ts` ("Copy type definitions").
 */
export function ScriptsWindow() {
  const { open, scriptId, draft, seq } = useUi((s) => s.scripts);
  const close = useUi((s) => s.closeScripts);
  const scripts = useScripts();
  const current = draft ?? scripts.find((s) => s.id === scriptId) ?? null;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[88vh] flex-col gap-0 p-0 sm:max-w-[1240px]"
        onOpenAutoFocus={(e) => e.preventDefault()}
        // Escape closes the editor's completion list first, not the window.
        onEscapeKeyDown={(e) => document.querySelector(".cm-tooltip-autocomplete") && e.preventDefault()}
        data-testid="scripts-window"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <FileCode2 className="size-4" />
            Scripts
          </DialogTitle>
          <DialogDescription>
            JavaScript that decodes bodies into extra views, rewrites traffic or mocks responses. Scripts have no network
            or file access.
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          <ScriptList scripts={scripts} selected={draft ? null : scriptId} hasDraft={!!draft} />
          <div className="min-w-0 flex-1">
            {current ? (
              <ScriptForm key={`${seq}-${current.id}`} initial={current} />
            ) : (
              <div className="text-muted-foreground flex h-full flex-col items-center justify-center gap-2 text-xs">
                <span>Select a script, or start one from a template.</span>
                <NewScriptMenu />
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function NewScriptMenu() {
  const openScripts = useUi((s) => s.openScripts);
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="xs" data-testid="new-script">
          <Plus />
          New script
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-1 text-xs">
        {KIND_ORDER.map((kind) => (
          <div key={kind} className="py-0.5">
            <div className="text-muted-foreground px-2 py-0.5 text-2xs font-semibold uppercase">{KIND_LABELS[kind]}</div>
            {TEMPLATES.filter((t) => t.kind === kind).map((t) => (
              <button
                key={t.id}
                type="button"
                className="hover:bg-accent flex w-full flex-col rounded-sm px-2 py-1 text-left"
                onClick={() => {
                  openScripts({ draft: newScript(kind, t) });
                  setOpen(false);
                }}
                data-testid={`template-${t.id}`}
              >
                <span className="font-medium">{t.name}</span>
                <span className="text-muted-foreground">{t.description}</span>
              </button>
            ))}
          </div>
        ))}
      </PopoverContent>
    </Popover>
  );
}

/** A script row's menu: what the form's buttons do, plus copies. */
function ScriptItems({ script }: { script: Script }) {
  const openScripts = useUi((s) => s.openScripts);
  const store = useUi((s) => s.store);
  const flow = useUi((s) => s.selectedId);
  const fail = (what: string) => (e: unknown) => reportError(what, e);
  return (
    <>
      <ContextMenuItem onSelect={() => openScripts({ scriptId: script.id })}>Open</ContextMenuItem>
      <ContextMenuItem
        onSelect={() => void setScriptEnabled(script.id, !script.enabled).catch(fail("Couldn't change the script"))}
        data-testid="script-menu-toggle"
      >
        {script.enabled ? "Disable" : "Enable"}
      </ContextMenuItem>
      <ContextMenuItem
        onSelect={() =>
          void duplicateScript(script)
            .then((copy) => openScripts({ scriptId: copy.id }))
            .catch(fail("Couldn't duplicate the script"))
        }
        data-testid="script-menu-duplicate"
      >
        Duplicate
      </ContextMenuItem>
      {script.kind !== "view" && (
        <ContextMenuItem
          disabled={flow === null}
          onSelect={() => flow !== null && void ruleForScript(store, flow, script).catch(fail("Couldn't draft a rule"))}
        >
          Create rule for selected flow…
        </ContextMenuItem>
      )}
      <ContextMenuItem onSelect={() => void exportScript(script.id).catch(fail("Couldn't export the script"))}>Export…</ContextMenuItem>
      <ContextMenuSeparator />
      <CopyItem text={script.name} what="script name">
        Copy name
      </CopyItem>
      <CopyItem text={script.source} what="source" testId="script-menu-copy-source">
        Copy source
      </CopyItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        onSelect={() =>
          void deleteScript(script.id)
            .then(() => openScripts({ scriptId: null }))
            .catch(fail("Couldn't delete the script"))
        }
      >
        Delete
      </ContextMenuItem>
    </>
  );
}

function ScriptList({ scripts, selected, hasDraft }: { scripts: Script[]; selected: string | null; hasDraft: boolean }) {
  const openScripts = useUi((s) => s.openScripts);
  const menu = useMenuTarget((key) => scripts.find((s) => s.id === key));
  const agentScripts = scripts.filter((s) => s.origin.kind === "agent").length;
  const onImport = () =>
    importScript().then(
      (script) => script && openScripts({ scriptId: script.id }),
      (e: unknown) => reportError("Couldn't import the script", e),
    );

  return (
    <div className="bg-muted/20 flex w-72 shrink-0 flex-col border-r">
      <div className="flex flex-wrap items-center gap-1 border-b p-1.5">
        <NewScriptMenu />
        <Button variant="ghost" size="xs" onClick={() => void onImport()} title="Import a .js file" data-testid="import-script">
          <FileUp />
          Import…
        </Button>
        {agentScripts > 0 && (
          <Button
            variant="ghost"
            size="xs"
            className="text-muted-foreground ml-auto"
            title="Delete every script an AI agent created that no rule runs"
            onClick={() => void deleteAgentScripts().catch((e: unknown) => reportError("Couldn't delete agent scripts", e))}
            data-testid="remove-agent-scripts"
          >
            <Bot />
            Remove agent scripts
          </Button>
        )}
      </div>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div className="min-h-0 flex-1 overflow-y-auto" onContextMenu={menu.onContextMenu}>
            <div className="flex flex-col p-1" data-testid="script-list">
              {hasDraft && (
                <div className="bg-selection text-selection-foreground mb-0.5 rounded-sm px-2 py-1 text-xs italic">
                  Unsaved script
                </div>
              )}
              {scripts.length === 0 && !hasDraft && <div className="text-muted-foreground p-2 text-xs">No scripts yet.</div>}
              {KIND_ORDER.map((kind) => {
                const group = scripts.filter((s) => s.kind === kind);
                if (group.length === 0) return null;
                return (
                  <div key={kind} className="mb-1" data-testid={`script-group-${kind}`}>
                    <div className="text-muted-foreground px-1.5 pt-1 pb-0.5 text-2xs font-semibold tracking-wide uppercase">
                      {KIND_LABELS[kind]}
                    </div>
                    {group.map((script) => (
                      <div
                        key={script.id}
                        onClick={() => openScripts({ scriptId: script.id })}
                        data-selected={script.id === selected}
                        data-menu-key={script.id}
                        data-testid="script-row"
                        className={cn(
                          "flex cursor-default items-center gap-1.5 rounded-sm px-1.5 py-1 text-xs",
                          "hover:bg-accent data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
                        )}
                      >
                        <Switch
                          size="sm"
                          checked={script.enabled}
                          aria-label={script.enabled ? "Enabled" : "Disabled"}
                          onClick={(e) => e.stopPropagation()}
                          onCheckedChange={(on) =>
                            void setScriptEnabled(script.id, on).catch((e: unknown) => reportError("Couldn't change the script", e))
                          }
                        />
                        <span className={cn("min-w-0 flex-1 truncate font-medium", !script.enabled && "opacity-50")}>
                          {script.name || "Untitled"}
                        </span>
                        <RuleOriginBadge origin={script.origin} />
                      </div>
                    ))}
                  </div>
                );
              })}
            </div>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-56" data-testid="script-menu">
          {menu.target && <ScriptItems script={menu.target} />}
        </ContextMenuContent>
      </ContextMenu>
    </div>
  );
}

function ScriptForm({ initial }: { initial: Script }) {
  const openScripts = useUi((s) => s.openScripts);
  const store = useUi((s) => s.store);
  const selectedFlow = useUi((s) => s.selectedId);
  const [script, setScript] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [tab, setTab] = useState(initial.kind === "view" ? "runs" : "test");
  const [test, setTest] = useState<{ result: ScriptTestResult_Serialize | null; error: string | null; busy: boolean }>({
    result: null,
    error: null,
    busy: false,
  });
  const unsaved = !script.id || JSON.stringify(script) !== JSON.stringify(initial);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const saved = await saveScript(script);
      openScripts({ scriptId: saved.id });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const runTest = (part?: BodyPart) => {
    if (selectedFlow === null) return;
    setTab("test");
    setTest((t) => ({ ...t, busy: true }));
    backend
      .testScript(store, selectedFlow, {
        script_id: script.id || null,
        source: script.source,
        kind: script.kind,
        part: part ?? "response",
      })
      .then(
        (result) => setTest({ result, error: null, busy: false }),
        (e: unknown) => setTest({ result: null, error: errorText(e), busy: false }),
      );
  };

  const remove = async () => {
    try {
      await deleteScript(script.id);
      openScripts({ scriptId: null });
    } catch (e) {
      setError(errorText(e));
    }
  };

  // Lets dev/remote.ts press Save and Test in the real app.
  useEffect(() => {
    bridge.saveScript = save;
    bridge.testScript = runTest;
    return () => {
      bridge.saveScript = undefined;
      bridge.testScript = undefined;
    };
  });

  return (
    <div className="flex h-full flex-col" data-testid="script-form" data-kind={script.kind}>
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <Input
          value={script.name}
          onChange={(e) => setScript({ ...script, name: e.target.value })}
          className="h-7 max-w-md text-sm font-medium"
          aria-label="Script name"
          data-testid="script-name"
        />
        {script.id ? (
          <span className="text-muted-foreground text-xs" title={KIND_HELP[script.kind]}>
            {KIND_LABELS[script.kind]} script
          </span>
        ) : (
          <Choice
            value={script.kind}
            options={KIND_ORDER.map((k) => ({ value: k, label: `${KIND_LABELS[k]} script` }))}
            onChange={(kind: ScriptKind) =>
              setScript({ ...script, kind, view: kind === "view" ? (script.view ?? emptyViewConfig()) : null })
            }
            className="h-6 w-36 text-xs"
            testId="script-kind"
          />
        )}
        <Checkbox checked={script.enabled} onChange={(enabled) => setScript({ ...script, enabled })} label="Enabled" />
        <RuleOriginBadge origin={script.origin} />
        {unsaved && <span className="text-muted-foreground text-xs">Unsaved</span>}
      </div>
      <div className="text-muted-foreground border-b px-3 py-1 text-2xs">{KIND_HELP[script.kind]}</div>
      <div className="min-h-0 flex-[3] border-b">
        <ScriptEditor
          value={initial.source}
          kind={script.kind}
          onChange={(source) => setScript((s) => ({ ...s, source }))}
        />
      </div>
      <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-[2] flex-col gap-0">
        <TabsList className="h-7 shrink-0 rounded-none border-b bg-transparent px-2">
          <TabsTrigger value="runs" className="text-xs" data-testid="script-tab-runs">
            {script.kind === "view" ? "Where it runs" : "Used by rules"}
          </TabsTrigger>
          <TabsTrigger value="test" className="text-xs" data-testid="script-tab-test">
            Test
          </TabsTrigger>
          <TabsTrigger value="console" className="text-xs" data-testid="script-tab-console">
            Console
          </TabsTrigger>
        </TabsList>
        <TabsContent value="runs" className="min-h-0 flex-1 overflow-y-auto p-3">
          {script.kind === "view" ? (
            <ViewSettings script={script} onChange={setScript} />
          ) : (
            <UsedByRules script={script} store={store} flow={selectedFlow} />
          )}
        </TabsContent>
        <TabsContent value="test" className="min-h-0 flex-1 overflow-y-auto p-3">
          <ScriptTestPanel kind={script.kind} flowId={selectedFlow} onTest={runTest} {...test} />
        </TabsContent>
        <TabsContent value="console" className="min-h-0 flex-1">
          <ScriptConsole scriptId={script.id || null} />
        </TabsContent>
      </Tabs>
      <div className="flex items-center gap-2 border-t px-3 py-2">
        <div className="min-w-0 flex-1 truncate text-xs text-[var(--s-failed)]" title={error ?? ""} data-testid="script-error">
          {error}
        </div>
        <Button variant="ghost" size="sm" onClick={() => void copyScriptTypes()} title="Copy proxymate.d.ts" data-testid="copy-script-types">
          <ClipboardCopy />
          Copy type definitions
        </Button>
        {script.id && (
          <>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void exportScript(script.id).catch((e: unknown) => reportError("Couldn't export the script", e))}
              data-testid="export-script"
            >
              <FileDown />
              Export…
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void remove()} data-testid="delete-script">
              <Trash2 />
              Delete
            </Button>
          </>
        )}
        <Button size="sm" disabled={saving || !unsaved} onClick={() => void save()} data-testid="save-script">
          <Save />
          {script.id ? "Save" : "Save script"}
        </Button>
      </div>
    </div>
  );
}

const PARTS: { value: BodyPart; label: string }[] = [
  { value: "response", label: "Response bodies" },
  { value: "request", label: "Request bodies" },
];

const KINDS: { value: ContentKind; label: string }[] = [
  { value: "json", label: "JSON" },
  { value: "formurlencoded", label: "Form" },
  { value: "text", label: "Text" },
  { value: "xml", label: "XML" },
  { value: "html", label: "HTML" },
  { value: "graphql", label: "GraphQL" },
  { value: "javascript", label: "JavaScript" },
  { value: "multipart", label: "Multipart" },
  { value: "binary", label: "Binary" },
  { value: "image", label: "Image" },
  { value: "none", label: "No body" },
];

/** A view script's matcher, and which bodies (part and kind) it views. */
function ViewSettings({ script, onChange }: { script: Script; onChange: (s: Script) => void }) {
  const view = script.view ?? emptyViewConfig();
  const set = (patch: Partial<typeof view>) => onChange({ ...script, view: { ...view, ...patch } });
  const toggle = <T,>(list: T[], item: T) => (list.includes(item) ? list.filter((x) => x !== item) : [...list, item]);
  return (
    <div className="flex flex-col gap-3" data-testid="view-settings">
      <Section title="Bodies" hint="None ticked means all.">
        <div className="flex flex-wrap gap-3">
          {PARTS.map((p) => (
            <Checkbox key={p.value} checked={view.parts.includes(p.value)} onChange={() => set({ parts: toggle(view.parts, p.value) })} label={p.label} />
          ))}
        </div>
        <div className="flex flex-wrap gap-3">
          {KINDS.map((k) => (
            <Checkbox key={k.value} checked={view.kinds.includes(k.value)} onChange={() => set({ kinds: toggle(view.kinds, k.value) })} label={k.label} />
          ))}
        </div>
      </Section>
      <Section title="Flows">
        <MatcherBuilder matcher={view.matcher} onChange={(matcher) => set({ matcher })} />
      </Section>
    </div>
  );
}

/** The rules that run a rewrite or mock script, and a quick way to add one. */
function UsedByRules({ script, store, flow }: { script: Script; store: number; flow: number | null }) {
  const { data: rules = [] } = useQuery({ queryKey: rulesKey, queryFn: () => backend.listRules() });
  const openRules = useUi((s) => s.openRulesEditor);
  const users = rules
    .map(normalizeRule)
    .filter((r) => r.actions.some((a) => a.type === "Script" && a.script_id === script.id));
  return (
    <div className="flex flex-col gap-2 text-xs" data-testid="used-by-rules">
      {!script.id ? (
        <div className="text-muted-foreground">Save the script, then add it to a rule with a Run script action.</div>
      ) : users.length === 0 ? (
        <div className="text-muted-foreground">No rule runs this script yet; it does nothing until one does.</div>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {users.map((r) => (
            <li key={r.id}>
              <button type="button" className="hover:text-foreground inline-flex items-center gap-1 underline-offset-2 hover:underline" onClick={() => openRules({ ruleId: r.id })}>
                <ListChecks className="size-3" />
                {r.name}
                {!r.enabled && <span className="text-muted-foreground">(off)</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="xs"
          disabled={!script.id || flow === null}
          onClick={() => void ruleForScript(store, flow!, script).catch((e: unknown) => reportError("Couldn't draft the rule", e))}
          data-testid="rule-for-script"
        >
          <Plus />
          Create rule for selected flow
        </Button>
        <span className="text-muted-foreground">
          {!script.id ? "Save the script first" : flow === null ? "Select a flow first" : "Matches its method, host and path"}
        </span>
      </div>
    </div>
  );
}

function Section({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-muted-foreground text-2xs font-semibold tracking-wide uppercase">
        {title}
        {hint && <span className="ml-2 font-normal normal-case">{hint}</span>}
      </h3>
      {children}
    </section>
  );
}
