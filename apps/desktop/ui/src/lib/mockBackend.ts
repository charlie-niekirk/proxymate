import type {
  ContentGroup,
  ContentKind,
  FilterSpec_Deserialize,
  FlowRow_Serialize,
  FlowSummary_Serialize,
  ViewDelta,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

/**
 * Browser-only stand-in for the engine, so the UI runs under plain Vite and in
 * tests. Static data, no live traffic, no sorting. The real logic is in Rust.
 */
export function createMockBackend(count: number): Backend {
  const hosts = ["api.github.com", "www.google.com", "api.stripe.com", "slack.com", "localhost", "i.ytimg.com"];
  const kinds: ContentKind[] = ["json", "html", "javascript", "image", "css", "xml", "graphql", "none"];
  const methods = ["GET", "GET", "GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
  const statuses = [200, 200, 200, 201, 204, 304, 404, 500];
  const now = Date.now();
  const flows: FlowRow_Serialize[] = Array.from({ length: count }, (_, i) => {
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
  };
}
