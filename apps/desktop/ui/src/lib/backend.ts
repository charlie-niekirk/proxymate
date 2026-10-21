import { Channel, isTauri } from "@tauri-apps/api/core";

import {
  commands,
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
  type FilterSpec_Deserialize,
  type FlowDetail_Serialize,
  type FlowSummary_Serialize,
  type ProxyStatus_Serialize,
  type RowsPage_Serialize,
  type SidebarSnapshot_Deserialize,
  type SortSpec,
  type SsePage_Serialize,
  type SslSettings,
  type SystemProxyStatus_Serialize,
  type TrustStatus_Serialize,
  type ViewDelta,
  type ViewInfo,
  type WsPage_Serialize,
  type WsQuery_Deserialize,
} from "@/bindings";
import { createMockBackend } from "@/lib/mockBackend";

/** The engine as seen by the UI. Mirrors the Tauri commands one to one. */
export interface Backend {
  createView(filter: FilterSpec_Deserialize, sort: SortSpec): Promise<ViewInfo>;
  setFilter(view: number, filter: FilterSpec_Deserialize): Promise<ViewInfo>;
  setSort(view: number, sort: SortSpec): Promise<ViewInfo>;
  dropView(view: number): Promise<void>;
  getRows(view: number, start: number, count: number): Promise<RowsPage_Serialize>;
  getFlow(id: number): Promise<FlowSummary_Serialize | null>;
  subscribe(view: number, onDelta: (delta: ViewDelta) => void): Promise<void>;
  getFlowDetail(id: number): Promise<FlowDetail_Serialize | null>;
  /**
   * Body bytes; with `decoded`, content-encoding undone (rejects if that
   * fails). With `max`, only the first `max` bytes.
   */
  getBody(id: number, part: BodyPart, decoded: boolean, max?: number | null): Promise<ArrayBuffer>;
  /**
   * A URL serving the body with its detected Content-Type, for `<img>`,
   * `<video>`, PDF frames and fonts (`pmbody://` in the app).
   */
  bodyUrl(id: number, part: BodyPart, decoded: boolean): Promise<string>;
  /** Asks where to save a body and writes it; resolves to the path, or null if cancelled. */
  saveBody(id: number, part: BodyPart, decoded: boolean, fileName: string): Promise<string | null>;
  /** A window of a WebSocket flow's messages matching `query`. */
  getWebsocketMessages(id: number, query: WsQuery_Deserialize, start: number, count: number): Promise<WsPage_Serialize>;
  /** One message's recorded payload (at most 1 MB). */
  getWebsocketPayload(id: number, index: number): Promise<ArrayBuffer>;
  /** A window of an event-stream response's events matching `text`. */
  getSseEvents(id: number, text: string, start: number, count: number): Promise<SsePage_Serialize>;
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

  // ---- Breakpoints (M5) ----
  /** The paused-flow queue, pushed now and whenever it changes. */
  subscribeBreakpoints(onQueue: (queue: BreakpointInfo_Serialize[]) => void): Promise<void>;
  /** A paused message's decoded body. */
  getBreakpointBody(id: number): Promise<ArrayBuffer>;
  /** Continue (with edits), abort or answer; the first resolution wins. */
  resolveBreakpoint(id: number, resolution: BreakpointResolution_Deserialize): Promise<void>;
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
  createView: (filter, sort) => commands.createView(filter, sort),
  setFilter: (view, filter) => unwrap(commands.setFilter(view, filter)),
  setSort: (view, sort) => unwrap(commands.setSort(view, sort)),
  dropView: (view) => commands.dropView(view),
  getRows: (view, start, count) => unwrap(commands.getRows(view, start, count)),
  getFlow: (id) => commands.getFlow(id),
  subscribe: async (view, onDelta) => {
    const channel = new Channel<ViewDelta>();
    channel.onmessage = onDelta;
    await unwrap(commands.subscribeView(view, channel));
  },
  getFlowDetail: (id) => unwrap(commands.getFlowDetail(id)),
  // `get_body` answers with a raw ipc::Response, which arrives as an ArrayBuffer;
  // the binding can only say `unknown`.
  getBody: async (id, part, decoded, max = null) => (await unwrap(commands.getBody(id, part, decoded, max))) as ArrayBuffer,
  bodyUrl: async (id, part, decoded) => `${BODY_ORIGIN}/live/${id}/${part}${decoded ? "?decoded=1" : ""}`,
  saveBody: (id, part, decoded, fileName) => unwrap(commands.saveBody(id, part, decoded, fileName)),
  getWebsocketMessages: (id, query, start, count) => unwrap(commands.getWebsocketMessages(id, query, start, count)),
  getWebsocketPayload: async (id, index) => (await unwrap(commands.getWebsocketPayload(id, index))) as ArrayBuffer,
  getSseEvents: (id, text, start, count) => unwrap(commands.getSseEvents(id, text, start, count)),
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
};

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
