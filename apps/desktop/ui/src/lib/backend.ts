import { getVersion } from "@tauri-apps/api/app";
import { Channel, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  commands,
  type AndroidStatus_Serialize,
  type ConsoleEntry_Serialize,
  type Script_Deserialize,
  type Script_Serialize,
  type ScriptSettings_Deserialize,
  type ScriptSettings_Serialize,
  type ScriptTestResult_Serialize,
  type ScriptTestSpec_Deserialize,
  type ViewResult_Serialize,
  type DecodedBody_Serialize,
  type ProtoMapping_Deserialize,
  type ProtoMapping_Serialize,
  type ProtoSchema,
  type MockSource,
  type OpenApiSpec,
  type AutoSaveSettings_Deserialize,
  type AutoSaveSettings_Serialize,
  type InterceptApp,
  type InterceptSession_Serialize,
  type InterceptTarget,
  type RecordingSettings_Deserialize,
  type RecordingSettings_Serialize,
  type OperationMatch_Serialize,
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
  type FlowSelection_Deserialize,
  type FlowLabel,
  type HeaderColumn,
  type CodeLanguage,
  type RepeatOpts_Deserialize,
  type RepeatProgress_Deserialize,
  type RepeatProgress_Serialize,
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
  type ProxySettings_Serialize,
  type NetworkSettings_Deserialize,
  type NetworkSettings_Serialize,
  type StorageSettings_Serialize,
  type StorageStatus_Deserialize,
  type StorageStatus_Serialize,
  type UpdateSettings,
  type ViewDelta_Deserialize,
  type ViewDelta_Serialize,
  type ViewInfo_Serialize,
  type WsPage_Serialize,
  type WsQuery_Deserialize,
  type WsSend,
} from "@/bindings";
import { createMockBackend } from "@/lib/mockBackend";

/**
 * The engine as seen by the UI. Mirrors the Tauri commands one to one.
 *
 * Flow ids are per store: `store` is 0 (`LIVE_STORE`) for the live
 * capture, or an opened document's. View ids already name their store.
 */
export interface Backend {
  createView(store: number, filter: FilterSpec_Deserialize, sort: SortSpec): Promise<ViewInfo_Serialize>;
  setFilter(view: number, filter: FilterSpec_Deserialize): Promise<ViewInfo_Serialize>;
  setSort(view: number, sort: SortSpec): Promise<ViewInfo_Serialize>;
  dropView(view: number): Promise<void>;
  getRows(view: number, start: number, count: number): Promise<RowsPage_Serialize>;
  getFlow(store: number, id: number): Promise<FlowSummary_Serialize | null>;
  subscribe(view: number, onDelta: (delta: ViewDelta_Serialize) => void): Promise<void>;
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
  /** What arrived for an edited message (M19). */
  getWebsocketOriginal(store: number, id: number, index: number): Promise<ArrayBuffer>;
  /** Sends a message into an open WebSocket of the live capture. */
  wsSend(id: number, message: WsSend): Promise<void>;
  /** Closes an open WebSocket of the live capture. */
  wsClose(id: number): Promise<void>;
  /** A window of an event-stream response's events matching `text`. */
  getSseEvents(store: number, id: number, text: string, start: number, count: number): Promise<SsePage_Serialize>;
  clearFlows(): Promise<void>;
  setRecording(on: boolean): Promise<ProxyStatus_Serialize>;
  getProxyStatus(): Promise<ProxyStatus_Serialize>;
  getCaInfo(): Promise<CaInfo>;
  /** Replaces the CA; with `removeOldTrust`, the old one's keychain trust is removed (macOS prompts). */
  regenerateCa(removeOldTrust: boolean): Promise<CaInfo>;
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
  /** A preset's unsaved rule from a flow in `store`. */
  draftRule(store: number, flow: number, preset: PresetKind): Promise<Rule_Serialize>;
  /** Dry run of a rule against a recorded flow (computed in Rust). */
  /** Dry run on a flow in `store`; bodies cut at `bodyLimit` bytes (default 256 KB). */
  testRule(store: number, rule: Rule_Deserialize, flow: number, bodyLimit?: number | null): Promise<RuleTestResult_Serialize>;
  /** Flows of `store` (newest first) the rule's matcher matches. */
  flowsMatchingRule(store: number, rule: Rule_Deserialize, limit: number): Promise<number[]>;
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

