import { Channel, isTauri } from "@tauri-apps/api/core";

import {
  commands,
  type BodyPart,
  type FilterSpec_Deserialize,
  type FlowDetail_Serialize,
  type FlowSummary_Serialize,
  type ProxyStatus_Serialize,
  type RowsPage_Serialize,
  type SortSpec,
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
  /** Body bytes; with `decoded`, content-encoding undone (rejects if that fails). */
  getBody(id: number, part: BodyPart, decoded: boolean): Promise<ArrayBuffer>;
  clearFlows(): Promise<void>;
  setRecording(on: boolean): Promise<ProxyStatus_Serialize>;
  getProxyStatus(): Promise<ProxyStatus_Serialize>;
}

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };

async function unwrap<T>(result: Promise<Result<T>>): Promise<T> {
  const r = await result;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

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
  getBody: async (id, part, decoded) => (await unwrap(commands.getBody(id, part, decoded))) as ArrayBuffer,
  clearFlows: () => commands.clearFlows(),
  setRecording: (on) => commands.setRecording(on),
  getProxyStatus: () => commands.getProxyStatus(),
};

/**
 * The Tauri backend inside the app; an in-memory mock in a plain browser
 * (`npm run dev` on its own) and in tests.
 */
export const backend: Backend = isTauri() ? tauriBackend : createMockBackend(100_000);
