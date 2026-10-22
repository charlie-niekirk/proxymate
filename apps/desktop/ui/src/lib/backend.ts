import { getVersion } from "@tauri-apps/api/app";
import { Channel, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  commands,
  type AndroidStatus_Serialize,
  type BodyPart,
  type BreakpointInfo_Deserialize,
  type BreakpointInfo_Serialize,
  type BreakpointResolution_Deserialize,
  type PresetKind,
  type Rule_Deserialize,
  type Rule_Serialize,
  type RuleSettings,
  type RuleTestResult_Serialize,
  type CaInfo,
  type DeviceAccess_Serialize,
  type DevicesSnapshot_Deserialize,
  type DevicesSnapshot_Serialize,
  type DocumentInfo,
  type FilterSpec_Deserialize,
  type FlowSelection,
  type FlowDetail_Serialize,
  type FlowSummary_Serialize,
  type McpAuditPage_Serialize,
  type McpSettings,
  type McpStatus_Deserialize,
  type McpStatus_Serialize,
  type ProxyStatus_Serialize,
  type RequestSpec_Deserialize,
  type RequestSpec_Serialize,
  type RowsPage_Serialize,
  type SaveProgress_Deserialize,
  type SaveProgress_Serialize,
  type SidebarSnapshot_Deserialize,
  type SimInstallResult_Serialize,
  type SimulatorStatus_Serialize,
  type SortSpec,
  type SsePage_Serialize,
  type SslSettings,
  type SystemProxyStatus_Serialize,
  type TrustStatus_Serialize,
  type UpdateSettings,
  type ViewDelta,
  type ViewInfo,
  type WsPage_Serialize,
  type WsQuery_Deserialize,
} from "@/bindings";
import { createMockBackend } from "@/lib/mockBackend";

/**
 * The engine as seen by the UI. Mirrors the Tauri commands one to one.
 *
 * Flow ids are per store: `store` is 0 (`LIVE_STORE`) for the live
 * capture, or an opened document's. View ids already name their store.
 */
export interface Backend {
  createView(store: number, filter: FilterSpec_Deserialize, sort: SortSpec): Promise<ViewInfo>;
  setFilter(view: number, filter: FilterSpec_Deserialize): Promise<ViewInfo>;
  setSort(view: number, sort: SortSpec): Promise<ViewInfo>;
  dropView(view: number): Promise<void>;
  getRows(view: number, start: number, count: number): Promise<RowsPage_Serialize>;
  getFlow(store: number, id: number): Promise<FlowSummary_Serialize | null>;
  subscribe(view: number, onDelta: (delta: ViewDelta) => void): Promise<void>;
  getFlowDetail(store: number, id: number): Promise<FlowDetail_Serialize | null>;
  /**
   * Body bytes; with `decoded`, content-encoding undone (rejects if that
   * fails). With `max`, only the first `max` bytes.
   */
  getBody(store: number, id: number, part: BodyPart, decoded: boolean, max?: number | null): Promise<ArrayBuffer>;
  /**
   * A URL serving the body with its detected Content-Type, for `<img>`,
   * `<video>`, PDF frames and fonts (`pmbody://` in the app).
   */
  bodyUrl(store: number, id: number, part: BodyPart, decoded: boolean): Promise<string>;
  /** Asks where to save a body and writes it; resolves to the path, or null if cancelled. */
  saveBody(store: number, id: number, part: BodyPart, decoded: boolean, fileName: string): Promise<string | null>;
  /** A window of a WebSocket flow's messages matching `query`. */
  getWebsocketMessages(
    store: number,
    id: number,
    query: WsQuery_Deserialize,
    start: number,
    count: number,
  ): Promise<WsPage_Serialize>;
  /** One message's recorded payload (at most 1 MB). */
  getWebsocketPayload(store: number, id: number, index: number): Promise<ArrayBuffer>;
  /** A window of an event-stream response's events matching `text`. */
  getSseEvents(store: number, id: number, text: string, start: number, count: number): Promise<SsePage_Serialize>;
  clearFlows(): Promise<void>;
  setRecording(on: boolean): Promise<ProxyStatus_Serialize>;
  getProxyStatus(): Promise<ProxyStatus_Serialize>;
  getCaInfo(): Promise<CaInfo>;
  regenerateCa(): Promise<CaInfo>;
  getSslSettings(): Promise<SslSettings>;
  setSslSettings(settings: SslSettings): Promise<SslSettings>;
  /** Stops decrypting `host`: new CONNECTs to it are tunnelled. */
  addSslBypass(host: string): Promise<SslSettings>;
  /** Accepts `host`'s upstream certificate without verification. */
  setInsecureUpstream(host: string, allow: boolean): Promise<SslSettings>;
  /** Apps and Domains with counts, pushed whenever they change. */
  subscribeSidebar(onSnapshot: (snapshot: SidebarSnapshot_Deserialize) => void): Promise<void>;
  /** Stops (or resumes) decrypting an app, by bundle id or name. */
  setAppSslExcluded(app: string, excluded: boolean): Promise<SslSettings>;
  /** Keychain trust plus a real handshake check through the proxy. */
  getTrustStatus(): Promise<TrustStatus_Serialize>;
  /** Resolves once the macOS password / Touch ID prompt is answered. */
  installCaTrust(): Promise<TrustStatus_Serialize>;
  uninstallCaTrust(): Promise<TrustStatus_Serialize>;
  getSystemProxyStatus(): Promise<SystemProxyStatus_Serialize>;
  /** Failures come back in `error`, not as a rejection. */
  setSystemProxy(on: boolean): Promise<SystemProxyStatus_Serialize>;
  /** `export` lines for a proxied terminal. */
  getTerminalEnv(): Promise<string>;

