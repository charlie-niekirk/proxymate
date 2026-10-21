import type { McpAuditEntry_Serialize, McpSettings, McpStatus_Serialize } from "@/bindings";
import type { Backend } from "@/lib/backend";

type McpMethods = Pick<
  Backend,
  | "getMcpStatus"
  | "subscribeMcp"
  | "setMcpSettings"
  | "getMcpToken"
  | "regenerateMcpToken"
  | "getMcpAudit"
  | "clearMcpAudit"
>;

const token = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) => b.toString(16).padStart(2, "0")).join("");

/** MCP in the browser mock: serving, one connected agent, a short audit log. */
export function createMockMcp(now: number): McpMethods {
  let settings: McpSettings = { enabled: true, read_only: false };
  let current = token();
  let onStatus: ((s: McpStatus_Serialize) => void) | null = null;
  const tools = ["get_status", "list_flows", "get_flow_body", "create_map_local", "list_rules", "resolve_breakpoint"];
  let audit: McpAuditEntry_Serialize[] = tools.map((tool, i) => ({
    id: i + 1,
    time_ms: now - (tools.length - i) * 4_000,
    client: "claude-code",
    session: "3f2a9c1d7b0e",
    tool,
    arguments: tool === "create_map_local" ? '{"url":"https://api.stripe.com/v1/charges","body":"{\\"ok\\":true}"}' : "{}",
    outcome: "ok",
    error: null,
    duration_ms: 2 + i,
  }));
  const status = (): McpStatus_Serialize => ({
    settings,
    url: settings.enabled ? "http://127.0.0.1:9095/mcp" : null,
    error: null,
    clients: settings.enabled
      ? [{ session: "3f2a9c1d7b0e", name: "claude-code", version: "2.1.0", connected_at_ms: now - 60_000 }]
      : [],
  });
  return {
    async getMcpStatus() {
      return status();
    },
    async subscribeMcp(cb) {
      onStatus = cb;
      setTimeout(() => cb(status()), 0);
    },
    async setMcpSettings(next) {
      settings = next;
      onStatus?.(status());
      return next;
    },
    async getMcpToken() {
      return current;
    },
    async regenerateMcpToken() {
      current = token();
      return current;
    },
    async getMcpAudit(before, limit) {
      const older = audit.filter((e) => before === null || e.id < before).sort((a, b) => b.id - a.id);
      const entries = older.slice(0, limit);
      return { entries, next: older.length > limit ? (entries.at(-1)?.id ?? null) : null };
    },
    async clearMcpAudit() {
      audit = [];
    },
  };
}
