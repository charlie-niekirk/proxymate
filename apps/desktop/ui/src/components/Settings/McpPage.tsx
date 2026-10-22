import { useQuery } from "@tanstack/react-query";
import { Bot, KeyRound, LoaderCircle, RefreshCw, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { McpAuditEntry_Serialize, McpCallOutcome, McpSettings, McpStatus_Serialize } from "@/bindings";
import { CopyField } from "@/components/DeviceSetup/parts";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { backend } from "@/lib/backend";
import { claudeAddCommand, mcpJsonConfig, maskToken } from "@/lib/mcp";
import { mcpAuditKey, mcpTokenKey, queryClient } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useMcp } from "@/store/mcp";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * Settings › MCP: the server on/off, read-only mode, how to connect an
 * agent (with the bearer token), connected clients and the audit log.
 */
export function McpPage() {
  const status = useMcp((s) => s.status);
  const [error, setError] = useState<string | null>(null);
  if (!status) {
    return (
      <div className="text-muted-foreground flex h-full items-center justify-center text-xs">
        <LoaderCircle className="mr-2 size-4 animate-spin" /> Loading…
      </div>
    );
  }
  const save = async (settings: McpSettings) => {
    setError(null);
    try {
      await backend.setMcpSettings(settings);
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="flex flex-col gap-5 p-4 text-xs" data-testid="settings-mcp">
      <Section
        title="MCP server"
        description="Lets AI agents (Claude Code, Cursor, the MCP Inspector…) read recorded traffic, manage rules and resolve breakpoints."
        aside={
          <Switch
            checked={status.settings.enabled}
            onCheckedChange={(enabled) => void save({ ...status.settings, enabled })}
            aria-label="Enable the MCP server"
            data-testid="mcp-enabled"
          />
        }
      >
        <ServingLine status={status} />
      </Section>

      <Section
        title="Read-only"
        description="Agents can still inspect traffic and rules, but can't create or change rules, resolve breakpoints, pause recording or clear flows. Connected clients see their tool list change straight away."
        aside={
          <Switch
            checked={status.settings.read_only}
            onCheckedChange={(read_only) => void save({ ...status.settings, read_only })}
            aria-label="Read-only mode"
            data-testid="mcp-read-only"
          />
        }
      />
      {error && <p className="text-[var(--s-5xx)]">{error}</p>}

      <ConnectSection url={status.url ?? "http://127.0.0.1:9095/mcp"} />
      <ClientsSection status={status} />
      <AuditSection />
    </div>
  );
}

function Section({
  title,
  description,
  aside,
  children,
}: {
  title: string;
  description?: ReactNode;
  aside?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-semibold">{title}</h3>
          {description && <p className="text-muted-foreground mt-0.5 leading-4">{description}</p>}
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

function ServingLine({ status }: { status: McpStatus_Serialize }) {
  const [dot, text] = status.url
    ? ["var(--s-2xx)", `Serving on ${status.url}`]
    : status.error
      ? ["var(--s-5xx)", `Not serving: ${status.error}`]
      : ["var(--muted-foreground)", status.settings.enabled ? "Starting…" : "Off"];
  return (
    <div className="flex items-center gap-2 rounded-md border px-2 py-1.5" data-testid="mcp-serving">
      <span className="size-1.5 shrink-0 rounded-full" style={{ backgroundColor: dot }} />
      <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{text}</span>
      {status.settings.read_only && <span className="text-muted-foreground">read-only</span>}
    </div>
  );
}

function ConnectSection({ url }: { url: string }) {
  const { data: token } = useQuery({ queryKey: mcpTokenKey, queryFn: () => backend.getMcpToken() });
  const [confirm, setConfirm] = useState(false);
  const regenerate = async () => {
    queryClient.setQueryData(mcpTokenKey, await backend.regenerateMcpToken());
    setConfirm(false);
  };
  if (!token) return null;
  return (
    <Section
      title="Connect an agent"
      description="Each request needs this install's bearer token. Only loopback clients without a foreign Origin are accepted."
    >
      <Label>Claude Code</Label>
      <CopyField
        value={claudeAddCommand(url, token)}
        display={claudeAddCommand(url, maskToken(token))}
        label="Copy the claude mcp add command"
        testId="mcp-copy-claude"
      />
      <Label>JSON config (Cursor, Claude Desktop, .mcp.json…)</Label>
      <CopyField
        value={mcpJsonConfig(url, token)}
        display={mcpJsonConfig(url, maskToken(token))}
        label="Copy the JSON config"
        multiline
        testId="mcp-copy-json"
      />
      <p className="text-muted-foreground leading-4">
        Clients that only speak stdio can run <code className="font-mono">proxymate mcp-stdio</code> (the app binary),
        which reads the token itself and starts Proxymate if it isn&apos;t running.
      </p>
      <div className="flex items-center gap-2">
        {confirm ? (
          <>
            <span className="text-[var(--s-4xx)]">Connected agents will need the new token.</span>
            <Button size="xs" variant="destructive" onClick={() => void regenerate()} data-testid="mcp-regenerate-confirm">
              Regenerate
            </Button>
            <Button size="xs" variant="ghost" onClick={() => setConfirm(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <Button size="xs" variant="outline" onClick={() => setConfirm(true)} data-testid="mcp-regenerate">
            <KeyRound />
            Regenerate token
          </Button>
        )}
      </div>
    </Section>
  );
}

function Label({ children }: { children: ReactNode }) {
  return <div className="text-muted-foreground mt-1 text-2xs font-medium tracking-wide uppercase">{children}</div>;
}

function ClientsSection({ status }: { status: McpStatus_Serialize }) {
  return (
    <Section title={`Connected clients (${status.clients.length})`}>
      {status.clients.length === 0 ? (
        <p className="text-muted-foreground">No agent is connected.</p>
      ) : (
        <ul className="flex flex-col divide-y rounded-md border" data-testid="mcp-clients">
          {status.clients.map((c) => (
            <li key={c.session} className="flex items-center gap-2 px-2 py-1.5">
              <Bot className="size-3.5 text-[var(--rule-agent)]" />
              <span className="font-medium">{c.name}</span>
              <span className="text-muted-foreground">{c.version}</span>
              <span className="text-muted-foreground ml-auto font-mono text-2xs">
                session {c.session} · since {new Date(c.connected_at_ms ?? 0).toLocaleTimeString()}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

const OUTCOME_STYLE: Record<McpCallOutcome, { color: string; bg: string }> = {
  ok: { color: "var(--s-2xx)", bg: "var(--s-2xx-bg)" },
  error: { color: "var(--s-5xx)", bg: "var(--s-5xx-bg)" },
  refused: { color: "var(--s-4xx)", bg: "var(--s-4xx-bg)" },
};

function AuditSection() {
  const [limit, setLimit] = useState(50);
  const { data, isFetching, refetch } = useQuery({
    queryKey: [...mcpAuditKey, limit],
    queryFn: () => backend.getMcpAudit(null, limit),
    // Calls keep arriving while agents work.
    refetchInterval: 2000,
  });
  const clear = async () => {
    await backend.clearMcpAudit();
    await queryClient.invalidateQueries({ queryKey: mcpAuditKey });
  };
  const entries = data?.entries ?? [];
  return (
    <Section
      title="Audit log"
      description="Every tool call, newest first, with the client that made it."
      aside={
        <div className="flex gap-1">
          <Button size="icon-xs" variant="ghost" aria-label="Refresh" onClick={() => void refetch()}>
            <RefreshCw className={cn(isFetching && "animate-spin")} />
          </Button>
          <Button size="xs" variant="ghost" onClick={() => void clear()} disabled={entries.length === 0}>
            <Trash2 />
            Clear
          </Button>
        </div>
      }
    >
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No calls yet.</p>
      ) : (
        <div className="overflow-hidden rounded-md border" data-testid="mcp-audit">
          <table className="w-full table-fixed text-[11px]">
            <thead className="bg-muted/50 text-muted-foreground text-left">
              <tr>
                <th className="w-[72px] px-2 py-1 font-medium">Time</th>
                <th className="w-[96px] px-2 py-1 font-medium">Client</th>
                <th className="w-[140px] px-2 py-1 font-medium">Tool</th>
                <th className="px-2 py-1 font-medium">Arguments</th>
                <th className="w-[64px] px-2 py-1 font-medium">Result</th>
                <th className="w-[52px] px-2 py-1 text-right font-medium">ms</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {entries.map((e) => (
                <AuditRow key={e.id} entry={e} />
              ))}
            </tbody>
          </table>
        </div>
      )}
      {data?.next !== null && data?.next !== undefined && limit < 1000 && (
        <Button size="xs" variant="ghost" className="self-start" onClick={() => setLimit((l) => l + 100)}>
          Load older calls
        </Button>
      )}
    </Section>
  );
}

function AuditRow({ entry: e }: { entry: McpAuditEntry_Serialize }) {
  const style = OUTCOME_STYLE[e.outcome];
  return (
    <tr className="align-top">
      <td className="text-muted-foreground px-2 py-1 font-mono">{new Date(e.time_ms ?? 0).toLocaleTimeString()}</td>
      <td className="truncate px-2 py-1" title={e.session ? `session ${e.session}` : undefined}>
        {e.client}
      </td>
      <td className="truncate px-2 py-1 font-mono">{e.tool}</td>
      <td className="text-muted-foreground truncate px-2 py-1 font-mono" title={e.error ? `${e.arguments}\n\n${e.error}` : e.arguments}>
        {e.error ? <span className="text-[var(--s-5xx)]">{e.error}</span> : e.arguments}
      </td>
      <td className="px-2 py-1">
        <span className="rounded-sm px-1 py-px text-2xs font-medium" style={{ color: style.color, backgroundColor: style.bg }}>
          {e.outcome}
        </span>
      </td>
      <td className="text-muted-foreground px-2 py-1 text-right font-mono">{e.duration_ms}</td>
    </tr>
  );
}