  // ---- Rules (M5) ----
  /** Every rule, in evaluation order (built-in ones included). */
  listRules(): Promise<Rule_Serialize[]>;
  /** Saves a new rule at the end; the id is assigned by the engine. */
  createRule(rule: Rule_Deserialize): Promise<Rule_Serialize>;
  updateRule(rule: Rule_Deserialize): Promise<Rule_Serialize>;
  setRuleEnabled(id: string, enabled: boolean): Promise<Rule_Serialize>;
  deleteRule(id: string): Promise<void>;
  /** Deletes every agent-created rule; resolves to how many. */
  deleteAgentRules(): Promise<number>;
  /** Puts the listed rules first, in that order. */
  reorderRules(ids: string[]): Promise<Rule_Serialize[]>;
  /** Rejects with the compile error if the rule is invalid. */
  validateRule(rule: Rule_Deserialize): Promise<void>;
  /** An unsaved rule pre-filled from a flow by a preset. */
  draftRule(flow: number, preset: PresetKind): Promise<Rule_Serialize>;
  /** Dry run of a rule against a recorded flow (computed in Rust). */
  testRule(rule: Rule_Deserialize, flow: number): Promise<RuleTestResult_Serialize>;
  getRuleSettings(): Promise<RuleSettings>;
  setRuleSettings(settings: RuleSettings): Promise<RuleSettings>;
  /** The rule list, pushed now and whenever the UI or an agent changes it. */
  subscribeRules(onRules: (rules: Rule_Serialize[]) => void): Promise<void>;

  // ---- Breakpoints (M5) ----
  /** The paused-flow queue, pushed now and whenever it changes. */
  subscribeBreakpoints(onQueue: (queue: BreakpointInfo_Serialize[]) => void): Promise<void>;
  /** A paused message's decoded body. */
  getBreakpointBody(id: number): Promise<ArrayBuffer>;
  /** Continue (with edits), abort or answer; the first resolution wins. */
  resolveBreakpoint(id: number, resolution: BreakpointResolution_Deserialize): Promise<void>;

  // ---- Device setup (M9) ----
  /** LAN interfaces, the chosen one, the setup URL and its QR code (SVG). */
  getDeviceAccess(): Promise<DeviceAccess_Serialize>;
  /** Listens on every interface, or loopback only; failures come back in `error`. */
  setLanAccess(on: boolean): Promise<DeviceAccess_Serialize>;
  /** Picks the interface the toolbar and QR codes show. */
  setLanInterface(name: string): Promise<DeviceAccess_Serialize>;
  /** LAN clients (pending prompts first) and device activity, pushed on change. */
  subscribeDevices(onSnapshot: (snapshot: DevicesSnapshot_Deserialize) => void): Promise<void>;
  /** Allows or denies a LAN client; its held connections proceed or are refused. */
  decideDevice(ip: string, allow: boolean, nickname: string | null): Promise<DevicesSnapshot_Serialize>;
  /** Drops a LAN client from the list; it is asked about again. */
  forgetDevice(ip: string): Promise<DevicesSnapshot_Serialize>;
  getSimulatorStatus(): Promise<SimulatorStatus_Serialize>;
  /** Adds the CA to these booted simulators (all booted if empty). */
  installSimulatorCert(udids: string[]): Promise<SimInstallResult_Serialize[]>;
  getAndroidStatus(): Promise<AndroidStatus_Serialize>;
  /** Points an emulator's global proxy at us (`10.0.2.2:<port>`), or clears it. */
  setAndroidProxy(serial: string, on: boolean): Promise<AndroidStatus_Serialize>;
  /** Copies the CA to the device's Downloads; resolves to the path there. */
  pushAndroidCert(serial: string): Promise<string>;

