import { Bot } from "lucide-react";
import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { startMcp, useMcp } from "@/store/mcp";
import { useUi } from "@/store/ui";

/** The toolbar's MCP status: how many agents are connected. Opens Settings › MCP. */
export function McpIndicator() {
  const status = useMcp((s) => s.status);
  const openSettings = useUi((s) => s.openSettings);
  useEffect(startMcp, []);
  const clients = status?.clients ?? [];
  const serving = !!status?.url;
  const readOnly = !!status?.settings.read_only;
  const tip = !status
    ? "MCP server"
    : !status.settings.enabled
      ? "MCP server off · open Settings to turn it on"
      : !serving
        ? `MCP server not running${status.error ? `: ${status.error}` : ""}`
        : clients.length === 0
          ? `MCP server on ${status.url}${readOnly ? " · read-only" : ""} · no agents connected`
          : `MCP: ${clients.map((c) => c.name).join(", ")}${readOnly ? " · read-only" : ""}`;
  const color = !serving
    ? status?.error
      ? "text-[var(--s-5xx)]"
      : "text-muted-foreground opacity-60"
    : clients.length > 0
      ? "text-[var(--rule-agent)]"
      : "text-muted-foreground";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size={clients.length ? "sm" : "icon"}
          aria-label="MCP server"
          onClick={() => openSettings("mcp")}
          data-testid="mcp-indicator"
          className={color}
        >
          <Bot />
          {clients.length > 0 && (
            <span
              className="rounded-full px-1.5 font-mono text-2xs font-semibold"
              style={{ color: "var(--rule-agent)", backgroundColor: "var(--rule-agent-bg)" }}
              data-testid="mcp-client-count"
            >
              {clients.length}
            </span>
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}
