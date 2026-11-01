import type {
  BodyPart,
  BreakpointInfo_Serialize,
  CaInfo,
  FlowRules_Serialize,
  ContentGroup,
  ContentKind,
  DocumentInfo,
  FilterSpec_Deserialize,
  FlowDetail_Serialize,
  FlowSelection,
  FlowSelection_Deserialize,
  FlowRow_Serialize,
  HeaderColumn,
  SortSpec_Deserialize,
  FlowSummary_Serialize,
  ProxyStatus_Serialize,
  SaveProgress_Serialize,
  SidebarSnapshot_Deserialize,
  SseEvent_Serialize,
  SslSettings,
  SystemProxyStatus_Serialize,
  TlsInfo_Serialize,
  ProxySettings_Serialize,
  StorageSettings_Serialize,
  StorageStatus_Serialize,
  TrustStatus_Serialize,
  UpdateSettings,
  ViewDelta_Serialize,
  WsMessage_Serialize,
  DecodedBody_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";
import { GLOBAL_THROTTLE_RULE_ID, emptyMatcher, normalizeRule, type Rule } from "@/lib/rules";
import { newId, parseDoc, toJson, type JNode } from "@/lib/json/doc";
import { createMockDevices } from "@/lib/mockDevices";
import { createMockMcp } from "@/lib/mockMcp";
import { createMockNetwork } from "@/lib/mockNetwork";
import { createMockMocking } from "@/lib/mockMocking";
import { createMockProtos } from "@/lib/mockProtos";
import { createMockScripts } from "@/lib/mockScripts";

/** A fully specified mock flow with bodies (the dev zoo, see `dev/zoo.ts`). */
export interface MockFlow {
  row: FlowRow_Serialize;
  detail: FlowDetail_Serialize;
  bodies: Record<BodyPart, { raw: Uint8Array; decoded: Uint8Array }>;
  /** What `decodeBody` answers (M16): the engine's output for the zoo fixture. */
  decodedBodies?: Partial<Record<BodyPart, DecodedBody_Serialize>>;
}

/**
 * Browser-only stand-in for the engine, so the UI runs under plain Vite and in
 * tests. Static data, no live traffic, no sorting. The real logic is in Rust.
 * With `fixtures`, those flows replace the `count` synthetic ones.
 */
export function createMockBackend(count: number, fixtures?: Promise<MockFlow[]>): Backend {
  let recording = true;
  const network = createMockNetwork();
  const status = (): ProxyStatus_Serialize => ({
    listening: "127.0.0.1:9090",
    error: null,
    recording,
    listeners: network.listeners(),
  });
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
      !connect && i % 41 === 5,
      https && !connect && i % 3 !== 0 ? "HTTP/2" : "HTTP/1.1",
      tunnel ? "passthrough" : pinned ? "likely_pinned" : https ? "intercepted" : "none",
      !connect && kinds[(i * 5) % kinds.length] === "graphql" ? "GetViewer" : null,
      i % 211 === 3 ? "user" : null,
      false,
      null,
      null,
      null,
      [],
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
  let sidebarStore = 0;
  const deltaSinks = new Map<number, (d: ViewDelta_Serialize) => void>();
  let storageSettings: StorageSettings_Serialize = { max_flows: 250_000, max_disk_mb: 4096, max_ws_messages: 50_000 };
  let proxySettings: ProxySettings_Serialize = { port: 9090, pool_idle_secs: 90, pool_max_idle_per_host: 32, connect_timeout_secs: 10 };
  let onStorage: ((s: StorageStatus_Serialize) => void) | null = null;
  const storageStatus = (): StorageStatus_Serialize => ({
    flows: flows.length,
    pinned: flows.filter((f) => f[20]).length,
    disk_bytes: flows.length * 2_300,
    evicted: 0,
    // `?diskFull`: the footer's disk-full warning.
    error: typeof window !== "undefined" && new URLSearchParams(window.location.search).has("diskFull")
      ? "the disk is full: free some space, or lower the disk limit in Settings › Storage"
      : null,
    dir: "/Users/you/Library/Application Support/Proxymate/live/mock",
    sent_bytes: 0,
    received_bytes: 0,
  });
  const ca: CaInfo = {
    common_name: "Proxymate CA (mock, 2026-10-21)",
    cert_path: "/Users/you/Library/Application Support/Proxymate/ca/proxymate-ca.pem",
    not_before: "2026-10-20T12:00:00Z",
    not_after: "2036-10-18T12:00:00Z",
    sha256: "3F9A".repeat(16),
  };

  let lastRepeat = 0;
  const cancelledRepeats = new Set<number>();
  const views = new Map<number, number[]>();
  const viewFilters = new Map<number, FilterSpec_Deserialize>();
  let nextView = 0;
  // Opened documents show the first flows of the mock data, under the same ids.
  let documents: DocumentInfo[] = [];
  let lastDocument = 0;
  let updateSettings: UpdateSettings = { check_at_launch: true };
  const DOCUMENT_FLOWS = 50;
  // Deleted flows stay in `flows` (ids index it) but leave every view.
  const removed = new Set<number>();
  const storeFlows = (store: number) => (store === 0 ? flows : flows.slice(0, DOCUMENT_FLOWS)).filter((f) => !removed.has(f[0]));
  let headerColumns: HeaderColumn[] = [];
  // The headers the synthetic flows' details have (fixtures carry their own).
  const headerLists = (f: FlowRow_Serialize): Record<BodyPart, { name: string; value: string }[]> => {
    const fixture = full.get(f[0]);
    if (fixture) return { request: fixture.detail.request.headers, response: fixture.detail.response?.headers ?? [] };
    return {
      request: [
        { name: "host", value: f[4] },
        { name: "user-agent", value: "curl/8.7.1" },
        { name: "accept", value: "*/*" },
      ],
      response: [{ name: "content-type", value: f[9] === "json" ? "application/json" : "text/plain" }],
    };
  };
  const withExtra = (f: FlowRow_Serialize): FlowRow_Serialize => {
    if (headerColumns.length === 0) return f;
    const lists = headerLists(f);
    const extra = headerColumns.map((c) => lists[c.part].find((h) => h.name.toLowerCase() === c.name)?.value ?? null);
    return [...f.slice(0, 24), extra] as unknown as FlowRow_Serialize;
  };
  // A view's sort, for header columns (the mock sorts by nothing else).
  const viewSorts = new Map<number, SortSpec_Deserialize>();
  const sortIds = (ids: number[], sort: SortSpec_Deserialize | undefined): number[] => {
    if (!sort || sort.key !== "extra") return ids;
    const at = (id: number) => withExtra(flows[id]!)[24][sort.extra ?? 0] ?? "";
    const out = [...ids].sort((a, b) => at(a).localeCompare(at(b)) || a - b);
    return sort.dir === "desc" ? out.reverse() : out;
  };
  const changed = (ids: number[], structural = false) => {
    for (const [view, sink] of deltaSinks) {
      sink({ view, seq: 0, total: flows.length, len: views.get(view)?.length ?? 0, appended: 0, changed: ids, structural, deep: null });
    }
  };
  const idsOf = async (store: number, selection: FlowSelection_Deserialize): Promise<number[]> => {
    await ready;
    if (selection.kind === "ids") return selection.ids.filter((id) => flows[id] && !removed.has(id));
    if (selection.kind === "all") return storeFlows(store).map((f) => f[0]);
    const except = new Set(selection.except ?? []);
    return (views.get(selection.view) ?? []).filter((id) => !except.has(id));
  };

  let rules: Rule[] = [
    {
      id: GLOBAL_THROTTLE_RULE_ID,
      name: "Network conditions",
      enabled: false,
      order: -1_000_000,
      origin: { kind: "preset", preset: "throttle" },
      matcher: emptyMatcher(),
      actions: [{ type: "Throttle", download_kbps: 780, upload_kbps: 330, latency_ms: 100 }],
    },
    {
      id: "mock-agent-rule",
      name: "Mock checkout (agent)",
      enabled: true,
      order: 1,
      origin: { kind: "agent", client: "claude-code", session: null, created_at_ms: now },
      matcher: { ...emptyMatcher(), host: { kind: "exact", value: "api.stripe.com", case_insensitive: false } },
      actions: [{ type: "SetStatus", status: 503 }],
    },
  ];
  let ruleSettings = { breakpoint_timeout_secs: 300 };
  // Dev only: `?breakpoint` seeds a paused request, `?breakpoint=ws` a held
  // WebSocket message, for the Breakpoint Editor.
  const seed = typeof window !== "undefined" ? new URLSearchParams(window.location.search).get("breakpoint") : null;
  let breakpoints: BreakpointInfo_Serialize[] = seed === null ? [] : [seed === "ws" ? mockMessageBreakpoint(now) : mockBreakpoint(now)];
  // Messages sent from the composer, per flow; the mock connections stay open.
  const sentMessages = new Map<number, { meta: WsMessage_Serialize; bytes: Uint8Array }[]>();
  const closedSockets = new Set<number>();
  const wsLog = (id: number) => {
    const recorded = mockMessages(flows[id]?.[11] ?? 0).map((meta) => ({ meta, bytes: mockPayload(meta) }));
    return [...recorded, ...(sentMessages.get(id) ?? [])];
  };
  let onBreakpoints: ((q: BreakpointInfo_Serialize[]) => void) | null = null;
  const pushBreakpoints = () => onBreakpoints?.(breakpoints);
  const sorted = () => [...rules].sort((a, b) => a.order - b.order);
  const findRule = (id: string) => {
    const rule = rules.find((r) => r.id === id);
    if (!rule) throw new Error(`no rule with id ${id}`);
    return rule;
  };

  const GROUPS: Record<ContentKind, ContentGroup> = {
    none: "other", json: "json", xml: "xml", html: "document", formurlencoded: "form", multipart: "form",
    graphql: "graphql", javascript: "js", css: "css", text: "document", image: "media", svg: "media",
    video: "media", audio: "media", font: "media", pdf: "document", binary: "other",
    grpc: "other", protobuf: "other", messagepack: "other", cbor: "other",
  };
  const groupOf = (k: ContentKind): ContentGroup => GROUPS[k];

  function refilter(filter: FilterSpec_Deserialize, store = 0): number[] {
    const text = filter.text.trim().toLowerCase();
    const out: number[] = [];
    for (const f of storeFlows(store)) {
      if (filter.protocol === "http" && f[2] !== "http") continue;
      if (filter.protocol === "https" && f[2] !== "https") continue;
      if (filter.protocol === "websocket" && f[2] !== "ws" && f[2] !== "wss") continue;
      if (filter.content.length > 0 && !filter.content.includes(groupOf(f[9]))) continue;
      if (filter.host && !hostMatches(f[4], filter.host)) continue;
      if (filter.client && f[10] !== filter.client) continue;
      // The DSL lives in Rust; the mock knows `is:pinned` and plain text.
      const words = text.split(/\s+/).filter(Boolean);
      if (words.includes("is:pinned") && !f[20]) continue;
      if ((words.includes("is:labelled") || words.includes("label:any")) && !f[22]) continue;
      const label = words.find((w) => w.startsWith("label:") && w !== "label:any");
      if (label && !label.slice(6).split(",").includes(f[22] ?? "")) continue;
      if (words.includes("is:commented") && !f[23]) continue;
      const comment = words.find((w) => w.startsWith("comment:"));
      if (comment && !(f[23] ?? "").toLowerCase().includes(comment.slice(8).replace(/"/g, ""))) continue;
      const plain = words.filter((w) => !w.includes(":")).join(" ");
      if (plain && !`${f[4]}${f[6]}${f[7]}`.toLowerCase().includes(plain)) continue;
      out.push(f[0]);
    }
    return out;
  }

  function info(view: number, start: number) {
    return {
      view,
      total: flows.length,
      len: views.get(view)!.length,
      elapsed_us: Math.round((performance.now() - start) * 1000),
      deep: null,
    };
  }

  const api: Backend = {
    async createView(store, filter: FilterSpec_Deserialize, sort) {
      await ready;
      const start = performance.now();
      // Like the engine: a document's view ids carry its store.
      const view = store * 2 ** 24 + ++nextView;
      viewSorts.set(view, sort);
      viewFilters.set(view, filter);
      views.set(view, sortIds(refilter(filter, store), sort));
      return info(view, start);
    },
    async setFilter(view, filter) {
      await ready;
      const start = performance.now();
      viewFilters.set(view, filter);
      views.set(view, sortIds(refilter(filter, Math.floor(view / 2 ** 24)), viewSorts.get(view)));
      return info(view, start);
    },
    async setSort(view, sort) {
      await ready;
      viewSorts.set(view, sort);
      const filter = viewFilters.get(view);
      if (filter) views.set(view, sortIds(refilter(filter, Math.floor(view / 2 ** 24)), sort));
      return info(view, performance.now());
    },
    async dropView(view) {
      await ready;
      views.delete(view);
    },
    async getRows(view, start, n) {
      await ready;
      const ids = views.get(view) ?? [];
      return { view, start, len: ids.length, rows: ids.slice(start, start + n).map((id) => withExtra(flows[id]!)) };
    },
    async getFlow(_store, id): Promise<FlowSummary_Serialize | null> {
      await ready;
      const f = flows[id];
      if (!f || removed.has(id)) return null;
      return {
        id: f[0], state: f[1], scheme: f[2], method: f[3], host: f[4], port: f[5], path: f[6], query: f[7],
        status: f[8], content_kind: f[9], client: f[10], started_at_ms: f[11], duration_ms: f[12],
        req_size: f[13], resp_size: f[14], edited: f[15], http_version: f[16], tls: f[17], graphql_op: f[18],
        sent_by: f[19],
        pinned: f[20],
        grpc_status: f[21],
        label: f[22],
        comment: f[23],
        extra: withExtra(f)[24],
      };
    },
    async subscribe(view, onDelta: (d: ViewDelta_Serialize) => void) {
      const len = views.get(view)?.length ?? 0;
      deltaSinks.set(view, onDelta);
      setTimeout(
        () => onDelta({ view, seq: 1, total: flows.length, len, appended: 0, changed: [], structural: true, deep: null }),
        0,
      );
    },
    async getFlowDetail(_store, id): Promise<FlowDetail_Serialize | null> {
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
        rules: mockFlowRules(f[15], id),
        trailers: null,
        // Every fourth flow went through the corporate proxy (Settings › Network).
        via:
          id % 4 === 1
            ? {
                proxy: "http://proxy.corp.example:3128",
                addr: "10.20.0.8:3128",
                dns_override: null,
                client_cert: https ? "CN=pm-client, O=Example Corp" : null,
                reverse_port: null,
              }
            : null,
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
    async getBody(_store, id, part, decoded, max) {
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
    async bodyUrl(store, id, part, decoded) {
      await ready;
      const key = `${id}/${part}/${decoded}`;
      const cached = urls.get(key);
      if (cached) return cached;
      const fixture = full.get(id);
      const info = fixture?.detail[part === "request" ? "request_body" : "response_body"];
      const bytes = new Uint8Array(await this.getBody(store, id, part, decoded));
      const url = URL.createObjectURL(new Blob([bytes], { type: info?.mime ?? "application/octet-stream" }));
      urls.set(key, url);
      return url;
    },
    async saveBody(store, id, part, decoded, fileName) {
      await ready;
      const a = document.createElement("a");
      a.href = await this.bodyUrl(store, id, part, decoded);
      a.download = fileName;
      a.click();
      return fileName;
    },
    async getWebsocketMessages(_store, id, query, start, n) {
      const all = flows[id]?.[2] === "wss" || flows[id]?.[2] === "ws" ? wsLog(id).map((m) => m.meta) : [];
      const text = query.text.trim().toLowerCase();
      const matching = all.filter(
        (m) => (query.direction === null || m.direction === query.direction) && (!text || m.preview.toLowerCase().includes(text)),
      );
      // Every other mock socket is still open, so the composer shows.
      const open = id % 2 === 1 && !closedSockets.has(id);
      const close = all.length && !open ? { code: 1000, reason: "bye", by: "sent" as const } : null;
      return {
        total: all.length,
        len: matching.length,
        start,
        messages: matching.slice(start, start + n),
        open,
        close,
        dropped: 0,
      };
    },
    async getWebsocketPayload(_store, id, index) {
      const m = wsLog(id)[index];
      if (!m) throw new Error(`flow ${id} has no message ${index}`);
      return m.bytes.slice().buffer as ArrayBuffer;
    },
    async getWebsocketOriginal(_store, id, index) {
      const m = wsLog(id)[index];
      if (!m?.meta.edited) throw new Error(`message ${index + 1} wasn't edited`);
      return new TextEncoder().encode(m.meta.preview.replace(/"price":\d+/, '"price":4.2')).buffer as ArrayBuffer;
    },
    async wsSend(id, message) {
      if (id % 2 === 0 || closedSockets.has(id)) throw new Error(`flow ${id}'s WebSocket isn't open`);
      const bytes = message.binary ? Uint8Array.from(atob(message.payload), (c) => c.charCodeAt(0)) : new TextEncoder().encode(message.payload);
      const log = sentMessages.get(id) ?? [];
      const index = wsLog(id).length;
      log.push({
        meta: {
          index,
          direction: message.direction,
          opcode: message.binary ? "binary" : "text",
          len: bytes.length,
          time_ms: Date.now(),
          truncated: false,
          close_code: null,
          preview: message.binary ? "" : message.payload.slice(0, 256),
          edited: false,
          injected: true,
          dropped: false,
        },
        bytes,
      });
      sentMessages.set(id, log);
    },
    async wsClose(id) {
      closedSockets.add(id);
    },
    async getSseEvents(_store, id, search, start, n) {
      const all = flows[id]?.[6] === SSE_PATH ? mockEvents(flows[id]![11] ?? 0) : [];
      const text = search.trim().toLowerCase();
      const matching = all.filter((e) => !text || e.data.toLowerCase().includes(text) || (e.event ?? "").includes(text));
      return { total: all.length, len: matching.length, start, events: matching.slice(start, start + n), open: false };
    },
    async clearFlows() {
      await ready;
      flows = [];
      for (const view of views.keys()) views.set(view, []);
      onSidebar?.(sidebar(flows, 0));
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
    async getStorageSettings() {
      return storageSettings;
    },
    async setStorageSettings(settings) {
      if (settings.max_flows !== 0 && settings.max_flows < 100) throw new Error("keep at least 100 flows (or 0 for no limit)");
      storageSettings = settings;
      return settings;
    },
    async subscribeStorage(cb) {
      onStorage = cb;
      setTimeout(() => cb(storageStatus()), 0);
    },
    async pinFlows(store, selection, pinned) {
      const ids = await idsOf(store, selection);
      for (const id of ids) flows[id]![20] = pinned;
      changed(ids);
      onStorage?.(storageStatus());
      return ids.length;
    },
    selectedIds: idsOf,
    async getFlowUrls(store, selection) {
      return (await idsOf(store, selection)).map((id) => {
        const f = flows[id]!;
        return `${f[2]}://${f[4]}${f[6]}${f[7] ? `?${f[7]}` : ""}`;
      });
    },
    async removeFlows(store, selection) {
      const ids = await idsOf(store, selection);
      for (const id of ids) removed.add(id);
      const gone = new Set(ids);
      for (const [view, list] of views) views.set(view, list.filter((id) => !gone.has(id)));
      changed([], true);
      onSidebar?.(sidebar(storeFlows(sidebarStore), sidebarStore));
      onStorage?.(storageStatus());
      return ids.length;
    },
    async labelFlows(store, selection, label) {
      const ids = await idsOf(store, selection);
      for (const id of ids) flows[id]![22] = label;
      changed(ids);
      return ids.length;
    },
    async commentFlow(_store, id, comment) {
      await ready;
      const f = flows[id];
      if (!f) throw new Error(`unknown flow ${id}`);
      f[23] = comment.trim() || null;
      changed([id]);
    },
    async setHeaderColumns(columns) {
      await ready;
      headerColumns = columns.map((c) => ({ part: c.part, name: c.name.trim().toLowerCase() }));
      changed([], true);
      return headerColumns;
    },
    async getCode(store, id, language) {
      const spec = await this.getRequestSpec(store, id);
      if (language === "curl") return this.getCurl(store, id);
      const headers = spec.headers.map((h) => `    ${JSON.stringify(h.name)}: ${JSON.stringify(h.value)},`).join("\n");
      return `// ${language} (mock)\nfetch(${JSON.stringify(spec.url)}, {\n  method: ${JSON.stringify(spec.method)},\n  headers: {\n${headers}\n  },\n});`;
    },
    async startRepeat(store, id, opts, onProgress) {
      await ready;
      const repeat = ++lastRepeat;
      const total = opts.iterations;
      const f = flows[id];
      const status = f?.[8] ?? null;
      let done = 0;
      const tick = () => {
        done = Math.min(total, done + Math.max(1, Math.ceil(total / 10)));
        const finished = done >= total || cancelledRepeats.has(repeat);
        onProgress({
          id: repeat,
          done,
          total,
          failed: status === null ? done : 0,
          statuses: [{ status, count: done }],
          min_ms: 12,
          p50_ms: 40,
          p95_ms: 95,
          max_ms: 180,
          first_flow: null,
          last_flow: null,
          finished,
          cancelled: finished && done < total,
          error: null,
        });
        if (!finished) setTimeout(tick, 60);
      };
      setTimeout(tick, 30);
      void store;
      return repeat;
    },
    async cancelRepeat(id) {
      cancelledRepeats.add(id);
      return true;
    },
    async getProxySettings() {
      return proxySettings;
    },
    getNetworkSettings: network.getNetworkSettings,
    setNetworkSettings: network.setNetworkSettings,
    async setProxySettings(settings) {
      if (settings.port < 1024) throw new Error("pick a port from 1024 up: lower ones need administrator rights");
      if (settings.port === 8080) throw new Error("port 8080 is in use (Charles, pid 4242). Quit the app using it, or pick another port.");
      proxySettings = settings;
      return settings;
    },
    async setSidebarStore(store) {
      sidebarStore = store;
      onSidebar?.(sidebar(storeFlows(store), store));
    },
    async setWindowTheme() {},
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
      setTimeout(() => cb(sidebar(storeFlows(sidebarStore), sidebarStore)), 0);
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
    async listRules() {
      return sorted();
    },
    async createRule(rule) {
      const created = normalizeRule({
        ...(rule as Rule),
        id: crypto.randomUUID(),
        order: Math.max(0, ...rules.map((r) => r.order)) + 1,
      });
      rules = [...rules, created];
      return created;
    },
    async updateRule(rule) {
      findRule(rule.id);
      const updated = normalizeRule(rule as Rule);
      rules = rules.map((r) => (r.id === rule.id ? updated : r));
      return updated;
    },
    async setRuleEnabled(id, enabled) {
      const updated = { ...findRule(id), enabled };
      rules = rules.map((r) => (r.id === id ? updated : r));
      return updated;
    },
    async deleteRule(id) {
      if (id.startsWith("builtin-")) throw new Error(`the built-in rule ${id} can't be deleted`);
      findRule(id);
      rules = rules.filter((r) => r.id !== id);
    },
    async deleteAgentRules() {
      const before = rules.length;
      rules = rules.filter((r) => r.origin.kind !== "agent");
      return before - rules.length;
    },
    async reorderRules(ids) {
      const user = sorted().filter((r) => !r.id.startsWith("builtin-"));
      const ordered = [...ids.map((id) => user.find((r) => r.id === id)).filter((r): r is Rule => !!r), ...user.filter((r) => !ids.includes(r.id))];
      rules = [...rules.filter((r) => r.id.startsWith("builtin-")), ...ordered.map((r, i) => ({ ...r, order: i + 1 }))];
      return sorted();
    },
    async validateRule(rule) {
      for (const p of [rule.matcher.host, rule.matcher.path, rule.matcher.client_app]) {
        if (p?.kind === "regex") {
          try {
            new RegExp(p.value);
          } catch (e) {
            throw new Error(`rule ${JSON.stringify(rule.name)}, pattern: invalid regex: ${String(e)}`, { cause: e });
          }
        }
      }
    },
    async draftRule(_store, flow, preset) {
      await ready;
      const f = flows[flow];
      if (!f) throw new Error(`unknown flow ${flow}`);
      const target = `${f[3]} ${f[4]}${f[6]}`;
      const base: Rule = {
        id: "",
        name: "",
        enabled: true,
        order: 0,
        origin: { kind: "preset", preset },
        matcher: {
          ...emptyMatcher(),
          methods: [f[3]],
          host: { kind: "exact", value: f[4], case_insensitive: false },
          path: { kind: "exact", value: f[6], case_insensitive: false },
        },
        actions: [],
      };
      switch (preset) {
        case "map_local":
          return {
            ...base,
            name: `Map Local · ${target}`,
            actions: [{ type: "MapLocal", source: { kind: "inline", body: mockBody(f[9]) }, status: f[8] ?? 200, headers: [], content_type: f[9] === "json" ? "application/json" : "text/plain" }],
          };
        case "breakpoint":
          return { ...base, name: `Breakpoint · ${target}`, actions: [{ type: "Breakpoint", request: true, response: false }] };
        default:
          return { ...base, name: `Rewrite · ${target}` };
      }
    },
    async testRule(store, rule, flow, bodyLimit) {
      await ready;
      const f = flows[flow];
      if (!f) throw new Error(`unknown flow ${flow}`);
      const r = normalizeRule(rule as Rule);
      const mismatch = mockMismatch(r, f);
      const detail = (await this.getFlowDetail(store, flow))!;
      const text = async (part: BodyPart) => new TextDecoder().decode(await this.getBody(store, flow, part, true));
      const limit = bodyLimit ?? 256 * 1024;
      if (mismatch) {
        return {
          matched: false,
          mismatch,
          response_matched: null,
          request: detail.request,
          request_body: null,
          response: detail.response,
          response_body: null,
          notes: [],
          request_body_truncated: false,
          response_body_truncated: false,
        };
      }
      const request = mockApply(r.actions, "request", { headers: detail.request.headers, body: await text("request"), url: detail.request.url, status: 0 });
      const response = mockApply(r.actions, "response", { headers: detail.response?.headers ?? [], body: await text("response"), url: "", status: detail.response?.status ?? 200 });
      return {
        matched: true,
        mismatch: null,
        response_matched: detail.response ? true : null,
        request: { ...detail.request, url: request.url, headers: request.headers },
        request_body: request.body.slice(0, limit),
        response: detail.response && { ...detail.response, status: response.status, headers: response.headers },
        response_body: response.body.slice(0, limit),
        notes: [...request.notes, ...response.notes],
        request_body_truncated: request.body.length > limit,
        response_body_truncated: response.body.length > limit,
      };
    },
    async flowsMatchingRule(_store, rule, limit) {
      await ready;
      const r = normalizeRule(rule as Rule);
      return flows
        .filter((f) => f[3] !== "CONNECT" && !mockMismatch(r, f))
        .map((f) => f[0])
        .reverse()
        .slice(0, limit);
    },
    async getRuleSettings() {
      return ruleSettings;
    },
    async setRuleSettings(settings) {
      ruleSettings = settings;
      return settings;
    },
    async subscribeRules(cb) {
      // The mock has no agents; the editor's own saves refetch the list.
      setTimeout(() => cb(sorted()), 0);
    },
    async subscribeBreakpoints(cb) {
      onBreakpoints = cb;
      setTimeout(pushBreakpoints, 0);
    },
    async getBreakpointBody(id) {
      const bp = breakpoints.find((b) => b.id === id);
      if (!bp) throw new Error(`no breakpoint ${id}`);
      return new TextEncoder().encode(JSON.stringify({ email: "you@example.com", amount: 4200 }, null, 2)).buffer as ArrayBuffer;
    },
    async resolveBreakpoint(id) {
      if (!breakpoints.some((b) => b.id === id)) throw new Error(`breakpoint ${id} was already resolved`);
      breakpoints = breakpoints.filter((b) => b.id !== id);
      pushBreakpoints();
    },
    async getTerminalEnv() {
      const proxy = "http://127.0.0.1:9090";
      return [
        ...["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"].map((v) => `export ${v}=${proxy}`),
        ...["NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"].map((v) => `export ${v}='${ca.cert_path}'`),
        "",
      ].join("\n");
    },
    // `?devicePrompt=<ip>`: a LAN client waiting to be allowed.
    ...createMockDevices(9090, new URLSearchParams(window.location.search).get("devicePrompt")),
    async sendRequest(spec) {
      await ready;
      const url = new URL(spec.url);
      const id = flows.length;
      const https = url.protocol === "https:";
      flows = [
        ...flows,
        [
          id, "completed", https ? "https" : "http", spec.method, url.hostname, Number(url.port) || (https ? 443 : 80),
          url.pathname, url.search.slice(1), 200, "json", "Proxymate", Date.now(), 120, (spec.body ?? "").length, 512,
          false, "HTTP/1.1", https ? "intercepted" : "none", null, "user", false, null, null, null, [],
        ],
      ];
      return id;
    },
    async replayFlow(store, id) {
      const spec = await this.getRequestSpec(store, id);
      return this.sendRequest(spec, true);
    },
    async getRequestSpec(store, id) {
      const detail = await this.getFlowDetail(store, id);
      if (!detail) throw new Error(`unknown flow ${id}`);
      const skip = new Set(["host", "content-length"]);
      return {
        method: detail.request.method,
        url: detail.request.url,
        headers: detail.request.headers.filter((h) => !skip.has(h.name.toLowerCase())),
        body: null,
        body_base64: false,
      };
    },
    async getCurl(store, id) {
      const spec = await this.getRequestSpec(store, id);
      const q = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;
      return [`curl ${q(spec.url)}`, ...spec.headers.map((h) => `  -H ${q(`${h.name}: ${h.value}`)}`)].join(" \\\n");
    },
    async listDocuments() {
      return documents;
    },
    async openDocument(path, har) {
      await ready;
      const name = path?.split("/").pop() ?? (har ? "mock.har" : "mock.proxymate");
      const doc: DocumentInfo = {
        store: ++lastDocument,
        kind: name.endsWith(".har") ? "har" : "session",
        name,
        path: path ?? `/Users/you/Documents/${name}`,
        flows: Math.min(DOCUMENT_FLOWS, flows.length),
      };
      documents = [...documents, doc];
      return doc;
    },
    async closeDocument(store) {
      documents = documents.filter((d) => d.store !== store);
    },
    async saveSession(store, selection, path, fileName, onProgress) {
      return mockSave(storeFlows(store).length, selection, path ?? `/Users/you/Documents/${fileName}`, onProgress);
    },
    async exportHar(store, selection, path, fileName, onProgress) {
      return mockSave(storeFlows(store).length, selection, path ?? `/Users/you/Documents/${fileName}`, onProgress);
    },
    async onMenu() {
      return () => {};
    },
    ...createMockMcp(now),
    ...createMockProtos((store, id) => (store === 0 ? full.get(id)?.decodedBodies : undefined)),
    ...createMockMocking({
      documentName: (store) => documents.find((d) => d.store === store)?.name,
      hostsOf: (store) => [...new Set(storeFlows(store).map((f) => f[4]))].sort().slice(0, 8),
      flowOf: (_store, id) => {
        const f = flows[id];
        return f && { method: f[3], path: f[6] };
      },
      saveRules: async (drafts) => {
        let order = Math.max(0, ...rules.map((r) => r.order));
        const created = drafts.map((d) => normalizeRule({ ...(d as Rule), id: crypto.randomUUID(), order: ++order }));
        rules = [...rules, ...created];
        return created;
      },
      usedBy: (source) => rules.filter((r) => r.actions.some((a) => a.type === "ServeFrom" && a.source === source)).map((r) => r.name),
    }),
    async appVersion() {
      return "0.1.0";
    },
    async getUpdateSettings() {
      return updateSettings;
    },
    async setUpdateSettings(settings) {
      updateSettings = settings;
      return settings;
    },
    async checkForUpdates() {},
    ...createMockScripts(
      now,
      async (store, id) => {
        const detail = await api.getFlowDetail(store, id);
        if (!detail) return null;
        const body = async (part: BodyPart) => new Uint8Array(await api.getBody(store, id, part, true).catch(() => new ArrayBuffer(0)));
        return { detail, bodies: { request: await body("request"), response: await body("response") } };
      },
      (id) => rules.filter((r) => r.actions.some((a) => a.type === "Script" && a.script_id === id)).map((r) => r.name),
    ),
  };
  return api;
}

/** `*.example.com` matches `example.com` and its subdomains, as in Rust. */
function hostMatches(host: string, pattern: string): boolean {
  if (!pattern.startsWith("*.")) return host === pattern;
  const domain = pattern.slice(2);
  return host === domain || host.endsWith(`.${domain}`);
}

/** The engine aggregates this in Rust; the mock does it the simple way. */
function sidebar(flows: FlowRow_Serialize[], store: number): SidebarSnapshot_Deserialize {
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
    store,
    total: flows.length,
    apps: [...apps]
      .sort(([a], [b]) => a.toLowerCase().localeCompare(b.toLowerCase()))
      .map(([name, count]) => ({ name, bundle_id: null, has_icon: false, remote: false, count })),
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

function mockPayload(m: WsMessage_Serialize): Uint8Array {
  return m.opcode === "binary" ? Uint8Array.from({ length: 48 }, (_, i) => (i * 37) % 256) : new TextEncoder().encode(m.preview);
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
      // A rule edited one, dropped another; Proxymate sent a third.
      edited: i === 7,
      injected: i === 12,
      dropped: i === 9,
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

/** Applied rules for mock flows marked edited. */
function mockFlowRules(edited: boolean, id: number): FlowRules_Serialize {
  const applied: FlowRules_Serialize["applied"] = edited
    ? [
        { rule_id: "mock-rewrite", name: "Add debug header", origin: { kind: "user" }, phase: "response" },
        ...(id % 2 === 0
          ? [{ rule_id: "mock-agent-rule", name: "Mock checkout (agent)", origin: { kind: "agent" as const, client: "claude-code", session: null, created_at_ms: 0 }, phase: "response" as const }]
          : []),
      ]
    : [];
  return { applied, rewrite_skipped: null, breakpoint_timeout: false, mocked: false, scripts: [] };
}

function mockBreakpoint(now: number): BreakpointInfo_Serialize {
  return {
    id: 1,
    flow_id: 3,
    phase: "request",
    rule_id: "mock-bp",
    rule_name: "Breakpoint · POST api.stripe.com/v1/charges",
    paused_at_ms: now,
    deadline_ms: now + 300_000,
    request: {
      method: "POST",
      url: "https://api.stripe.com/v1/charges?expand=customer",
      version: "HTTP/2",
      headers: [
        { name: "content-type", value: "application/json" },
        { name: "authorization", value: "Bearer sk_test_mock" },
        { name: "user-agent", value: "curl/8.7.1" },
      ],
    },
    response: null,
    body: { size: 48, editable: true, note: null },
    message: null,
  };
}

function mockMessageBreakpoint(now: number): BreakpointInfo_Serialize {
  return {
    id: 2,
    flow_id: 13,
    phase: "response",
    rule_id: "mock-ws-bp",
    rule_name: "Hold ticker messages",
    paused_at_ms: now,
    deadline_ms: now + 300_000,
    request: { method: "GET", url: "wss://stream.example.com/socket", version: "HTTP/1.1", headers: [] },
    response: null,
    body: { size: 48, editable: true, note: null },
    message: { direction: "received", opcode: "text" },
  };
}

function mockBody(kind: ContentKind): string {
  return kind === "json" ? JSON.stringify({ mock: true, items: [1, 2, 3], nested: { ok: true } }) : "mock body\n";
}

/** A save that reports progress in a few steps, like the engine's writer. */
function mockSave(
  all: number,
  selection: FlowSelection,
  path: string,
  onProgress: (p: SaveProgress_Serialize) => void,
): Promise<string> {
  const total = selection.kind === "ids" ? selection.ids.length : all;
  [0, 0.5, 1].forEach((f, i) =>
    setTimeout(() => onProgress({ done: Math.round(total * f), total, finished: f === 1, error: null, path }), i * 150),
  );
  return Promise.resolve(path);
}

/** The mock's matcher: method, host and path only. */
function mockMismatch(r: Rule, f: FlowRow_Serialize): string | null {
  const host = r.matcher.host;
  if (host && !(host.kind === "exact" ? host.value === f[4] : hostMatches(f[4], host.value))) return `host ${f[4]} doesn't match`;
  if (r.matcher.methods.length > 0 && !r.matcher.methods.includes(f[3])) return `method ${f[3]} isn't one of ${r.matcher.methods.join(", ")}`;
  const path = r.matcher.path;
  if (path && (path.kind === "exact" ? path.value !== f[6] : !globMatches(f[6], path.value))) return `path ${f[6]} doesn't match`;
  return null;
}

function globMatches(text: string, glob: string): boolean {
  const source = glob.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${source}$`, "i").test(text);
}

interface MockMessage {
  headers: { name: string; value: string }[];
  body: string;
  url: string;
  status: number;
}

/**
 * The visual actions of `phase`, applied the way the engine does (JSON
 * edits on an ordered document, so key order and numbers survive), for
 * the Rewrite dialog's preview in the browser. The rest only add a note.
 */
function mockApply(actions: Rule["actions"], phase: "request" | "response", m: MockMessage): MockMessage & { notes: string[] } {
  let { headers, body, url, status } = m;
  const notes: string[] = [];
  const query = (name: string, value: string | null) => {
    const u = new URL(url);
    if (value === null) u.searchParams.delete(name);
    else u.searchParams.set(name, value);
    url = u.toString();
  };
  for (const a of actions) {
    if ("phase" in a && a.phase !== phase) continue;
    switch (a.type) {
      case "SetHeader":
        headers = [...headers.filter((h) => h.name.toLowerCase() !== a.name.toLowerCase()), { name: a.name, value: a.value }];
        break;
      case "AddHeader":
        headers = [...headers, { name: a.name, value: a.value }];
        break;
      case "RemoveHeader":
        headers = headers.filter((h) => h.name.toLowerCase() !== a.name.toLowerCase());
        break;
      case "SetStatus":
        if (phase === "response") status = a.status;
        break;
      case "SetQuery":
      case "RemoveQuery":
        if (phase === "request") query(a.name, a.type === "SetQuery" ? a.value : null);
        break;
      case "SetBody":
        body = a.body;
        break;
      case "FormSet":
      case "FormRemove": {
        const form = new URLSearchParams(body);
        if (a.type === "FormSet") form.set(a.name, a.value);
        else form.delete(a.name);
        body = form.toString();
        break;
      }
      case "JsonSet":
      case "JsonRemove":
        try {
          const doc = parseDoc(body);
          const next = a.type === "JsonSet" ? setAtPointer(doc, a.pointer, parseDoc(a.raw ?? JSON.stringify(a.value))) : removeAtPointer(doc, a.pointer);
          body = toJson(next, /\n/.test(body) ? 2 : 0);
        } catch (e) {
          notes.push(`Body action skipped: ${e instanceof Error ? e.message : String(e)}`);
        }
        break;
      default:
        if (phase === "request") notes.push(`(mock) ${a.type} isn't simulated`);
    }
  }
  return { headers, body, url, status, notes };
}

function tokensOf(pointer: string): string[] {
  return pointer === "" ? [] : pointer.slice(1).split("/").map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function setAtPointer(root: JNode, pointer: string, value: JNode): JNode {
  const [token, ...rest] = tokensOf(pointer);
  if (token === undefined) return value;
  const inner = (child: JNode | undefined) => (rest.length === 0 ? value : setAtPointer(child ?? { t: "obj", entries: [] }, `/${rest.join("/")}`, value));
  if (root.t === "obj") {
    const at = root.entries.map((e) => e.key).lastIndexOf(token);
    if (at < 0) return { t: "obj", entries: [...root.entries, { id: newId(), key: token, v: inner(undefined) }] };
    return { t: "obj", entries: root.entries.map((e, i) => (i === at ? { ...e, v: inner(e.v) } : e)) };
  }
  if (root.t === "arr") {
    const i = token === "-" ? root.items.length : Number(token);
    if (i === root.items.length) return { t: "arr", items: [...root.items, { id: newId(), v: inner(undefined) }] };
    if (!(i >= 0 && i < root.items.length)) throw new Error(`${pointer}: index out of range`);
    return { t: "arr", items: root.items.map((item, j) => (j === i ? { ...item, v: inner(item.v) } : item)) };
  }
  throw new Error(`${pointer}: the parent isn't an object or array`);
}

function removeAtPointer(root: JNode, pointer: string): JNode {
  const [token, ...rest] = tokensOf(pointer);
  if (token === undefined) return root;
  if (root.t === "obj") {
    if (rest.length === 0) return { t: "obj", entries: root.entries.filter((e) => e.key !== token) };
    return { t: "obj", entries: root.entries.map((e) => (e.key === token ? { ...e, v: removeAtPointer(e.v, `/${rest.join("/")}`) } : e)) };
  }
  if (root.t === "arr") {
    const i = Number(token);
    if (rest.length === 0) return { t: "arr", items: root.items.filter((_, j) => j !== i) };
    return { t: "arr", items: root.items.map((item, j) => (j === i ? { ...item, v: removeAtPointer(item.v, `/${rest.join("/")}`) } : item)) };
  }
  return root;
}
