import type {
  BodyPart,
  CaInfo,
  ContentGroup,
  ContentKind,
  FilterSpec_Deserialize,
  FlowDetail_Serialize,
  FlowRow_Serialize,
  FlowSummary_Serialize,
  ProxyStatus_Serialize,
  SidebarSnapshot_Deserialize,
  SseEvent_Serialize,
  SslSettings,
  SystemProxyStatus_Serialize,
  TlsInfo_Serialize,
  TrustStatus_Serialize,
  ViewDelta,
  WsMessage_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

/** A fully specified mock flow with bodies (the dev zoo, see `dev/zoo.ts`). */
export interface MockFlow {
  row: FlowRow_Serialize;
  detail: FlowDetail_Serialize;
  bodies: Record<BodyPart, { raw: Uint8Array; decoded: Uint8Array }>;
}

/**
 * Browser-only stand-in for the engine, so the UI runs under plain Vite and in
 * tests. Static data, no live traffic, no sorting. The real logic is in Rust.
 * With `fixtures`, those flows replace the `count` synthetic ones.
 */
export function createMockBackend(count: number, fixtures?: Promise<MockFlow[]>): Backend {
  let recording = true;
  const status = (): ProxyStatus_Serialize => ({ listening: "127.0.0.1:9090", error: null, recording });
  const hosts = ["api.github.com", "www.google.com", "api.stripe.com", "slack.com", "localhost", "i.ytimg.com"];
  const kinds: ContentKind[] = ["json", "html", "javascript", "image", "css", "xml", "graphql", "none"];
  const full = new Map<number, MockFlow>();
  const urls = new Map<string, string>();
  const methods = ["GET", "GET", "GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
  const statuses = [200, 200, 200, 201, 204, 304, 404, 500];
  const clients = ["Safari", "Google Chrome", "curl", "Slack", "node"];
  const now = Date.now();
  let flows: FlowRow_Serialize[] = Array.from({ length: count }, (_, i) => {
    const host = hosts[i % hosts.length]!;
    const https = host !== "localhost";
    // A few HTTPS flows are tunnels or pinned clients, to exercise the SSL column.
    const tunnel = https && i % 50 === 7;
    const pinned = https && i % 97 === 11;
    const connect = tunnel || pinned;
    const row: FlowRow_Serialize = [
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
      clients[(i * 3) % clients.length]!,
      now - (count - i) * 20,
      (i * 37) % 900,
      0,
      (i * 131) % 90_000,
      false,
      https && !connect && i % 3 !== 0 ? "HTTP/2" : "HTTP/1.1",
      tunnel ? "passthrough" : pinned ? "likely_pinned" : https ? "intercepted" : "none",
      !connect && kinds[(i * 5) % kinds.length] === "graphql" ? "GetViewer" : null,
    ];
    // A few are WebSockets and event streams, for the message panels.
    if (!connect && i % 61 === 13) return asStream(row, "websocket");
    if (!connect && i % 67 === 17) return asStream(row, "sse");
    return row;
  });
  const ready = (fixtures ?? Promise.resolve(null)).then((zoo) => {
    if (!zoo) return;
    flows = zoo.map((f) => f.row);
    for (const f of zoo) full.set(f.row[0], f);
  });
  let ssl: SslSettings = {
    include: ["*"],
    exclude: ["*.apple.com", "ocsp*"],
    bypass: [],
    insecure_upstream: [],
    exclude_apps: [],
  };
  let trust: TrustStatus_Serialize = {
    keychain: "not_trusted",
    verified: false,
    detail: "the system doesn't trust the Proxymate CA yet (mock)",
  };
  let sysproxy: SystemProxyStatus_Serialize = { enabled: false, services: [], supported: true, error: null };
  let onSidebar: ((s: SidebarSnapshot_Deserialize) => void) | null = null;
  const ca: CaInfo = {
    common_name: "Proxymate CA (mock, 2026-10-21)",
    cert_path: "/Users/you/Library/Application Support/Proxymate/ca/proxymate-ca.pem",
    not_before: "2026-10-20T12:00:00Z",
    not_after: "2036-10-18T12:00:00Z",
    sha256: "3F9A".repeat(16),
  };

  const views = new Map<number, number[]>();
  let nextView = 0;

  const GROUPS: Record<ContentKind, ContentGroup> = {
    none: "other", json: "json", xml: "xml", html: "document", formurlencoded: "form", multipart: "form",
    graphql: "graphql", javascript: "js", css: "css", text: "document", image: "media", svg: "media",
    video: "media", audio: "media", font: "media", pdf: "document", binary: "other",
  };
  const groupOf = (k: ContentKind): ContentGroup => GROUPS[k];

  function refilter(filter: FilterSpec_Deserialize): number[] {
    const text = filter.text.trim().toLowerCase();
    const out: number[] = [];
    for (const f of flows) {
      if (filter.protocol === "http" && f[2] !== "http") continue;
      if (filter.protocol === "https" && f[2] !== "https") continue;
      if (filter.protocol === "websocket" && f[2] !== "ws" && f[2] !== "wss") continue;
      if (filter.content.length > 0 && !filter.content.includes(groupOf(f[9]))) continue;
      if (filter.host && !hostMatches(f[4], filter.host)) continue;
      if (filter.client && f[10] !== filter.client) continue;
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
      await ready;
      const start = performance.now();
      const view = ++nextView;
      views.set(view, refilter(filter));
      return info(view, start);
    },
    async setFilter(view, filter) {
      await ready;
      const start = performance.now();
      views.set(view, refilter(filter));
      return info(view, start);
    },
    async setSort(view) {
      await ready;
      return info(view, performance.now());
    },
    async dropView(view) {
      await ready;
      views.delete(view);
    },
    async getRows(view, start, n) {
      await ready;
      const ids = views.get(view) ?? [];
      return { view, start, len: ids.length, rows: ids.slice(start, start + n).map((id) => flows[id]!) };
    },
    async getFlow(id): Promise<FlowSummary_Serialize | null> {
      await ready;
      const f = flows[id];
      if (!f) return null;
      return {
        id: f[0], state: f[1], scheme: f[2], method: f[3], host: f[4], port: f[5], path: f[6], query: f[7],
        status: f[8], content_kind: f[9], client: f[10], started_at_ms: f[11], duration_ms: f[12],
        req_size: f[13], resp_size: f[14], edited: f[15], http_version: f[16], tls: f[17], graphql_op: f[18],
      };
    },
    async subscribe(view, onDelta: (d: ViewDelta) => void) {
      const len = views.get(view)?.length ?? 0;
      setTimeout(() => onDelta({ view, seq: 1, total: flows.length, len, appended: 0, changed: [], structural: true }), 0);
    },
    async getFlowDetail(id): Promise<FlowDetail_Serialize | null> {
      await ready;
      const fixture = full.get(id);
      if (fixture) return fixture.detail;
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
            { name: "content-type", value: f[6] === SSE_PATH ? "text/event-stream" : f[9] === "json" ? "application/json" : "text/plain" },
            { name: "content-length", value: String(body.length) },
          ],
        },
        request_body: { size: 0, captured: 0, truncated: false, encoding: null, complete: true, kind: "none", mime: null },
        response_body: {
          size: body.length,
          captured: body.length,
          truncated: false,
          encoding: null,
          complete: true,
          kind: f[9] === "json" ? "json" : "text",
          mime: f[9] === "json" ? "application/json" : "text/plain",
        },
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
    async getBody(id, part, decoded, max) {
      await ready;
      const fixture = full.get(id);
      const f = flows[id];
      let bytes = fixture
        ? fixture.bodies[part][decoded ? "decoded" : "raw"]
        : f && part === "response"
          ? new TextEncoder().encode(mockBody(f[9]))
          : new Uint8Array();
      if (max != null) bytes = bytes.subarray(0, max);
      return bytes.slice().buffer as ArrayBuffer;
    },
    async bodyUrl(id, part, decoded) {
      await ready;
      const key = `${id}/${part}/${decoded}`;
      const cached = urls.get(key);
      if (cached) return cached;
      const fixture = full.get(id);
      const info = fixture?.detail[part === "request" ? "request_body" : "response_body"];
      const bytes = new Uint8Array(await this.getBody(id, part, decoded));
      const url = URL.createObjectURL(new Blob([bytes], { type: info?.mime ?? "application/octet-stream" }));
      urls.set(key, url);
      return url;
    },
    async saveBody(id, part, decoded, fileName) {
      await ready;
      const a = document.createElement("a");
      a.href = await this.bodyUrl(id, part, decoded);
      a.download = fileName;
      a.click();
      return fileName;
    },
    async getWebsocketMessages(id, query, start, n) {
      const all = flows[id]?.[2] === "wss" || flows[id]?.[2] === "ws" ? mockMessages(flows[id]![11] ?? 0) : [];
      const text = query.text.trim().toLowerCase();
      const matching = all.filter(
        (m) => (query.direction === null || m.direction === query.direction) && (!text || m.preview.toLowerCase().includes(text)),
      );
      const close = all.length ? { code: 1000, reason: "bye", by: "sent" as const } : null;
      return { total: all.length, len: matching.length, start, messages: matching.slice(start, start + n), open: false, close };
    },
    async getWebsocketPayload(id, index) {
      const m = mockMessages(flows[id]?.[11] ?? 0)[index];
      const bytes = m?.opcode === "binary" ? Uint8Array.from({ length: 48 }, (_, i) => (i * 37) % 256) : new TextEncoder().encode(m?.preview ?? "");
      return bytes.buffer as ArrayBuffer;
    },
    async getSseEvents(id, search, start, n) {
      const all = flows[id]?.[6] === SSE_PATH ? mockEvents(flows[id]![11] ?? 0) : [];
      const text = search.trim().toLowerCase();
      const matching = all.filter((e) => !text || e.data.toLowerCase().includes(text) || (e.event ?? "").includes(text));
      return { total: all.length, len: matching.length, start, events: matching.slice(start, start + n), open: false };
    },
    async clearFlows() {
      await ready;
      flows = [];
      for (const view of views.keys()) views.set(view, []);
      onSidebar?.(sidebar(flows));
    },
    async setRecording(on) {
      await ready;
      recording = on;
      return status();
    },
    async getProxyStatus() {
      await ready;
      return status();
    },
    async getCaInfo() {
      await ready;
      return ca;
    },
    async regenerateCa() {
      await ready;
      return ca;
    },
    async getSslSettings() {
      await ready;
      return ssl;
    },
    async setSslSettings(settings) {
      await ready;
      ssl = settings;
      return ssl;
    },
    async addSslBypass(host) {
      await ready;
      if (!ssl.bypass.includes(host)) ssl = { ...ssl, bypass: [...ssl.bypass, host] };
      return ssl;
    },
    async setInsecureUpstream(host, allow) {
      await ready;
      const rest = ssl.insecure_upstream.filter((h) => h !== host);
      ssl = { ...ssl, insecure_upstream: allow ? [...rest, host] : rest };
      return ssl;
    },
    async subscribeSidebar(cb) {
      onSidebar = cb;
      setTimeout(() => cb(sidebar(flows)), 0);
    },
    async setAppSslExcluded(app, excluded) {
      const rest = (ssl.exclude_apps ?? []).filter((a) => a.toLowerCase() !== app.toLowerCase());
      ssl = { ...ssl, exclude_apps: excluded ? [...rest, app] : rest };
      return ssl;
    },
    async getTrustStatus() {
      return trust;
    },
    async installCaTrust() {
      trust = { keychain: "trusted", verified: true, detail: null };
      return trust;
    },
    async uninstallCaTrust() {
      trust = { keychain: "not_trusted", verified: false, detail: "the system doesn't trust the Proxymate CA yet (mock)" };
      return trust;
    },
    async getSystemProxyStatus() {
      return sysproxy;
    },
    async setSystemProxy(on) {
      sysproxy = { ...sysproxy, enabled: on, services: on ? ["Wi-Fi"] : [], error: null };
      return sysproxy;
    },
    async getTerminalEnv() {
      const proxy = "http://127.0.0.1:9090";
      return [
        ...["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"].map((v) => `export ${v}=${proxy}`),
        ...["NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"].map((v) => `export ${v}='${ca.cert_path}'`),
        "",
      ].join("\n");
    },
  };
}

/** `*.example.com` matches `example.com` and its subdomains, as in Rust. */
function hostMatches(host: string, pattern: string): boolean {
  if (!pattern.startsWith("*.")) return host === pattern;
  const domain = pattern.slice(2);
  return host === domain || host.endsWith(`.${domain}`);
}

/** The engine aggregates this in Rust; the mock does it the simple way. */
function sidebar(flows: FlowRow_Serialize[]): SidebarSnapshot_Deserialize {
  const apps = new Map<string, number>();
  const hosts = new Map<string, number>();
  for (const f of flows) {
    if (f[10]) apps.set(f[10], (apps.get(f[10]) ?? 0) + 1);
    hosts.set(f[4], (hosts.get(f[4]) ?? 0) + 1);
  }
  const domains = new Map<string, { host: string; count: number }[]>();
  for (const [host, count] of hosts) {
    const labels = host.split(".");
    const domain = labels.length > 2 ? labels.slice(-2).join(".") : host;
    domains.set(domain, [...(domains.get(domain) ?? []), { host, count }]);
  }
  return {
    total: flows.length,
    apps: [...apps]
      .sort(([a], [b]) => a.toLowerCase().localeCompare(b.toLowerCase()))
      .map(([name, count]) => ({ name, bundle_id: null, has_icon: false, count })),
    domains: [...domains]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([domain, list]) => ({
        domain,
        count: list.reduce((n, h) => n + h.count, 0),
        hosts: list.length === 1 && list[0]!.host === domain ? [] : list.sort((a, b) => a.host.localeCompare(b.host)),
      })),
  };
}