  // ---- Storage, proxy and appearance (M10) ----
  getStorageSettings(): Promise<StorageSettings_Serialize>;
  /** Saves the limits; the capture is evicted down to them straight away. */
  setStorageSettings(settings: StorageSettings_Serialize): Promise<StorageSettings_Serialize>;
  /** Flows, disk use and write errors, pushed now and whenever they change. */
  subscribeStorage(onStatus: (status: StorageStatus_Serialize) => void): Promise<void>;
  /** Pinned live flows are never evicted; a saved session keeps its pins. */
  pinFlows(store: number, selection: FlowSelection_Deserialize, pinned: boolean): Promise<number>;

  // ---- Workflow (M17) ----
  /** The ids a selection names, in its order. */
  selectedIds(store: number, selection: FlowSelection_Deserialize): Promise<number[]>;
  getFlowUrls(store: number, selection: FlowSelection_Deserialize): Promise<string[]>;
  /** Deletes flows (in-flight ones stay); resolves to how many went. */
  removeFlows(store: number, selection: FlowSelection_Deserialize): Promise<number>;
  /** Null takes the label off. */
  labelFlows(store: number, selection: FlowSelection_Deserialize, label: FlowLabel | null): Promise<number>;
  /** An empty comment removes it. */
  commentFlow(store: number, id: number, comment: string): Promise<void>;
  /** The header columns every store fills (rows' `extra`, in this order). */
  setHeaderColumns(columns: HeaderColumn[]): Promise<HeaderColumn[]>;
  /** The request as code in `language` (Copy as code). */
  getCode(store: number, id: number, language: CodeLanguage): Promise<string>;
  /** Repeats a recorded request; progress streams to `onProgress`, the last with `finished`. */
  startRepeat(
    store: number,
    id: number,
    opts: RepeatOpts_Deserialize,
    onProgress: (p: RepeatProgress_Serialize) => void,
  ): Promise<number>;
  cancelRepeat(id: number): Promise<boolean>;
  getProxySettings(): Promise<ProxySettings_Serialize>;
  // ---- Network (M18) ----
  /** Settings › Network; passwords never come back (`has_password` says one is saved). */
  getNetworkSettings(): Promise<NetworkSettings_Serialize>;
  /** Saves and applies it (passwords to the keychain, reverse proxies restarted); rejects with why. */
  setNetworkSettings(settings: NetworkSettings_Deserialize): Promise<NetworkSettings_Serialize>;
  /** Restarts the listener if the port or pool changed; rejects if the port can't be bound. */
  setProxySettings(settings: ProxySettings_Serialize): Promise<ProxySettings_Serialize>;
  /** Makes the sidebar count the store the table shows. */
  setSidebarStore(store: number): Promise<void>;
  /** The native window's appearance (vibrancy, traffic lights); null follows the system. */
  setWindowTheme(theme: "light" | "dark" | null): Promise<void>;
  // ---- Updates (M11) ----
  /** The app's version (`tauri.conf.json`). */
  appVersion(): Promise<string>;
  getUpdateSettings(): Promise<UpdateSettings>;
  setUpdateSettings(settings: UpdateSettings): Promise<UpdateSettings>;
  /** Same as the app menu's "Check for Updates…"; the answer is a native dialog. */
  checkForUpdates(): Promise<void>;

