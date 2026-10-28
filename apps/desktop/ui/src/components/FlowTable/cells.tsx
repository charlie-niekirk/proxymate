import { Bot, Lock, LockOpen, Repeat2, ShieldAlert } from "lucide-react";
import type { CSSProperties } from "react";

import type { FlowState, SentBy, TlsStatus } from "@/bindings";
import { methodToken, statusLabel, statusToken, urlParts, type Row } from "@/lib/flow";
import { sslTitle } from "@/lib/tls";
import { cn } from "@/lib/utils";

const ON_SELECTED = "group-data-[selected=true]/row:!text-selection-foreground";

function pillStyle(token: string): CSSProperties {
  return { color: `var(--${token})`, backgroundColor: `var(--${token}-bg)` };
}

const pillBase =
  "inline-flex h-4 min-w-0 items-center justify-center rounded-sm px-1.5 font-mono text-2xs font-semibold tracking-wide " +
  "group-data-[selected=true]/row:!bg-[var(--pill-on-selection)] " +
  ON_SELECTED;

export function MethodPill({ method, className }: { method: string; className?: string }) {
  return (
    <span className={cn(pillBase, className)} style={pillStyle(`m-${methodToken(method)}`)}>
      {method}
    </span>
  );
}

export function StatusPill({
  status,
  state,
  grpc = null,
  className,
}: {
  status: number | null;
  state: FlowState;
  /** A gRPC status, which the pill shows by name instead of the HTTP one. */
  grpc?: number | null;
  className?: string;
}) {
  const token = statusToken(status, state, grpc);
  const style: CSSProperties =
    token === "failed"
      ? { color: "var(--s-failed)", boxShadow: "inset 0 0 0 1px var(--s-failed)" }
      : pillStyle(`s-${token}`);
  return (
    <span
      className={cn(pillBase, token === "active" && "animate-pulse", grpc !== null && "max-w-full", className)}
      style={style}
      title={grpc !== null && status !== null ? `gRPC ${statusLabel(status, state, grpc)} (${grpc}) · HTTP ${status}` : undefined}
    >
      {grpc !== null ? <span className="truncate">{statusLabel(status, state, grpc)}</span> : statusLabel(status, state)}
    </span>
  );
}

const DOT_COLOR: Record<FlowState, string> = {
  active: "var(--s-active)",
  paused: "var(--s-paused)",
  failed: "var(--s-failed)",
  blocked: "var(--s-blocked)",
  tunnel: "var(--s-tunnel)",
  completed: "var(--s-2xx)",
};

export function StateDot({ state }: { state: FlowState }) {
  return (
    <span
      title={state}
      className={cn(
        "inline-block size-1.5 rounded-full group-data-[selected=true]/row:!bg-selection-foreground",
        state === "active" && "animate-pulse",
        state === "completed" && "opacity-40",
      )}
      style={{ backgroundColor: DOT_COLOR[state] }}
    />
  );
}

/** Marks a flow Proxymate sent itself: replayed or composed, or by an agent. */
export function SentBadge({ by }: { by: SentBy }) {
  const agent = by === "agent";
  const Icon = agent ? Bot : Repeat2;
  return (
    <span
      className={cn(pillBase, "mr-1.5 gap-0.5 px-1 align-[1px] font-sans tracking-normal")}
      style={pillStyle(agent ? "sent-agent" : "sent")}
      title={agent ? "Sent by an agent through Proxymate" : "Sent from Proxymate (Replay or Compose)"}
      data-testid="sent-badge"
    >
      <Icon className="size-2.5" />
      {agent ? "AI" : "Replay"}
    </span>
  );
}

export function UrlText({
  row,
  operation,
  sentBy,
  tone = "table",
  className,
}: {
  row: Pick<Row, "scheme" | "host" | "port" | "path" | "query">;
  /** GraphQL operation name, shown as a badge before the URL. */
  operation?: string | null;
  /** Who sent a flow Proxymate made itself, shown as a badge first. */
  sentBy?: SentBy | null;
  /** `bar` colours the host blue and the path green (the inspector's URL bar); the table keeps them grey. */
  tone?: "table" | "bar";
  className?: string;
}) {
  const u = urlParts(row);
  const part = cn(ON_SELECTED);
  return (
    <span className={cn("truncate font-mono", className)}>
      {sentBy && <SentBadge by={sentBy} />}
      {operation && (
        <span
          className={cn(pillBase, "mr-1.5 align-[1px] font-sans tracking-normal")}
          style={pillStyle("graphql")}
          title={`GraphQL operation ${operation}`}
          data-testid="graphql-op"
        >
          {operation}
        </span>
      )}
      <span className={part} style={{ color: "var(--url-scheme)" }}>
        {u.scheme}
      </span>
      <span
        className={cn(part, "font-medium")}
        style={{ color: tone === "bar" ? "var(--url-bar-host)" : "var(--url-host)" }}
      >
        {u.host}
      </span>
      {u.port && (
        <span className={part} style={{ color: "var(--url-port)" }}>
          {u.port}
        </span>
      )}
      <span className={part} style={{ color: tone === "bar" ? "var(--url-bar-path)" : "var(--url-path)" }}>
        {u.path}
      </span>
      {u.query && (
        <span className={part} style={{ color: "var(--url-query)" }}>
          {u.query}
        </span>
      )}
    </span>
  );
}

/** The SSL column: decrypted, passed through, or rejected by the client. */
export function SslIcon({ tls }: { tls: TlsStatus }) {
  const selected = "group-data-[selected=true]/row:!text-selection-foreground";
  switch (tls) {
    case "none":
      return null;
    case "intercepted":
      return <Lock aria-label={sslTitle(tls)} className="size-3" />;
    case "passthrough":
      return (
        <LockOpen
          aria-label={sslTitle(tls)}
          className={cn("size-3 text-[var(--ssl-passthrough)]", selected)}
        />
      );
    case "likely_pinned":
      return (
        <ShieldAlert aria-label={sslTitle(tls)} className={cn("size-3 text-[var(--ssl-pinned)]", selected)} />
      );
  }
}