const SSE_PATH = "/v1/chat/stream";

/** `row` turned into a WebSocket or SSE flow; host, client and time stay. */
function asStream(row: FlowRow_Serialize, kind: "websocket" | "sse"): FlowRow_Serialize {
  const ws = kind === "websocket";
  const tls = row[2] === "https";
  const out: FlowRow_Serialize = [...row];
  out[1] = "completed";
  out[2] = ws ? (tls ? "wss" : "ws") : row[2];
  out[3] = ws ? "GET" : "POST";
  out[6] = ws ? "/socket" : SSE_PATH;
  out[7] = "";
  out[8] = ws ? 101 : 200;
  out[9] = ws ? "none" : "text";
  out[12] = 4_200;
  out[13] = 1_480;
  out[14] = 9_310;
  out[16] = "HTTP/1.1";
  out[18] = null;
  return out;
}

function mockMessages(started: number): WsMessage_Serialize[] {
  return Array.from({ length: 40 }, (_, i) => {
    const sent = i % 2 === 0;
    const last = i === 39;
    const binary = i % 10 === 5;
    const preview = last ? "bye" : binary ? "" : JSON.stringify(sent ? { op: "subscribe", channel: `ticker.${i}` } : { channel: `ticker.${i - 1}`, price: 100 + i, ok: true });
    return {
      index: i,
      direction: sent || last ? "sent" : "received",
      opcode: last ? "close" : binary ? "binary" : "text",
      len: binary ? 48 : preview.length,
      time_ms: started + i * 90,
      truncated: false,
      close_code: last ? 1000 : null,
      preview,
    };
  });
}

function mockEvents(started: number): SseEvent_Serialize[] {
  const words = ["Hello", " there", "! How", " can", " I", " help", " today", "?"];
  return words.map((w, i) => ({
    index: i,
    time_ms: started + i * 120,
    id: String(i),
    event: i === words.length - 1 ? "done" : "delta",
    data: JSON.stringify({ delta: w, index: i }),
    retry: null,
    truncated: false,
  }));
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
