import type {
  ContentGroup,
  ContentKind,
  FilterSpec_Deserialize,
  FlowDetail_Serialize,
  FlowRow_Serialize,
  FlowSummary_Serialize,
  ProxyStatus_Serialize,
  ViewDelta,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

/**
 * Browser-only stand-in for the engine, so the UI runs under plain Vite and in
 * tests. Static data, no live traffic, no sorting. The real logic is in Rust.
 */
export function createMockBackend(count: number): Backend {
  let recording = true;
  const status = (): ProxyStatus_Serialize => ({ listening: "127.0.0.1:9090", error: null, recording });
  const hosts = ["api.github.com", "www.google.com", "api.stripe.com", "slack.com", "localhost", "i.ytimg.com"];
  const kinds: ContentKind[] = ["json", "html", "javascript", "image", "css", "xml", "graphql", "none"];
  const methods = ["GET", "GET", "GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
  const statuses = [200, 200, 200, 201, 204, 304, 404, 500];
  const now = Date.now();
  let flows: FlowRow_Serialize[] = Array.from({ length: count }, (_, i) => {
    const host = hosts[i % hosts.length]!;
    const https = host !== "localhost";
    return [
      i,
      "completed",
      https ? "https" : "http",
      methods[(i * 7) % methods.length]!,
      host,
      https ? 443 : 3000,
      `/v1/items/${i % 997}`,
      i % 3 === 0 ? `page=${i % 20}` : "",
      statuses[(i * 13) % statuses.length]!,
      kinds[(i * 5) % kinds.length]!,
      "Mock",
      now - (count - i) * 20,
      (i * 37) % 900,
      0,
      (i * 131) % 90_000,
      false,
    ];
  });

  const views = new Map<number, number[]>();
  let nextView = 0;

  const groupOf = (k: ContentKind): ContentGroup =>
    k === "javascript" ? "js" : k === "html" ? "document" : k === "image" ? "media" : k === "none" ? "other" : (k as ContentGroup);

  function refilter(filter: FilterSpec_Deserialize): number[] {
    const text = filter.text.trim().toLowerCase();
    const out: number[] = [];
    for (const f of flows) {
      if (filter.protocol === "http" && f[2] !== "http") continue;
      if (filter.protocol === "https" && f[2] !== "https") continue;
      if (filter.protocol === "websocket") continue;
      if (filter.content.length > 0 && !filter.content.includes(groupOf(f[9]))) continue;
      if (filter.host && f[4] !== filter.host) continue;
      if (text && !`${f[4]}${f[6]}${f[7]}`.toLowerCase().includes(text)) continue;
      out.push(f[0]);
    }
    return out;
  }

  function info(view: number, start: number) {
    return { view, total: flows.length, len: views.get(view)!.length, elapsed_us: Math.round((performance.now() - start) * 1000) };
  }

  return {
    async createView(filter: FilterSpec_Deserialize) {
      const start = performance.now();
      const view = ++nextView;
      views.set(view, refilter(filter));
      return info(view, start);
    },
    async setFilter(view, filter) {
      const start = performance.now();
      views.set(view, refilter(filter));
      return info(view, start);
    },
    async setSort(view) {
      return info(view, performance.now());
    },
    async dropView(view) {
      views.delete(view);
    },
    async getRows(view, start, n) {
      const ids = views.get(view) ?? [];
      return { view, start, len: ids.length, rows: ids.slice(start, start + n).map((id) => flows[id]!) };
    },
    async getFlow(id): Promise<FlowSummary_Serialize | null> {
      const f = flows[id];
      if (!f) return null;
      return {
        id: f[0], state: f[1], scheme: f[2], method: f[3], host: f[4], port: f[5], path: f[6], query: f[7],
        status: f[8], content_kind: f[9], client: f[10], started_at_ms: f[11], duration_ms: f[12],
        req_size: f[13], resp_size: f[14], edited: f[15],
      };
    },
    async subscribe(view, onDelta: (d: ViewDelta) => void) {
      const len = views.get(view)?.length ?? 0;
      setTimeout(() => onDelta({ view, seq: 1, total: flows.length, len, appended: 0, changed: [], structural: true }), 0);
    },
    async getFlowDetail(id): Promise<FlowDetail_Serialize | null> {
      const f = flows[id];
      if (!f) return null;
      const body = mockBody(f[9]);
      return {
        id,
        request: {
          method: f[3],
          url: `${f[2]}://${f[4]}${f[6]}${f[7] ? `?${f[7]}` : ""}`,
          version: "HTTP/1.1",
          headers: [
            { name: "host", value: f[4] },
            { name: "user-agent", value: "curl/8.7.1" },
            { name: "accept", value: "*/*" },
          ],
        },
        response: {
          status: f[8] ?? 200,
          reason: "OK",
          version: "HTTP/1.1",
          headers: [
            { name: "content-type", value: f[9] === "json" ? "application/json" : "text/plain" },
            { name: "content-length", value: String(body.length) },
          ],
        },
        request_body: { size: 0, captured: 0, truncated: false, encoding: null, complete: true },
        response_body: { size: body.length, captured: body.length, truncated: false, encoding: null, complete: true },
        error: null,
      };
    },
    async getBody(id, part) {
      const f = flows[id];
      const bytes = f && part === "response" ? new TextEncoder().encode(mockBody(f[9])) : new Uint8Array();
      return bytes.buffer as ArrayBuffer;
    },
    async clearFlows() {
      flows = [];
      for (const view of views.keys()) views.set(view, []);
    },
    async setRecording(on) {
      recording = on;
      return status();
    },
    async getProxyStatus() {
      return status();
    },
  };
}

function mockBody(kind: ContentKind): string {
  return kind === "json" ? JSON.stringify({ mock: true, items: [1, 2, 3], nested: { ok: true } }) : "mock body\n";
}
