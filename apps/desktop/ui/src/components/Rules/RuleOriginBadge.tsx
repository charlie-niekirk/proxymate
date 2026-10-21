import { Bot } from "lucide-react";

import type { RuleOrigin_Serialize } from "@/bindings";
import { originLabel } from "@/lib/rules";
import { cn } from "@/lib/utils";

/**
 * Where a rule came from: nothing for the user's own, the preset's name, or
 * "AI · <client>" for a rule an MCP client created (M7).
 */
export function RuleOriginBadge({ origin, className }: { origin: RuleOrigin_Serialize; className?: string }) {
  const label = originLabel(origin);
  if (!label) return null;
  const agent = origin.kind === "agent";
  return (
    <span
      className={cn(
        "inline-flex h-4 shrink-0 items-center gap-0.5 rounded-sm px-1 text-2xs font-medium whitespace-nowrap",
        className,
      )}
      style={
        agent
          ? { color: "var(--rule-agent)", backgroundColor: "var(--rule-agent-bg)" }
          : { color: "var(--muted-foreground)", backgroundColor: "var(--muted)" }
      }
      title={agent ? `Created by the MCP client ${origin.client}` : `Created from the ${label} preset`}
      data-testid={agent ? "rule-agent-badge" : "rule-preset-badge"}
    >
      {agent && <Bot className="size-2.5" />}
      {label}
    </span>
  );
}