  // ---- Scripts (M14) ----
  /** Every script: view scripts, then rewrite, then mock; by name within each. */
  listScripts(): Promise<Script_Serialize[]>;
  /** Saves a new script (a fresh id is assigned); rejects with what's wrong with it. */
  createScript(script: Script_Deserialize): Promise<Script_Serialize>;
  updateScript(script: Script_Deserialize): Promise<Script_Serialize>;
  setScriptEnabled(id: string, enabled: boolean): Promise<Script_Serialize>;
  /** Rejects while a rule runs the script, naming those rules. */
  deleteScript(id: string): Promise<void>;
  /** Deletes the agent-created scripts no rule runs; resolves to how many. */
  deleteAgentScripts(): Promise<number>;
  /** Dry run of a script (saved, or unsaved source) against a recorded flow. */
  testScript(store: number, flow: number, spec: ScriptTestSpec_Deserialize): Promise<ScriptTestResult_Serialize>;
  /** What the enabled view scripts make of one body; empty when none applies. */
  getFlowViews(store: number, flow: number, part: BodyPart): Promise<ViewResult_Serialize[]>;
  /** A gRPC, protobuf, MessagePack or CBOR body as JSON trees (M16); live flows too. */
  decodeBody(store: number, flow: number, part: BodyPart): Promise<DecodedBody_Serialize>;
  listProtoSchemas(): Promise<ProtoSchema[]>;
  /** `.proto` files or one descriptor set; asks with a file dialog without `paths`. Null if cancelled. */
  importProtoSchema(paths: string[] | null): Promise<ProtoSchema | null>;
  removeProtoSchema(id: number): Promise<void>;
  /** Every message type the imported schemas define, for "Decode as…". */
  listProtoMessageTypes(): Promise<string[]>;
  listProtoMappings(): Promise<ProtoMapping_Serialize[]>;
  setProtoMappings(mappings: ProtoMapping_Deserialize[]): Promise<void>;
  /** The mock library ServeFrom rules answer from (M20). */
  listMockSources(): Promise<MockSource[]>;
  /** "Mock from session…": copies a store (a document, or the live capture) into the library. */
  createMockSource(store: number, name: string | null): Promise<MockSource>;
  /** Copies a session, HAR or Charles file; asks for one without `path`. Null if cancelled. */
  createMockSourceFromFile(path: string | null): Promise<MockSource | null>;
  removeMockSource(id: string): Promise<void>;
  /** The "Serve from" rule for a library entry, unsaved. */
  draftServeFromRule(source: string): Promise<Rule_Serialize>;
  listOpenApiSpecs(): Promise<OpenApiSpec[]>;
  /** An OpenAPI 3.0/3.1 file (JSON or YAML); asks for one without `path`. Null if cancelled. */
  importOpenApiSpec(path: string | null): Promise<OpenApiSpec | null>;
  removeOpenApiSpec(id: number): Promise<void>;
  /** "Generate mocks": saves one disabled Map Local rule per operation. */
  generateOpenApiMocks(id: number): Promise<Rule_Serialize[]>;
  /** The OpenAPI operation a flow's method and path match, if any. */
  flowOperation(store: number, flow: number): Promise<OperationMatch_Serialize | null>;
  /** Console entries newer than `since`, oldest first, at most `limit`. */
  getScriptConsole(since: number | null, limit: number): Promise<ConsoleEntry_Serialize[]>;
  clearScriptConsole(): Promise<void>;
  /** The newest console sequence number, pushed whenever entries are added. */
  subscribeScriptConsole(onSeq: (seq: number) => void): Promise<void>;
  /** The script list, pushed now and whenever the UI or an agent changes it. */
  subscribeScripts(onScripts: (scripts: Script_Serialize[]) => void): Promise<void>;
  getScriptSettings(): Promise<ScriptSettings_Serialize>;
  setScriptSettings(settings: ScriptSettings_Deserialize): Promise<ScriptSettings_Serialize>;
  /** `proxymate.d.ts`. */
  getScriptTypeDefinitions(): Promise<string>;
  /** Saves a script as `.js` (asking where without `path`); null if cancelled. */
  exportScript(id: string, path: string | null): Promise<string | null>;
  /** Imports a `.js` file (asking which without `path`); null if cancelled. */
  importScript(path: string | null): Promise<Script_Serialize | null>;