  // ---- MCP (M7) ----
  getMcpStatus(): Promise<McpStatus_Serialize>;
  /** Settings, where it serves and connected clients, pushed on change. */
  subscribeMcp(onStatus: (status: McpStatus_Serialize) => void): Promise<void>;
  /** Starts or stops the server, or flips read-only (clients see the tool list change). */
  setMcpSettings(settings: McpSettings): Promise<McpSettings>;
  /** The bearer token clients send. */
  getMcpToken(): Promise<string>;
  /** A new token; clients using the old one are refused from then on. */
  regenerateMcpToken(): Promise<string>;
  /** Tool calls older than `before` (newest first). */
  getMcpAudit(before: number | null, limit: number): Promise<McpAuditPage_Serialize>;
  clearMcpAudit(): Promise<void>;

  // ---- Replay, compose, cURL (M8) ----
  /** Sends a composed request; resolves to the new live flow's id once recorded. */
  sendRequest(spec: RequestSpec_Deserialize, applyRules: boolean): Promise<number>;
  /** Sends a recorded request again, as it was sent. */
  replayFlow(store: number, id: number, applyRules: boolean): Promise<number>;
  /** A recorded request with its body decoded, for Edit & Replay. */
  getRequestSpec(store: number, id: number): Promise<RequestSpec_Serialize>;
  /** A shell command that sends the request again with curl. */
  getCurl(store: number, id: number): Promise<string>;

  // ---- Documents: sessions and HAR (M8) ----
  listDocuments(): Promise<DocumentInfo[]>;
  /** Opens a `.proxymate` session or a HAR; asks for the file without `path`. Null if cancelled. */
  openDocument(path: string | null, har: boolean): Promise<DocumentInfo | null>;
  closeDocument(store: number): Promise<void>;
  /**
   * Saves flows as a `.proxymate` session in the background (asking where
   * without `path`). Resolves to the path once started, or null if cancelled;
   * `onProgress` hears the rest.
   */
  saveSession(
    store: number,
    selection: FlowSelection,
    path: string | null,
    fileName: string,
    onProgress: (p: SaveProgress_Serialize) => void,
  ): Promise<string | null>;
  /** Exports flows as a HAR file in the background; like `saveSession`. */
  exportHar(
    store: number,
    selection: FlowSelection,
    path: string | null,
    fileName: string,
    onProgress: (p: SaveProgress_Serialize) => void,
  ): Promise<string | null>;
  /** File menu clicks (`open`, `import-har`, `save-session`, `export-har`, `compose`). */
  onMenu(handler: (id: string) => void): Promise<() => void>;

  // ---- Updates (M11) ----
  /** The app's version (`tauri.conf.json`). */
  appVersion(): Promise<string>;
  getUpdateSettings(): Promise<UpdateSettings>;
  setUpdateSettings(settings: UpdateSettings): Promise<UpdateSettings>;
  /** Same as the app menu's "Check for Updates…"; the answer is a native dialog. */
  checkForUpdates(): Promise<void>;
}

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };

