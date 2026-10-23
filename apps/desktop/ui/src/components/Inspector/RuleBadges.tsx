import { Bot, FileCode2, PencilLine, TriangleAlert } from "lucide-react";

import type { FlowRules_Serialize } from "@/bindings";
import { useUi } from "@/store/ui";

/**
 * What rules did to the inspected flow: one badge per applied rule (click
 * to open it), plus why a body was left alone, a timed-out breakpoint, a
 * mocked response, or a script that failed (the flow went on without it).
 */
export function RuleBadges({ rules }: { rules: FlowRules_Serialize }) {
  const open = useUi((s) => s.openRulesEditor);
  const openScripts = useUi((s) => s.openScripts);
  const failed = rules.scripts ?? [];
  const notes = [
    rules.mocked && "Mocked: the response didn't come from the upstream.",
    rules.breakpoint_timeout && "A breakpoint timed out; the flow continued unmodified.",
    rules.rewrite_skipped,
  ].filter((n): n is string => !!n);
  if (rules.applied.length === 0 && notes.length === 0 && failed.length === 0) return null;
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-1 border-b px-2 py-1 text-xs" data-testid="rule-badges">
      <PencilLine className="size-3 text-[var(--rule)]" />
      <span className="text-muted-foreground mr-1">Rules</span>
      {rules.applied.map((r) => {
        const client = r.origin.kind === "agent" ? r.origin.client : null;
        return (
          <button
            key={`${r.rule_id}-${r.phase}`}
            type="button"
            onClick={() => open({ ruleId: r.rule_id })}
            title={`${r.name} changed the ${r.phase}${client !== null ? ` · created by ${client}` : ""}`}
            className="inline-flex h-5 items-center gap-1 rounded-sm px-1.5 hover:brightness-110"
            style={
              client !== null
                ? { color: "var(--rule-agent)", backgroundColor: "var(--rule-agent-bg)" }
                : { color: "var(--rule)", backgroundColor: "var(--rule-bg)" }
            }
            data-testid="rule-badge"
          >
            {client !== null && <Bot className="size-3" />}
            {client !== null && <span className="font-semibold">AI · {client}</span>}
            <span>{r.name}</span>
            <span className="opacity-60">{r.phase === "request" ? "req" : "resp"}</span>
          </button>
        );
      })}
      {failed.map((issue) => (
        <button
          key={`${issue.script_id}-${issue.phase}`}
          type="button"
          onClick={() => openScripts({ scriptId: issue.script_id })}
          title={issue.message}
          className="inline-flex h-5 max-w-full items-center gap-1 rounded-sm px-1.5 hover:brightness-110"
          style={{ color: "var(--s-5xx)", backgroundColor: "var(--s-5xx-bg)" }}
          data-testid="script-issue"
        >
          <FileCode2 className="size-3 shrink-0" />
          <span className="shrink-0 font-medium">Script “{issue.script_name}” failed</span>
          <span className="truncate opacity-80">
            {issue.phase} · {issue.message.split("\n")[0]}
          </span>
        </button>
      ))}
      {notes.map((n) => (
        <span key={n} className="text-muted-foreground inline-flex items-center gap-1" data-testid="rule-note">
          <TriangleAlert className="size-3 text-[var(--s-4xx)]" />
          {n}
        </span>
      ))}
    </div>
  );
}
