import { useMutation, useQuery } from "@tanstack/react-query";
import { Bot, ChevronDown, ChevronRight, FolderPlus, GripVertical, Plus } from "lucide-react";
import { useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";

import type { RuleGroup_Serialize } from "@/bindings";
import { CopyItem } from "@/components/menus/items";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Input } from "@/components/ui/input";
import { deleteAgentRules, deleteRule, duplicateRule, openRewrite, reorderRules, reportError, setRuleEnabled } from "@/lib/actions";
import { runMutation, mutations } from "@/lib/mutations";
import { queries } from "@/lib/queries";
import { isVisual } from "@/lib/rewrite/diff";
import { actionSummary, matcherSummary, newRule, type Rule } from "@/lib/rules";
import { cn } from "@/lib/utils";
import { useMockSourceNames } from "@/hooks/useMocking";
import { useScriptNames } from "@/hooks/useScripts";
import { useUi } from "@/store/ui";

import { NumberInput } from "./fields";
import { RuleOriginBadge } from "./RuleOriginBadge";

export type RuleGroup = RuleGroup_Serialize;

/** What a drag carries: a rule, or a group (by its header). */
type Dragged = { kind: "rule" | "group"; id: string };

/** What a right-click in the list landed on. */
type Target = { kind: "rule"; rule: Rule } | { kind: "group"; group: RuleGroup };

const GROUP_KEY = "group:";
const fail = (what: string) => (e: unknown) => reportError(what, e);

/**
 * The ordered rule list: the ungrouped rules, then each group as a
 * collapsible section with its own on/off switch (the engine runs a rule
 * only while it and its group are on). Rules drag within and between
 * sections; groups drag by their headers.
 */
