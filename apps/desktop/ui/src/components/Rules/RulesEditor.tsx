import { useQuery } from "@tanstack/react-query";
import { Bot, GripVertical, ListChecks, PencilLine, Plus, Save, Trash2 } from "lucide-react";
import { useEffect, useState, type DragEvent, type ReactNode } from "react";

import type { RuleTestResult_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { bridge } from "@/dev/bridge";
import { deleteAgentRules, deleteRule, openRewrite, reorderRules, saveRule, setRuleEnabled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { queryClient, ruleSettingsKey, rulesKey } from "@/lib/query";
import { isVisual } from "@/lib/rewrite/diff";
import { actionSummary, isBuiltin, matcherSummary, newRule, normalizeRule, type Rule } from "@/lib/rules";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

import { ActionListEditor } from "./ActionListEditor";
import { Checkbox, NumberInput } from "./fields";
import { MatcherBuilder } from "./MatcherBuilder";
import { RuleOriginBadge } from "./RuleOriginBadge";
import { RuleTestPanel } from "./RuleTestPanel";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The Rules Editor: the ordered rule list (drag to reorder) beside the
 * selected rule's matcher, actions and dry run. Presets from the flow menu
 * open here as unsaved drafts.
 */
export function RulesEditor() {
  const { open, ruleId, draft, seq } = useUi((s) => s.rulesEditor);
  const close = useUi((s) => s.closeRulesEditor);
  const { data: rules = [] } = useQuery({ queryKey: rulesKey, queryFn: () => backend.listRules(), enabled: open });
  const userRules = rules.filter((r) => !isBuiltin(r)).map(normalizeRule);
  const current = draft ?? userRules.find((r) => r.id === ruleId) ?? null;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[84vh] flex-col gap-0 p-0 sm:max-w-[1180px]"
        onOpenAutoFocus={(e) => e.preventDefault()}
        data-testid="rules-editor"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <ListChecks className="size-4" />
            Rules
          </DialogTitle>
          <DialogDescription>
            Every matching rule runs, top to bottom. Map Local, rewrites, breakpoints, Map Remote, throttling and
            blocking are all actions.
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          <RuleList rules={userRules} selected={draft ? null : ruleId} hasDraft={!!draft} />
          <div className="min-w-0 flex-1">
            {current ? (
              <RuleForm key={`${seq}-${current.id}`} initial={current} />
            ) : (
              <div className="text-muted-foreground flex h-full items-center justify-center text-xs">
                Select a rule, or create one from a flow&apos;s context menu.
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function RuleList({ rules, selected, hasDraft }: { rules: Rule[]; selected: string | null; hasDraft: boolean }) {
  const openEditor = useUi((s) => s.openRulesEditor);
  const [dragged, setDragged] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const agentRules = rules.filter((r) => r.origin.kind === "agent").length;

  const onDrop = (target: string) => {
    if (!dragged || dragged === target) return;
    const ids = rules.map((r) => r.id);
    const from = ids.indexOf(dragged);
    const to = ids.indexOf(target);
    ids.splice(from, 1);
    ids.splice(to, 0, dragged);
    void reorderRules(ids);
  };

  return (
    <div className="bg-muted/20 flex w-72 shrink-0 flex-col border-r">
      <div className="flex items-center gap-1 border-b p-1.5">
        <Button variant="outline" size="xs" onClick={() => openEditor({ draft: newRule() })} data-testid="new-rule">
          <Plus />
          New rule
        </Button>
        {agentRules > 0 && (
          <Button
            variant="ghost"
            size="xs"
            className="text-muted-foreground ml-auto"
            title="Delete every rule an AI agent created"
            onClick={() => void deleteAgentRules()}
            data-testid="remove-agent-rules"
          >
            <Bot />
            Remove all agent rules
          </Button>
        )}
      </div>
      {/* A plain scroller: ScrollArea's table layout defeats truncation. */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="flex flex-col p-1" data-testid="rule-list">
          {hasDraft && (
            <div className="bg-selection text-selection-foreground mb-0.5 rounded-sm px-2 py-1 text-xs italic">
              Unsaved rule
            </div>
          )}
          {rules.length === 0 && !hasDraft && <div className="text-muted-foreground p-2 text-xs">No rules yet.</div>}
          {rules.map((rule) => (
            <div
              key={rule.id}
              draggable
              onDragStart={(e: DragEvent) => {
                setDragged(rule.id);
                e.dataTransfer.effectAllowed = "move";
              }}
              onDragOver={(e) => {
                e.preventDefault();
                setOver(rule.id);
              }}
              onDragLeave={() => setOver((o) => (o === rule.id ? null : o))}
              onDrop={(e) => {
                e.preventDefault();
                onDrop(rule.id);
                setOver(null);
              }}
              onDragEnd={() => setDragged(null)}
              onClick={() => openEditor({ ruleId: rule.id })}
              data-selected={rule.id === selected}
              data-testid="rule-row"
              className={cn(
                "group flex cursor-default items-start gap-1 rounded-sm px-1 py-1 text-xs",
                "hover:bg-accent data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
                over === rule.id && dragged !== rule.id && "ring-primary ring-1",
                dragged === rule.id && "opacity-50",
              )}
            >
              <GripVertical className="text-muted-foreground mt-0.5 size-3 shrink-0 opacity-50" />
              <input
                type="checkbox"
                className="mt-0.5 size-3 shrink-0 accent-[var(--primary)]"
                checked={rule.enabled}
                title={rule.enabled ? "Enabled" : "Disabled"}
                onClick={(e) => e.stopPropagation()}
                onChange={(e) => void setRuleEnabled(rule.id, e.target.checked)}
              />
              <div className={cn("min-w-0 flex-1", !rule.enabled && "opacity-50")}>
                <div className="flex items-center gap-1">
                  <span className="truncate font-medium">{rule.name || "Untitled"}</span>
                  <RuleOriginBadge origin={rule.origin} />
                </div>
                <div className="truncate font-mono text-2xs opacity-70">{matcherSummary(rule.matcher)}</div>
                <div className="truncate text-2xs opacity-70">
                  {rule.actions.map(actionSummary).join(" · ") || "no actions"}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
      <BreakpointTimeout />
    </div>
  );
}

function BreakpointTimeout() {
  const { data: settings } = useQuery({ queryKey: ruleSettingsKey, queryFn: () => backend.getRuleSettings() });
  if (!settings) return null;
  return (
    <div className="text-muted-foreground flex items-center gap-1.5 border-t p-2 text-xs" title="0 waits forever">
      Breakpoint timeout
      <NumberInput
        value={settings.breakpoint_timeout_secs}
        className="w-16"
        onChange={(secs) => {
          void backend
            .setRuleSettings({ breakpoint_timeout_secs: secs ?? 0 })
            .then((s) => queryClient.setQueryData(ruleSettingsKey, s));
        }}
      />
      s
    </div>
  );
}

function RuleForm({ initial }: { initial: Rule }) {
  const openEditor = useUi((s) => s.openRulesEditor);
  const selectedFlow = useUi((s) => s.selectedId);
  const store = useUi((s) => s.store);
  const [rule, setRule] = useState(initial);
  const [invalid, setInvalid] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<{ result: RuleTestResult_Serialize | null; error: string | null; busy: boolean }>({
    result: null,
    error: null,
    busy: false,
  });
  const unsaved = !rule.id || JSON.stringify(rule) !== JSON.stringify(initial);

  // Live validation: the engine compiles the rule as it would on save.
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      backend.validateRule(rule).then(
        () => !cancelled && setInvalid(null),
        (e: unknown) => !cancelled && setInvalid(errorText(e)),
      );
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [rule]);

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await saveRule(rule);
      openEditor({ ruleId: saved.id });
    } catch (e) {
      setSaveError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  const runTest = () => {
    if (selectedFlow === null) return;
    setTest((t) => ({ ...t, busy: true }));
    backend.testRule(store, rule, selectedFlow).then(
      (result) => setTest({ result, error: null, busy: false }),
      (e: unknown) => setTest({ result: null, error: errorText(e), busy: false }),
    );
  };

  const remove = async () => {
    try {
      await deleteRule(rule.id);
      openEditor({ ruleId: null });
    } catch (e) {
      setSaveError(errorText(e));
    }
  };

  // Lets dev/remote.ts press Save and Test in the real app.
  useEffect(() => {
    bridge.saveRule = save;
    bridge.testRule = runTest;
    return () => {
      bridge.saveRule = undefined;
      bridge.testRule = undefined;
    };
  });

  return (
    <div className="flex h-full flex-col" data-testid="rule-form">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <Input
          value={rule.name}
          onChange={(e) => setRule({ ...rule, name: e.target.value })}
          className="h-7 max-w-md text-sm font-medium"
          aria-label="Rule name"
          data-testid="rule-name"
        />
        <Checkbox checked={rule.enabled} onChange={(enabled) => setRule({ ...rule, enabled })} label="Enabled" />
        <RuleOriginBadge origin={rule.origin} />
        {unsaved && <span className="text-muted-foreground text-xs">Unsaved</span>}
        {(rule.actions.some(isVisual) || (rule.origin.kind === "preset" && rule.origin.preset === "rewrite")) && (
          <Button
            variant="outline"
            size="xs"
            className="ml-auto"
            title="Edit this rule's rewrites on a flow it matches"
            onClick={() => void openRewrite(store, null, rule)}
            data-testid="edit-visually"
          >
            <PencilLine />
            Edit visually
          </Button>
        )}
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-4 p-3">
          <Section title="Match">
            <MatcherBuilder matcher={rule.matcher} onChange={(matcher) => setRule({ ...rule, matcher })} />
          </Section>
          <Section title="Actions">
            <ActionListEditor actions={rule.actions} onChange={(actions) => setRule({ ...rule, actions })} />
          </Section>
          <Section title="Dry run">
            <RuleTestPanel flowId={selectedFlow} onTest={runTest} {...test} />
          </Section>
        </div>
      </ScrollArea>
      <div className="flex items-center gap-2 border-t px-3 py-2">
        <div className="min-w-0 flex-1 truncate text-xs text-[var(--s-failed)]" title={saveError ?? invalid ?? ""} data-testid="rule-error">
          {saveError ?? invalid}
        </div>
        {rule.id && (
          <Button variant="ghost" size="sm" onClick={() => void remove()} data-testid="delete-rule">
            <Trash2 />
            Delete
          </Button>
        )}
        <Button size="sm" disabled={saving || invalid !== null || !unsaved} onClick={() => void save()} data-testid="save-rule">
          <Save />
          {rule.id ? "Save" : "Save rule"}
        </Button>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-muted-foreground text-2xs font-semibold tracking-wide uppercase">{title}</h3>
      {children}
    </section>
  );
}
