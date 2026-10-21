import type {
  CaInfo,
  ContentGroup,
  ContentKind,
  FilterSpec_Deserialize,
  FlowDetail_Serialize,
  FlowRow_Serialize,
  FlowSummary_Serialize,
  ProxyStatus_Serialize,
  SslSettings,
  TlsInfo_Serialize,
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
    // A few HTTPS flows are tunnels or pinned clients, to exercise the SSL column.
    const tunnel = https && i % 50 === 7;
    const pinned = https && i % 97 === 11;
    const connect = tunnel || pinned;
    return [
      i,
      tunnel ? "tunnel" : pinned ? "failed" : "completed",
      https ? "https" : "http",
      connect ? "CONNECT" : methods[(i * 7) % methods.length]!,
      host,
      https ? 443 : 3000,
      connect ? "" : `/v1/items/${i % 997}`,
      !connect && i % 3 === 0 ? `page=${i % 20}` : "",
      connect ? null : statuses[(i * 13) % statuses.length]!,
      connect ? "none" : kinds[(i * 5) % kinds.length]!,
      "Mock",
      now - (count - i) * 20,
      (i * 37) % 900,
      0,
      (i * 131) % 90_000,
      false,
      https && !connect && i % 3 !== 0 ? "HTTP/2" : "HTTP/1.1",
      tunnel ? "passthrough" : pinned ? "likely_pinned" : https ? "intercepted" : "none",
    ];
  });
  let ssl: SslSettings = { include: ["*"], exclude: ["*.apple.com", "ocsp*"], bypass: [], insecure_upstream: [] };
  const ca: CaInfo = {
    common_name: "Proxymate CA (mock, 2026-10-21)",
    cert_path: "/Users/you/Library/Application Support/Proxymate/ca/proxymate-ca.pem",
    not_before: "2026-10-20T12:00:00Z",
    not_after: "2036-10-18T12:00:00Z",
    sha256: "3F9A".repeat(16),
  };

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
        req_size: f[13], resp_size: f[14], edited: f[15], http_version: f[16], tls: f[17],
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
      const https = f[2] === "https";
      const tls: TlsInfo_Serialize | null = https && f[17] !== "passthrough"
        ? {
            sni: f[4],
            client_alpn: ["h2", "http/1.1"],
            alpn: f[16] === "HTTP/2" ? "h2" : "http/1.1",
            version: "TLSv1.3",
            cipher: "TLS13_AES_128_GCM_SHA256",
            upstream_verified: true,
            chain: [
              mockCert(`CN=${f[4]}`, "CN=Mock Issuing CA 1, O=Mock Trust", [f[4], `*.${f[4]}`]),
              mockCert("CN=Mock Issuing CA 1, O=Mock Trust", "CN=Mock Root CA, O=Mock Trust", []),
            ],
          }
        : null;
      return {
        id,
        request: {
          method: f[3],
          url: `${f[2]}://${f[4]}${f[6]}${f[7] ? `?${f[7]}` : ""}`,
          version: f[16],
          headers: [
            { name: "host", value: f[4] },
            { name: "user-agent", value: "curl/8.7.1" },
            { name: "accept", value: "*/*" },
          ],
        },
        response: {
          status: f[8] ?? 200,
          reason: "OK",
          version: f[16],
          headers: [
            { name: "content-type", value: f[9] === "json" ? "application/json" : "text/plain" },
            { name: "content-length", value: String(body.length) },
          ],
        },
        request_body: { size: 0, captured: 0, truncated: false, encoding: null, complete: true },
        response_body: { size: body.length, captured: body.length, truncated: false, encoding: null, complete: true },
        error: f[17] === "likely_pinned" ? "The client rejected the Proxymate certificate. It sent an unknown_ca alert." : null,
        tls,
        timing: {
          dns_ms: id % 4 === 0 ? 3.2 : null,
          connect_ms: id % 4 === 0 ? 18.4 : null,
          tls_ms: https && id % 4 === 0 ? 41.7 : null,
          ttfb_ms: (f[12] ?? 0) * 0.6,
          transfer_ms: (f[12] ?? 0) * 0.3,
          reused_connection: id % 4 !== 0,
        },
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
    async getCaInfo() {
      return ca;
    },
    async regenerateCa() {
      return ca;
    },
    async getSslSettings() {
      return ssl;
    },
    async setSslSettings(settings) {
      ssl = settings;
      return ssl;
    },
    async addSslBypass(host) {
      if (!ssl.bypass.includes(host)) ssl = { ...ssl, bypass: [...ssl.bypass, host] };
      return ssl;
    },
    async setInsecureUpstream(host, allow) {
      const rest = ssl.insecure_upstream.filter((h) => h !== host);
      ssl = { ...ssl, insecure_upstream: allow ? [...rest, host] : rest };
      return ssl;
    },
  };
}

function mockCert(subject: string, issuer: string, san: string[]) {
  return {
    subject,
    issuer,
    serial: "04:A1:7C:22:9E:10:5B:33",
    not_before: "2026-08-01T00:00:00Z",
    not_after: "2026-10-30T23:59:59Z",
    san,
    sha256: "AB12".repeat(16),
  };
}

function mockBody(kind: ContentKind): string {
  return kind === "json" ? JSON.stringify({ mock: true, items: [1, 2, 3], nested: { ok: true } }) : "mock body\n";
}
