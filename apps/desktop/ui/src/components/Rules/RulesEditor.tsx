import { useQuery } from "@tanstack/react-query";
import { ListChecks, PencilLine, Save, Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import type { RuleTestResult_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useBridge } from "@/dev/bridge";
import { deleteRule, openRewrite, saveRule } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { isVisual } from "@/lib/rewrite/diff";
import { isBuiltin, normalizeRule, type Rule } from "@/lib/rules";
import { useUi } from "@/store/ui";

import { ActionListEditor } from "./ActionListEditor";
import { Choice } from "./fields";
import { MatcherBuilder } from "./MatcherBuilder";
import { RuleList, type RuleGroup } from "./RuleList";
import { RuleOriginBadge } from "./RuleOriginBadge";
import { RuleTestPanel } from "./RuleTestPanel";
import { queries } from "@/lib/queries";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The Rules Editor: the ordered rule list (drag to reorder) beside the
 * selected rule's matcher, actions and dry run. Presets from the flow menu
 * open here as unsaved drafts.
 */
export function RulesEditor() {
  const { open, ruleId, draft, seq } = useUi((s) => s.rulesEditor);
  const close = useUi((s) => s.closeRulesEditor);
  const { data: rules = [] } = useQuery({ ...queries.rules(), enabled: open });
  const { data: groups = [] } = useQuery({ ...queries.ruleGroups(), enabled: open });
  const userRules = rules.filter((r) => !isBuiltin(r)).map(normalizeRule);
  const current = draft ?? userRules.find((r) => r.id === ruleId) ?? null;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[84vh] flex-col gap-0 p-0 sm:max-w-[1180px]"
        initialFocus="content"
        data-testid="rules-editor"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <ListChecks className="size-4" />
            Rules
          </DialogTitle>
          <DialogDescription>
            Every matching rule runs, top to bottom. Map Local, rewrites, breakpoints, Map Remote, throttling and
            blocking are all actions. Groups turn their rules on and off together.
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          <RuleList rules={userRules} groups={groups} selected={draft ? null : ruleId} hasDraft={!!draft} />
          <div className="min-w-0 flex-1">
            {current ? (
              <RuleForm key={`${seq}-${current.id}`} initial={current} groups={groups} />
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

const NO_GROUP = "none";

function RuleForm({ initial, groups }: { initial: Rule; groups: RuleGroup[] }) {
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
  // On/off belongs to the list's checkbox, which saves at once: the form
  // neither marks it as an edit nor saves the state it was opened with.
  // Likewise the list reorders and renumbers rules and moves them between
  // groups: the form keeps the current order, and claims the group only
  // once its own picker changes it.
  const [groupPicked, setGroupPicked] = useState(false);
  const live: Rule = {
    ...rule,
    enabled: initial.enabled,
    order: initial.order,
    group: groupPicked ? rule.group : initial.group,
  };
  const unsaved = !rule.id || JSON.stringify(live) !== JSON.stringify(initial);

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
      const saved = await saveRule(live);
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
  useBridge({ saveRule: save, testRule: runTest });

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
        {!initial.enabled && (
          <span className="text-muted-foreground text-xs" title="Turn it on with its checkbox in the list" data-testid="rule-disabled">
            Disabled
          </span>
        )}
        <RuleOriginBadge origin={rule.origin} />
        {groups.length > 0 && (
          <Choice
            value={live.group && groups.some((g) => g.id === live.group) ? live.group : NO_GROUP}
            options={[{ value: NO_GROUP, label: "No group" }, ...groups.map((g) => ({ value: g.id, label: g.name }))]}
            onChange={(group) => {
              setGroupPicked(true);
              setRule({ ...rule, group: group === NO_GROUP ? null : group });
            }}
            className="h-7 max-w-48 text-xs"
            testId="rule-group-picker"
          />
        )}
        {groups.some((g) => g.id === live.group && !g.enabled) && (
          <span className="text-muted-foreground text-xs" title="Turn the group on to run its rules" data-testid="rule-group-off">
            Group off
          </span>
        )}
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
