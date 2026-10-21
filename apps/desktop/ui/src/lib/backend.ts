import { Channel, isTauri } from "@tauri-apps/api/core";

import {
  commands,
  type BodyPart,
  type CaInfo,
  type FilterSpec_Deserialize,
  type FlowDetail_Serialize,
  type FlowSummary_Serialize,
  type ProxyStatus_Serialize,
  type RowsPage_Serialize,
  type SortSpec,
  type SslSettings,
  type ViewDelta,
  type ViewInfo,
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
  clearFlows: () => commands.clearFlows(),
  setRecording: (on) => commands.setRecording(on),
  getProxyStatus: () => commands.getProxyStatus(),
  getCaInfo: () => commands.getCaInfo(),
  regenerateCa: () => unwrap(commands.regenerateCa()),
  getSslSettings: () => commands.getSslSettings(),
  setSslSettings: (settings) => unwrap(commands.setSslSettings(settings)),
  addSslBypass: (host) => unwrap(commands.addSslBypass(host)),
  setInsecureUpstream: (host, allow) => unwrap(commands.setInsecureUpstream(host, allow)),
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
