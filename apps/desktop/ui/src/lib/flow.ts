import type { ContentKind, FlowRow_Serialize, FlowState, Scheme } from "@/bindings";

/** A decoded `FlowRow` tuple. Field order mirrors `pm_model::FlowRow`. */
export interface Row {
  id: number;
  state: FlowState;
  scheme: Scheme;
  method: string;
  host: string;
  port: number;
  path: string;
  query: string;
  status: number | null;
  contentKind: ContentKind;
  client: string | null;
  startedAtMs: number | null;
  durationMs: number | null;
  reqSize: number;
  respSize: number;
  edited: boolean;
}

export function decodeRow(t: FlowRow_Serialize): Row {
  return {
    id: t[0],
    state: t[1],
    scheme: t[2],
    method: t[3],
    host: t[4],
    port: t[5],
    path: t[6],
    query: t[7],
    status: t[8],
    contentKind: t[9],
    client: t[10],
    startedAtMs: t[11],
    durationMs: t[12],
    reqSize: t[13],
    respSize: t[14],
    edited: t[15],
  };
}

const DEFAULT_PORTS: Record<Scheme, number> = { http: 80, https: 443, ws: 80, wss: 443 };

/** URL split into the parts the table colourises. */
export interface UrlParts {
  scheme: string;
  host: string;
  port: string;
  path: string;
  query: string;
}

export function urlParts(r: Pick<Row, "scheme" | "host" | "port" | "path" | "query">): UrlParts {
  return {
    scheme: `${r.scheme}://`,
    host: r.host,
    port: r.port === DEFAULT_PORTS[r.scheme] ? "" : `:${r.port}`,
    path: r.path,
    query: r.query ? `?${r.query}` : "",
  };
}

const KNOWN_METHODS = new Set(["get", "post", "put", "patch", "delete", "head", "options", "connect"]);

/** Suffix of the `--m-*` colour tokens for a method. */
export function methodToken(method: string): string {
  const m = method.toLowerCase();
  return KNOWN_METHODS.has(m) ? m : "other";
}

/** Suffix of the `--s-*` colour tokens for a flow's status pill. */
export function statusToken(status: number | null, state: FlowState): string {
  switch (state) {
    case "active":
    case "paused":
    case "blocked":
    case "tunnel":
      return state;
    case "failed":
      return "failed";
    case "completed":
      if (status === null) return "1xx";
      if (status >= 500) return "5xx";
      if (status >= 400) return "4xx";
      if (status >= 300) return "3xx";
      if (status >= 200) return "2xx";
      return "1xx";
  }
}

/** Text shown in the status pill. */
export function statusLabel(status: number | null, state: FlowState): string {
  switch (state) {
    case "active":
      return "…";
    case "paused":
      return "Paused";
    case "failed":
      return "Failed";
    case "tunnel":
      return "Tunnel";
    case "blocked":
    case "completed":
      return status === null ? "—" : String(status);
  }
}

export function formatSize(bytes: number): string {
  if (bytes === 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(2)} s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

export function formatTime(ms: number | null): string {
  if (ms === null) return "—";
  const d = new Date(ms);
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}