  // ---- Capture targets and noise (M21) ----
  /** The Intercept menu's terminals and browsers, and which are installed. */
  listInterceptApps(): Promise<InterceptApp[]>;
  /** Intercepted launches still running. */
  listIntercepts(): Promise<InterceptSession_Serialize[]>;
  /** Starts a terminal or browser set up for the proxy. */
  intercept(target: InterceptTarget): Promise<InterceptSession_Serialize>;
  getRecordingSettings(): Promise<RecordingSettings_Serialize>;
  /** Saves the recording lists (normalized: lower case, no scheme, port or path). */
  setRecordingSettings(settings: RecordingSettings_Deserialize): Promise<RecordingSettings_Serialize>;
  /** "Never record this host". */
  neverRecordHost(host: string): Promise<RecordingSettings_Serialize>;
  getAutosaveSettings(): Promise<AutoSaveSettings_Serialize>;
  setAutosaveSettings(settings: AutoSaveSettings_Deserialize): Promise<AutoSaveSettings_Serialize>;
  /** Saves the live capture into the auto-save folder now; resolves to the flows it writes. */
  autosaveNow(): Promise<number>;
  /** Asks for the auto-save folder; null if cancelled. */
  chooseAutosaveFolder(): Promise<string | null>;
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
    // The channel carries what Rust serializes; the binding names the other shape.
    const channel = new Channel<ViewDelta_Serialize>();
    channel.onmessage = onDelta;
    await unwrap(commands.subscribeView(view, channel as unknown as Channel<ViewDelta_Deserialize>));
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
  getWebsocketOriginal: async (store, id, index) =>
    (await unwrap(commands.getWebsocketOriginal(store, id, index))) as ArrayBuffer,
  wsSend: async (id, message) => {
    await unwrap(commands.wsSend(id, message));
  },
  wsClose: async (id) => {
    await unwrap(commands.wsClose(id));
  },
  getSseEvents: (store, id, text, start, count) => unwrap(commands.getSseEvents(store, id, text, start, count)),
  clearFlows: () => commands.clearFlows(),
  setRecording: (on) => commands.setRecording(on),
  getProxyStatus: () => commands.getProxyStatus(),
  getCaInfo: () => commands.getCaInfo(),
  regenerateCa: (removeOldTrust) => unwrap(commands.regenerateCa(removeOldTrust)),
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
  draftRule: (store, flow, preset) => unwrap(commands.draftRule(store, flow, preset)),
  testRule: (store, rule, flow, bodyLimit = null) => unwrap(commands.testRule(store, rule, flow, bodyLimit)),
  flowsMatchingRule: (store, rule, limit) => unwrap(commands.flowsMatchingRule(store, rule, limit)),
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
  getStorageSettings: () => commands.getStorageSettings(),
  setStorageSettings: (settings) => unwrap(commands.setStorageSettings(settings)),
  subscribeStorage: async (onStatus) => {
    const channel = new Channel<StorageStatus_Serialize>();
    channel.onmessage = onStatus;
    await commands.subscribeStorage(channel as unknown as Channel<StorageStatus_Deserialize>);
  },
  pinFlows: (store, selection, pinned) => unwrap(commands.pinFlows(store, selection, pinned)),
  selectedIds: (store, selection) => unwrap(commands.selectedIds(store, selection)),
  getFlowUrls: (store, selection) => unwrap(commands.getFlowUrls(store, selection)),
  removeFlows: (store, selection) => unwrap(commands.removeFlows(store, selection)),
  labelFlows: (store, selection, label) => unwrap(commands.labelFlows(store, selection, label)),
  commentFlow: async (store, id, comment) => {
    await unwrap(commands.commentFlow(store, id, comment));
  },
  setHeaderColumns: (columns) => unwrap(commands.setHeaderColumns(columns)),
  getCode: (store, id, language) => unwrap(commands.getCode(store, id, language)),
  startRepeat: (store, id, opts, onProgress) => {
    const channel = new Channel<RepeatProgress_Serialize>();
    channel.onmessage = onProgress;
    return unwrap(commands.startRepeat(store, id, opts, channel as unknown as Channel<RepeatProgress_Deserialize>));
  },
  cancelRepeat: (id) => commands.cancelRepeat(id),
  getProxySettings: () => commands.getProxySettings(),
  getNetworkSettings: () => commands.getNetworkSettings(),
  setNetworkSettings: (settings) => unwrap(commands.setNetworkSettings(settings)),
  setProxySettings: (settings) => unwrap(commands.setProxySettings(settings)),
  setSidebarStore: (store) => commands.setSidebarStore(store),
  setWindowTheme: async (theme) => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTheme(theme);
  },
  appVersion: () => getVersion(),
  getUpdateSettings: () => commands.getUpdateSettings(),
  setUpdateSettings: (settings) => unwrap(commands.setUpdateSettings(settings)),
  checkForUpdates: () => commands.checkForUpdates(),
  listScripts: () => commands.listScripts(),
  createScript: (script) => unwrap(commands.createScript(script)),
  updateScript: (script) => unwrap(commands.updateScript(script)),
  setScriptEnabled: (id, enabled) => unwrap(commands.setScriptEnabled(id, enabled)),
  deleteScript: async (id) => {
    await unwrap(commands.deleteScript(id));
  },
  deleteAgentScripts: () => unwrap(commands.deleteAgentScripts()),
  testScript: (store, flow, spec) => unwrap(commands.testScript(store, flow, spec)),
  getFlowViews: (store, flow, part) => unwrap(commands.getFlowViews(store, flow, part)),
  decodeBody: (store, flow, part) => unwrap(commands.decodeBody(store, flow, part)),
  listProtoSchemas: () => commands.listProtoSchemas(),
  importProtoSchema: (paths) => unwrap(commands.importProtoSchema(paths)),
  removeProtoSchema: async (id) => {
    await unwrap(commands.removeProtoSchema(id));
  },
  listProtoMessageTypes: () => commands.listProtoMessageTypes(),
  listProtoMappings: () => commands.listProtoMappings(),
  setProtoMappings: async (mappings) => {
    await unwrap(commands.setProtoMappings(mappings));
  },
  listMockSources: () => commands.listMockSources(),
  createMockSource: (store, name) => unwrap(commands.createMockSource(store, name)),
  createMockSourceFromFile: (path) => unwrap(commands.createMockSourceFromFile(path)),
  removeMockSource: async (id) => {
    await unwrap(commands.removeMockSource(id));
  },
  draftServeFromRule: (source) => unwrap(commands.draftServeFromRule(source)),
  listOpenApiSpecs: () => commands.listOpenapiSpecs(),
  importOpenApiSpec: (path) => unwrap(commands.importOpenapiSpec(path)),
  removeOpenApiSpec: async (id) => {
    await unwrap(commands.removeOpenapiSpec(id));
  },
  generateOpenApiMocks: (id) => unwrap(commands.generateOpenapiMocks(id)),
  flowOperation: (store, flow) => unwrap(commands.flowOperation(store, flow)),
  getScriptConsole: (since, limit) => commands.getScriptConsole(since, limit),
  clearScriptConsole: () => commands.clearScriptConsole(),
  subscribeScriptConsole: async (onSeq) => {
    const channel = new Channel<number>();
    channel.onmessage = onSeq;
    await commands.subscribeScriptConsole(channel);
  },
  subscribeScripts: async (onScripts) => {
    // The channel carries what Rust serializes; the binding names the other shape.
    const channel = new Channel<Script_Serialize[]>();
    channel.onmessage = onScripts;
    await commands.subscribeScripts(channel as unknown as Channel<Script_Deserialize[]>);
  },
  getScriptSettings: () => commands.getScriptSettings(),
  setScriptSettings: (settings) => unwrap(commands.setScriptSettings(settings)),
  getScriptTypeDefinitions: () => commands.getScriptTypeDefinitions(),
  exportScript: (id, path) => unwrap(commands.exportScript(id, path)),
  importScript: (path) => unwrap(commands.importScript(path)),
  listInterceptApps: () => commands.listInterceptApps(),
  listIntercepts: () => commands.listIntercepts(),
  intercept: (target) => unwrap(commands.intercept(target)),
  getRecordingSettings: () => commands.getRecordingSettings(),
  setRecordingSettings: (settings) => unwrap(commands.setRecordingSettings(settings)),
  neverRecordHost: (host) => unwrap(commands.neverRecordHost(host)),
  getAutosaveSettings: () => commands.getAutosaveSettings(),
  setAutosaveSettings: (settings) => unwrap(commands.setAutosaveSettings(settings)),
  autosaveNow: () => unwrap(commands.autosaveNow()),
  chooseAutosaveFolder: () => unwrap(commands.chooseAutosaveFolder()),
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