export function RuleList({
  rules,
  groups,
  selected,
  hasDraft,
}: {
  rules: Rule[];
  groups: RuleGroup[];
  selected: string | null;
  hasDraft: boolean;
}) {
  const openEditor = useUi((s) => s.openRulesEditor);
  const [dragged, setDragged] = useState<Dragged | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [renaming, setRenaming] = useState<string | null>(null);
  const agentRules = rules.filter((r) => r.origin.kind === "agent").length;
  const known = new Set(groups.map((g) => g.id));
  const inGroup = (id: string | null) => rules.filter((r) => (r.group && known.has(r.group) ? r.group : null) === id);
  const ungrouped = inGroup(null);

  const menu = useMenuTarget<Target>((key) => {
    if (key.startsWith(GROUP_KEY)) {
      const group = groups.find((g) => g.id === key.slice(GROUP_KEY.length));
      return group && { kind: "group", group };
    }
    const rule = rules.find((r) => r.id === key);
    return rule && { kind: "rule", rule };
  });

  const toggleCollapsed = (id: string, open?: boolean) =>
    setCollapsed((c) => {
      const next = new Set(c);
      if (open ?? next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const newGroup = async (ruleIds: string[] = []) => {
    try {
      const group = await runMutation(mutations.createRuleGroup, "New group");
      if (ruleIds.length > 0) await runMutation(mutations.moveRulesToGroup, { ids: ruleIds, group: group.id });
      toggleCollapsed(group.id, true);
      setRenaming(group.id);
    } catch (e) {
      reportError("Couldn't create the group", e);
    }
  };

  /**
   * Drops a rule on `target` (a rule): it takes the target's place, so it
   * lands after a rule below it and before one above. A rule from another
   * section moves into the target's first.
   */
  const dropOnRule = async (target: Rule) => {
    if (dragged?.kind !== "rule" || dragged.id === target.id) return;
    const moving = rules.find((r) => r.id === dragged.id);
    if (!moving) return;
    const ids = rules.map((r) => r.id);
    const to = ids.indexOf(target.id);
    ids.splice(ids.indexOf(moving.id), 1);
    ids.splice(to, 0, moving.id);
    try {
      if ((moving.group ?? null) !== (target.group ?? null)) {
        await runMutation(mutations.moveRulesToGroup, { ids: [moving.id], group: target.group ?? null });
      }
      await reorderRules(ids);
    } catch (e) {
      reportError("Couldn't move the rule", e);
    }
  };

  /** Drops on a section's header: a rule moves to its end, a group moves before it. */
  const dropOnSection = (group: RuleGroup | null) => {
    if (!dragged) return;
    if (dragged.kind === "rule") {
      const moving = rules.find((r) => r.id === dragged.id);
      if (moving && (moving.group ?? null) !== (group?.id ?? null)) {
        void runMutation(mutations.moveRulesToGroup, { ids: [moving.id], group: group?.id ?? null }).catch(
          fail("Couldn't move the rule"),
        );
      }
    } else if (group && dragged.id !== group.id) {
      const ids = groups.map((g) => g.id).filter((id) => id !== dragged.id);
      ids.splice(ids.indexOf(group.id), 0, dragged.id);
      void runMutation(mutations.reorderRuleGroups, ids).catch(fail("Couldn't move the group"));
    }
  };

  /** Drag and drop props for a row that takes drops. */
  const dropProps = (key: string, onDrop: () => void) => ({
    onDragOver: (e: DragEvent) => {
      e.preventDefault();
      setOver(key);
    },
    onDragLeave: () => setOver((o) => (o === key ? null : o)),
    onDrop: (e: DragEvent) => {
      e.preventDefault();
      onDrop();
      setOver(null);
    },
  });
  const dragProps = (what: Dragged) => ({
    draggable: true,
    // A press on a checkbox, button or field only does its own thing. WebKit
    // drops the click once a drag has begun, even a cancelled one, so the row
    // stops being draggable for that press (and is again for any other).
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
      e.currentTarget.draggable = !(e.target instanceof HTMLInputElement || (e.target as Element).closest("button"));
    },
    onDragStart: (e: DragEvent) => {
      setDragged(what);
      e.dataTransfer.effectAllowed = "move";
    },
    onDragEnd: () => setDragged(null),
  });

  const row = (rule: Rule, group: RuleGroup | null) => (
    <RuleRow
      key={rule.id}
      rule={rule}
      groupOff={group !== null && !group.enabled}
      indented={group !== null}
      selected={rule.id === selected}
      dropTarget={over === rule.id && dragged?.kind === "rule" && dragged.id !== rule.id}
      dragging={dragged?.kind === "rule" && dragged.id === rule.id}
      onOpen={() => openEditor({ ruleId: rule.id })}
      {...dragProps({ kind: "rule", id: rule.id })}
      {...dropProps(rule.id, () => void dropOnRule(rule))}
    />
  );

  return (
    <div className="bg-muted/20 flex w-80 shrink-0 flex-col border-r">
      <div className="flex items-center gap-1 border-b p-1.5">
        <Button variant="outline" size="xs" onClick={() => openEditor({ draft: newRule() })} data-testid="new-rule">
          <Plus />
          New rule
        </Button>
        <Button
          variant="ghost"
          size="xs"
          title="A named group of rules that turn on and off together"
          onClick={() => void newGroup()}
          data-testid="new-rule-group"
        >
          <FolderPlus />
          New group
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
            Remove agent rules
          </Button>
        )}
      </div>
      {/* A plain scroller: ScrollArea's table layout defeats truncation. */}
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div className="min-h-0 flex-1 overflow-y-auto" onContextMenu={menu.onContextMenu}>
            <div className="flex flex-col p-1" data-testid="rule-list">
              {hasDraft && (
                <div className="bg-selection text-selection-foreground mb-0.5 rounded-sm px-2 py-1 text-xs italic">
                  Unsaved rule
                </div>
              )}
              {rules.length === 0 && groups.length === 0 && !hasDraft && (
                <div className="text-muted-foreground p-2 text-xs">No rules yet.</div>
              )}
              {groups.length > 0 && (ungrouped.length > 0 || dragged?.kind === "rule") && (
                <SectionLabel
                  dropTarget={over === "ungrouped" && dragged?.kind === "rule"}
                  {...dropProps("ungrouped", () => dropOnSection(null))}
                >
                  Ungrouped
                </SectionLabel>
              )}
              {ungrouped.map((rule) => row(rule, null))}
              {groups.map((group) => {
                const members = inGroup(group.id);
                const open = !collapsed.has(group.id);
                return (
                  <div key={group.id} className="mt-1 flex flex-col" data-testid="rule-group">
                    <GroupHeader
                      group={group}
                      members={members}
                      open={open}
                      renaming={renaming === group.id}
                      dropTarget={over === GROUP_KEY + group.id && !(dragged?.kind === "group" && dragged.id === group.id)}
                      dragging={dragged?.kind === "group" && dragged.id === group.id}
                      onToggleOpen={() => toggleCollapsed(group.id)}
                      onRename={() => setRenaming(group.id)}
                      onRenamed={() => setRenaming(null)}
                      {...dragProps({ kind: "group", id: group.id })}
                      {...dropProps(GROUP_KEY + group.id, () => dropOnSection(group))}
                    />
                    {open && members.map((rule) => row(rule, group))}
                    {open && members.length === 0 && (
                      <div className="text-muted-foreground py-1 pr-2 pl-8 text-2xs italic">
                        No rules: drag rules here, or use a rule&apos;s Move to Group menu.
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="min-w-52" data-testid="rule-menu">
          {menu.target?.kind === "rule" && (
            <RuleItems rule={menu.target.rule} rules={rules} groups={groups} onNewGroup={(ids) => void newGroup(ids)} />
          )}
          {menu.target?.kind === "group" && (
            <GroupItems
              group={menu.target.group}
              groups={groups}
              held={inGroup(menu.target.group.id).length}
              onRename={setRenaming}
            />
          )}
        </ContextMenuContent>
      </ContextMenu>
      <BreakpointTimeout />
    </div>
  );
}

type RowEvents = {
  draggable: boolean;
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onDragStart: (e: DragEvent) => void;
  onDragEnd: () => void;
  onDragOver: (e: DragEvent) => void;
  onDragLeave: () => void;
  onDrop: (e: DragEvent) => void;
};

function SectionLabel({ children, dropTarget, ...events }: { children: ReactNode; dropTarget: boolean } & Partial<RowEvents>) {
  return (
    <div
      className={cn(
        "text-muted-foreground rounded-sm px-2 pt-1 pb-0.5 text-2xs font-semibold tracking-wide uppercase",
        dropTarget && "ring-primary ring-1",
      )}
      data-testid="rule-section-ungrouped"
      {...events}
    >
      {children}
    </div>
  );
}

function RuleRow({
  rule,
  groupOff,
  indented,
  selected,
  dropTarget,
  dragging,
  onOpen,
  ...events
}: {
  rule: Rule;
  groupOff: boolean;
  indented: boolean;
  selected: boolean;
  dropTarget: boolean;
  dragging: boolean;
  onOpen: () => void;
} & RowEvents) {
  const scriptNames = useScriptNames();
  const mockNames = useMockSourceNames();
  const off = !rule.enabled || groupOff;
  return (
    // eslint-disable-next-line jsx-a11y-x/click-events-have-key-events, jsx-a11y-x/no-static-element-interactions -- a pointer shortcut; keyboard list navigation is an R4 follow-up
    <div
      {...events}
      onClick={onOpen}
      data-selected={selected}
      data-menu-key={rule.id}
      data-testid="rule-row"
      className={cn(
        "group flex cursor-default items-start gap-1 rounded-sm px-1 py-1 text-xs",
        indented && "pl-4",
        "hover:bg-accent data-[selected=true]:bg-selection data-[selected=true]:text-selection-foreground",
        dropTarget && "ring-primary ring-1",
        dragging && "opacity-50",
      )}
    >
      <GripVertical className="text-muted-foreground mt-0.5 size-3 shrink-0 opacity-50" />
      <input
        type="checkbox"
        className="mt-0.5 size-3 shrink-0 accent-[var(--primary)]"
        checked={rule.enabled}
        title={
          groupOff
            ? "Its group is off: the rule runs once the group is back on"
            : rule.enabled
              ? "Enabled: click to turn it off"
              : "Disabled: click to turn it on"
        }
        aria-label={`${rule.name || "Untitled"} enabled`}
        // Toggles without selecting the row; the row's onPointerDown keeps a
        // shaky press from starting a drag.
        onClick={(e) => e.stopPropagation()}
        onChange={(e) => void setRuleEnabled(rule.id, e.target.checked).catch(fail("Couldn't change the rule"))}
        data-testid="rule-enabled"
      />
      {/* An off rule is muted rather than faded, so its text keeps 4.5:1 (R4). */}
      <div className={cn("min-w-0 flex-1", off && !selected && "text-muted-foreground")}>
        <div className="flex items-center gap-1">
          <span className="truncate font-medium">{rule.name || "Untitled"}</span>
          <RuleOriginBadge origin={rule.origin} />
        </div>
        <div className={cn("truncate font-mono text-2xs", !off && "opacity-70")}>{matcherSummary(rule.matcher)}</div>
        <div className={cn("truncate text-2xs", !off && "opacity-70")}>
          {rule.actions.map((a) => actionSummary(a, scriptNames, mockNames)).join(" · ") || "no actions"}
        </div>
      </div>
    </div>
  );
}

function GroupHeader({
  group,
  members,
  open,
  renaming,
  dropTarget,
  dragging,
  onToggleOpen,
  onRename,
  onRenamed,
  ...events
}: {
  group: RuleGroup;
  members: Rule[];
  open: boolean;
  renaming: boolean;
  dropTarget: boolean;
  dragging: boolean;
  onToggleOpen: () => void;
  onRename: () => void;
  onRenamed: () => void;
} & RowEvents) {
  const on = members.filter((r) => r.enabled).length;
  const toggle = useMutation({ ...mutations.updateRuleGroup, onError: fail("Couldn't change the group") });
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div
      {...events}
      data-menu-key={GROUP_KEY + group.id}
      data-testid="rule-group-header"
      className={cn(
        "bg-muted/60 flex cursor-default items-center gap-1 rounded-sm px-1 py-1 text-xs",
        dropTarget && "ring-primary ring-1",
        dragging && "opacity-50",
      )}
    >
      <button
        type="button"
        className="text-muted-foreground hover:text-foreground shrink-0 rounded-sm"
        aria-expanded={open}
        aria-label={`${open ? "Collapse" : "Expand"} ${group.name}`}
        onClick={onToggleOpen}
        data-testid="rule-group-toggle-open"
      >
        <Chevron className="size-3.5" />
      </button>
      <input
        type="checkbox"
        className="size-3 shrink-0 accent-[var(--primary)]"
        checked={group.enabled}
        title={
          group.enabled
            ? "On: its enabled rules run. Click to turn the whole group off"
            : "Off: none of its rules run. Click to turn the group on"
        }
        aria-label={`${group.name} group enabled`}
        onChange={(e) => toggle.mutate({ id: group.id, patch: { name: null, enabled: e.target.checked } })}
        data-testid="rule-group-enabled"
      />
      {renaming ? (
        <GroupNameInput group={group} onDone={onRenamed} />
      ) : (
        <span
          className={cn("min-w-0 flex-1 truncate font-semibold", !group.enabled && "text-muted-foreground")}
          onDoubleClick={onRename}
          title={group.name}
          data-testid="rule-group-name"
        >
          {group.name}
        </span>
      )}
      <RuleOriginBadge origin={group.origin} />
      <span
        className="text-muted-foreground shrink-0 text-2xs tabular-nums"
        title={`${on} of ${members.length} rule${members.length === 1 ? "" : "s"} enabled`}
        data-testid="rule-group-count"
      >
        {on === members.length ? members.length : `${on}/${members.length}`}
      </span>
    </div>
  );
}

/** The group's name as a field: Enter or leaving saves, Escape cancels. */
function GroupNameInput({ group, onDone }: { group: RuleGroup; onDone: () => void }) {
  const [name, setName] = useState(group.name);
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const finish = (save: boolean) => {
    if (done.current) return;
    done.current = true;
    onDone();
    if (save && name.trim() && name.trim() !== group.name) {
      void runMutation(mutations.updateRuleGroup, { id: group.id, patch: { name, enabled: null } }).catch(
        fail("Couldn't rename the group"),
      );
    }
  };
  return (
    <Input
      ref={ref}
      value={name}
      onChange={(e) => setName(e.target.value)}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(true);
        if (e.key === "Escape") {
          // Keeps the dialog open: Escape only cancels the rename.
          e.stopPropagation();
          finish(false);
        }
      }}
      className="h-5 min-w-0 flex-1 px-1 text-xs"
      aria-label="Group name"
      data-testid="rule-group-name-input"
    />
  );
}

/** A rule row's menu: what the form and the drag handle do, plus copies. */
function RuleItems({
  rule,
  rules,
  groups,
  onNewGroup,
}: {
  rule: Rule;
  rules: Rule[];
  groups: RuleGroup[];
  onNewGroup: (ruleIds: string[]) => void;
}) {
  const openEditor = useUi((s) => s.openRulesEditor);
  const store = useUi((s) => s.store);
  // Top and bottom of the rule's own section: that's where it can go.
  const section = rules.filter((r) => (r.group ?? null) === (rule.group ?? null));
  const move = (to: "top" | "bottom") => {
    const ids = rules.map((r) => r.id).filter((id) => id !== rule.id);
    void reorderRules(to === "top" ? [rule.id, ...ids] : [...ids, rule.id]).catch(fail("Couldn't move the rule"));
  };
  const moveTo = (group: string | null) =>
    void runMutation(mutations.moveRulesToGroup, { ids: [rule.id], group }).catch(fail("Couldn't move the rule"));
  const index = section.findIndex((r) => r.id === rule.id);
  return (
    <>
      <ContextMenuItem onSelect={() => openEditor({ ruleId: rule.id })}>Open</ContextMenuItem>
      <ContextMenuItem onSelect={() => void setRuleEnabled(rule.id, !rule.enabled).catch(fail("Couldn't change the rule"))} data-testid="rule-menu-toggle">
        {rule.enabled ? "Disable" : "Enable"}
      </ContextMenuItem>
      {(rule.actions.some(isVisual) || (rule.origin.kind === "preset" && rule.origin.preset === "rewrite")) && (
        <ContextMenuItem onSelect={() => void openRewrite(store, null, rule)}>Edit visually…</ContextMenuItem>
      )}
      <ContextMenuItem
        onSelect={() =>
          void duplicateRule(rule)
            .then((copy) => openEditor({ ruleId: copy.id }))
            .catch(fail("Couldn't duplicate the rule"))
        }
        data-testid="rule-menu-duplicate"
      >
        Duplicate
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={index === 0} onSelect={() => move("top")}>
        Move to top
      </ContextMenuItem>
      <ContextMenuItem disabled={index === section.length - 1} onSelect={() => move("bottom")}>
        Move to bottom
      </ContextMenuItem>
      <ContextMenuSub>
        <ContextMenuSubTrigger data-testid="rule-menu-move-to-group">Move to group</ContextMenuSubTrigger>
        <ContextMenuSubContent className="max-w-72 min-w-44">
          {groups.map((g) => (
            <ContextMenuItem key={g.id} disabled={rule.group === g.id} onSelect={() => moveTo(g.id)} data-testid="rule-menu-group">
              <span className="truncate">{g.name}</span>
            </ContextMenuItem>
          ))}
          {groups.length > 0 && <ContextMenuSeparator />}
          <ContextMenuItem onSelect={() => onNewGroup([rule.id])} data-testid="rule-menu-new-group">
            New group…
          </ContextMenuItem>
          <ContextMenuItem disabled={!rule.group} onSelect={() => moveTo(null)} data-testid="rule-menu-ungroup">
            No group
          </ContextMenuItem>
        </ContextMenuSubContent>
      </ContextMenuSub>
      <ContextMenuSeparator />
      <CopyItem text={rule.name} what="rule name">
        Copy name
      </CopyItem>
      <CopyItem text={JSON.stringify(rule, null, 2)} what="rule as JSON">
        Copy as JSON
      </CopyItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        onSelect={() => {
          void deleteRule(rule.id)
            .then(() => openEditor({ ruleId: null }))
            .catch(fail("Couldn't delete the rule"));
        }}
        data-testid="rule-menu-delete"
      >
        Delete
      </ContextMenuItem>
    </>
  );
}

/** A group header's menu. */
function GroupItems({
  group,
  groups,
  held,
  onRename,
}: {
  group: RuleGroup;
  groups: RuleGroup[];
  held: number;
  onRename: (id: string) => void;
}) {
  const openEditor = useUi((s) => s.openRulesEditor);
  const index = groups.findIndex((g) => g.id === group.id);
  const move = (by: -1 | 1) => {
    const ids = groups.map((g) => g.id).filter((id) => id !== group.id);
    ids.splice(index + by, 0, group.id);
    void runMutation(mutations.reorderRuleGroups, ids).catch(fail("Couldn't move the group"));
  };
  const remove = (deleteRules: boolean) =>
    void runMutation(mutations.deleteRuleGroup, { id: group.id, deleteRules }).catch(fail("Couldn't delete the group"));
  return (
    <>
      <ContextMenuItem
        onSelect={() =>
          void runMutation(mutations.updateRuleGroup, { id: group.id, patch: { name: null, enabled: !group.enabled } }).catch(
            fail("Couldn't change the group"),
          )
        }
        data-testid="rule-group-menu-toggle"
      >
        {group.enabled ? "Turn group off" : "Turn group on"}
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => openEditor({ draft: { ...newRule(), group: group.id } })} data-testid="rule-group-menu-new-rule">
        New rule in group
      </ContextMenuItem>
      {/* After the menu closes, so its focus return doesn't steal the field's focus. */}
      <ContextMenuItem onSelect={() => setTimeout(() => onRename(group.id), 0)} data-testid="rule-group-menu-rename">
        Rename…
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={index === 0} onSelect={() => move(-1)}>
        Move up
      </ContextMenuItem>
      <ContextMenuItem disabled={index === groups.length - 1} onSelect={() => move(1)}>
        Move down
      </ContextMenuItem>
      <ContextMenuSeparator />
      <CopyItem text={group.name} what="group name">
        Copy name
      </CopyItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => remove(false)} data-testid="rule-group-menu-ungroup">
        Delete group, keep its rules
      </ContextMenuItem>
      <ContextMenuItem
        variant="destructive"
        disabled={held === 0}
        onSelect={() => remove(true)}
        data-testid="rule-group-menu-delete"
      >
        Delete group and its {held === 1 ? "rule" : `${held} rules`}
      </ContextMenuItem>
    </>
  );
}

function BreakpointTimeout() {
  const { data: settings } = useQuery(queries.ruleSettings());
  const save = useMutation({ ...mutations.setRuleSettings, onError: (e) => reportError("Couldn't set the timeout", e) });
  if (!settings) return null;
  return (
    <div className="text-muted-foreground flex items-center gap-1.5 border-t p-2 text-xs" title="0 waits forever">
      Breakpoint timeout
      <NumberInput
        value={settings.breakpoint_timeout_secs}
        className="w-16"
        aria-label="Breakpoint timeout in seconds"
        onChange={(secs) => save.mutate({ breakpoint_timeout_secs: secs ?? 0 })}
      />
      s
    </div>
  );
}