async function unwrap<T>(result: Promise<Result<T>>): Promise<T> {
  const r = await result;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

/** The `pmbody://` scheme (see src-tauri `body_scheme.rs`); WebView2 needs the http form. */
const BODY_ORIGIN = navigator.userAgent.includes("Windows") ? "http://pmbody.localhost" : "pmbody://localhost";

const tauriBackend: Backend = {
  createView: (store, filter, sort) => unwrap(commands.createView(store, filter, sort)),
  setFilter: (view, filter) => unwrap(commands.setFilter(view, filter)),
  setSort: (view, sort) => unwrap(commands.setSort(view, sort)),
  dropView: (view) => commands.dropView(view),
  getRows: (view, start, count) => unwrap(commands.getRows(view, start, count)),
  getFlow: (store, id) => commands.getFlow(store, id),
  subscribe: async (view, onDelta) => {
    const channel = new Channel<ViewDelta>();
    channel.onmessage = onDelta;
    await unwrap(commands.subscribeView(view, channel));
  },
  getFlowDetail: (store, id) => unwrap(commands.getFlowDetail(store, id)),
  // `get_body` answers with a raw ipc::Response, which arrives as an ArrayBuffer;
  // the binding can only say `unknown`.
  getBody: async (store, id, part, decoded, max = null) =>
    (await unwrap(commands.getBody(store, id, part, decoded, max))) as ArrayBuffer,
  bodyUrl: async (store, id, part, decoded) =>
    `${BODY_ORIGIN}/${store === 0 ? "live" : store}/${id}/${part}${decoded ? "?decoded=1" : ""}`,
  saveBody: (store, id, part, decoded, fileName) => unwrap(commands.saveBody(store, id, part, decoded, fileName)),
  getWebsocketMessages: (store, id, query, start, count) =>
    unwrap(commands.getWebsocketMessages(store, id, query, start, count)),
  getWebsocketPayload: async (store, id, index) =>
    (await unwrap(commands.getWebsocketPayload(store, id, index))) as ArrayBuffer,
  getSseEvents: (store, id, text, start, count) => unwrap(commands.getSseEvents(store, id, text, start, count)),
  clearFlows: () => commands.clearFlows(),
  setRecording: (on) => commands.setRecording(on),
  getProxyStatus: () => commands.getProxyStatus(),
  getCaInfo: () => commands.getCaInfo(),
  regenerateCa: () => unwrap(commands.regenerateCa()),
  getSslSettings: () => commands.getSslSettings(),
  setSslSettings: (settings) => unwrap(commands.setSslSettings(settings)),
  addSslBypass: (host) => unwrap(commands.addSslBypass(host)),
  setInsecureUpstream: (host, allow) => unwrap(commands.setInsecureUpstream(host, allow)),
  subscribeSidebar: async (onSnapshot) => {
    const channel = new Channel<SidebarSnapshot_Deserialize>();
    channel.onmessage = onSnapshot;
    await commands.subscribeSidebar(channel);
  },
  setAppSslExcluded: (app, excluded) => unwrap(commands.setAppSslExcluded(app, excluded)),
  getTrustStatus: () => unwrap(commands.getTrustStatus()),
  installCaTrust: () => unwrap(commands.installCaTrust()),
  uninstallCaTrust: () => unwrap(commands.uninstallCaTrust()),
  getSystemProxyStatus: () => unwrap(commands.getSystemProxyStatus()),
  setSystemProxy: (on) => unwrap(commands.setSystemProxy(on)),
  getTerminalEnv: () => commands.getTerminalEnv(),
  listRules: () => commands.listRules(),
  createRule: (rule) => unwrap(commands.createRule(rule)),
  updateRule: (rule) => unwrap(commands.updateRule(rule)),
  setRuleEnabled: (id, enabled) => unwrap(commands.setRuleEnabled(id, enabled)),
  deleteRule: async (id) => {
    await unwrap(commands.deleteRule(id));
  },
  deleteAgentRules: () => unwrap(commands.deleteAgentRules()),
  reorderRules: (ids) => unwrap(commands.reorderRules(ids)),
  validateRule: async (rule) => {
    await unwrap(commands.validateRule(rule));
  },
  draftRule: (flow, preset) => unwrap(commands.draftRule(flow, preset)),
  testRule: (rule, flow) => unwrap(commands.testRule(rule, flow)),
  getRuleSettings: () => commands.getRuleSettings(),
  setRuleSettings: (settings) => unwrap(commands.setRuleSettings(settings)),
  subscribeRules: async (onRules) => {
    const channel = new Channel<Rule_Serialize[]>();
    channel.onmessage = onRules;
    await commands.subscribeRules(channel as unknown as Channel<Rule_Deserialize[]>);
  },
  subscribeBreakpoints: async (onQueue) => {
    // The channel carries what Rust serializes; the binding names the other shape.
    const channel = new Channel<BreakpointInfo_Serialize[]>();
    channel.onmessage = onQueue;
    await commands.subscribeBreakpoints(channel as unknown as Channel<BreakpointInfo_Deserialize[]>);
  },
  getBreakpointBody: async (id) => (await unwrap(commands.getBreakpointBody(id))) as ArrayBuffer,
  resolveBreakpoint: async (id, resolution) => {
    await unwrap(commands.resolveBreakpoint(id, resolution));
  },
  getDeviceAccess: () => unwrap(commands.getDeviceAccess()),
  setLanAccess: (on) => unwrap(commands.setLanAccess(on)),
  setLanInterface: (name) => unwrap(commands.setLanInterface(name)),
  subscribeDevices: async (onSnapshot) => {
    const channel = new Channel<DevicesSnapshot_Deserialize>();
    channel.onmessage = onSnapshot;
    await commands.subscribeDevices(channel);
  },
  decideDevice: (ip, allow, nickname) => unwrap(commands.decideDevice(ip, allow, nickname)),
  forgetDevice: (ip) => unwrap(commands.forgetDevice(ip)),
  getSimulatorStatus: () => unwrap(commands.getSimulatorStatus()),
  installSimulatorCert: (udids) => unwrap(commands.installSimulatorCert(udids)),
  getAndroidStatus: () => unwrap(commands.getAndroidStatus()),
  setAndroidProxy: (serial, on) => unwrap(commands.setAndroidProxy(serial, on)),
  pushAndroidCert: (serial) => unwrap(commands.pushAndroidCert(serial)),
  getMcpStatus: () => commands.getMcpStatus(),
  subscribeMcp: async (onStatus) => {
    const channel = new Channel<McpStatus_Serialize>();
    channel.onmessage = onStatus;
    await commands.subscribeMcp(channel as unknown as Channel<McpStatus_Deserialize>);
  },
  setMcpSettings: (settings) => unwrap(commands.setMcpSettings(settings)),
  getMcpToken: () => commands.getMcpToken(),
  regenerateMcpToken: () => unwrap(commands.regenerateMcpToken()),
  getMcpAudit: (before, limit) => unwrap(commands.getMcpAudit(before, limit)),
  clearMcpAudit: async () => {
    await unwrap(commands.clearMcpAudit());
  },
  sendRequest: (spec, applyRules) => unwrap(commands.sendRequest(spec, applyRules)),
  replayFlow: (store, id, applyRules) => unwrap(commands.replayFlow(store, id, applyRules)),
  getRequestSpec: (store, id) => unwrap(commands.getRequestSpec(store, id)),
  getCurl: (store, id) => unwrap(commands.getCurl(store, id)),
  listDocuments: () => commands.listDocuments(),
  openDocument: (path, har) => unwrap(commands.openDocument(path, har)),
  closeDocument: async (store) => {
    await unwrap(commands.closeDocument(store));
  },
  saveSession: (store, selection, path, fileName, onProgress) =>
    unwrap(commands.saveSession(store, selection, path, fileName, progressChannel(onProgress))),
  exportHar: (store, selection, path, fileName, onProgress) =>
    unwrap(commands.exportHar(store, selection, path, fileName, progressChannel(onProgress))),
  onMenu: (handler) => listen<string>("pm-menu", (e) => handler(e.payload)),
  appVersion: () => getVersion(),
  getUpdateSettings: () => commands.getUpdateSettings(),
  setUpdateSettings: (settings) => unwrap(commands.setUpdateSettings(settings)),
  checkForUpdates: () => commands.checkForUpdates(),
};

function progressChannel(onProgress: (p: SaveProgress_Serialize) => void) {
  // The channel carries what Rust serializes; the binding names the other shape.
  const channel = new Channel<SaveProgress_Serialize>();
  channel.onmessage = onProgress;
  return channel as unknown as Channel<SaveProgress_Deserialize>;
}

function pickBackend(): Backend {
  if (isTauri()) return tauriBackend;
  // Dev only: `?zoo` serves the body-viewer fixtures (crates/pm-testkit/fixtures/zoo)
  // instead of synthetic rows, for the Playwright viewer tests.
  if (import.meta.env.DEV && new URLSearchParams(window.location.search).has("zoo")) {
    return createMockBackend(0, import("@/dev/zoo").then((m) => m.loadZoo()));
  }
  return createMockBackend(100_000);
}

/**
 * The Tauri backend inside the app; an in-memory mock in a plain browser
 * (`npm run dev` on its own) and in tests.
 */
export const backend: Backend = pickBackend();
