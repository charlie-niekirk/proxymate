import { QueryClient, type DataTag } from "@tanstack/react-query";

import type {
  BodyPart,
  FlowDetail_Serialize,
  FlowSummary_Serialize,
  WsQuery_Deserialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";

/**
 * Shared TanStack Query client for flow detail and bodies. Exported so
 * non-React code (the view delta handler) can invalidate what changed.
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The engine pushes changes; nothing goes stale on a timer.
      staleTime: Infinity,
      gcTime: 30_000,
      retry: false,
      refetchOnWindowFocus: false,
    },
  },
});

/*
 * Every query's key (R4). One root per shape: a key's first element names
 * what it holds, and nothing else uses that root with other elements. Keys
 * of engine data carry the data's type (`DataTag`), so `useQuery`,
 * `getQueryData` and `setQueryData` check it; the query options that fetch
 * them are in `lib/queries.ts`. Prefix keys (`…RootKey`, `streamKey`,
 * `flowKey`'s parts) are for invalidating a family at once.
 */

/** What a `Backend` method resolves to. */
type Out<M extends keyof Backend> = Backend[M] extends (...args: never[]) => Promise<infer T> ? T : never;

/** Tags `key` with the type of the data stored under it. */
const typed =
  <T>() =>
  <const K extends readonly unknown[]>(key: K) =>
    key as DataTag<K, T>;

/** A flow as the inspector shows it: its summary, and its detail when recorded. */
export interface InspectedFlow {
  flow: FlowSummary_Serialize;
  /** Null for synthetic flows, which have no recorded messages. */
  detail: FlowDetail_Serialize | null;
}

/** A body's bytes, and how they were decoded. */
export interface LoadedBody {
  bytes: Uint8Array;
  /** Set when the bytes were decoded from this content-encoding. */
  decodedFrom?: string;
  /** Set when decoding failed and `bytes` are the raw, encoded body. */
  decodeError?: string;
}

// ---- flows ----

/**
 * A flow in a store (ids are per store): the inspector's summary and detail.
 * Also the prefix of the flow's summary and detail keys, so invalidating it
 * refreshes every reader of the flow.
 */
export const flowKey = (store: number, id: number | null) => typed<InspectedFlow | null>()(["flow", store, id]);
/** Every flow of a store, and every body: dropped when the store is cleared or closed. */
export const storeFlowsKey = (store: number) => ["flow", store] as const;
export const storeBodiesKey = (store: number) => ["body", store] as const;
export const flowSummaryKey = (store: number, id: number) => typed<Out<"getFlow">>()(["flow", store, id, "summary"]);
export const flowDetailKey = (store: number, id: number) => typed<Out<"getFlowDetail">>()(["flow", store, id, "detail"]);
/** A body as the inspector loads it. Not under flowKey: live-flow refreshes must not refetch bodies. */
export const bodyKey = (
  store: number,
  id: number,
  part: BodyPart,
  decoded: boolean,
  captured: number,
  finished: boolean,
  max: number | null,
) => typed<LoadedBody>()(["body", store, id, part, decoded, captured, finished, max]);
export const bodyUrlKey = (store: number, id: number, part: BodyPart, captured: number, finished: boolean) =>
  typed<string>()(["body-url", store, id, part, captured, finished]);
/** A body's size and text for Compare (M17). */
export const compareBodyKey = (store: number, id: number, part: BodyPart) =>
  typed<{ size: number; text: string }>()(["compare-body", store, id, part]);
/** Live state of a flow's message log (counts, open/closed); refetched as the flow changes. */
export const streamKey = (store: number, id: number | null) => ["stream", store, id] as const;
export const sseHeadKey = (store: number, id: number, text: string) =>
  typed<Out<"getSseEvents">>()([...streamKey(store, id), "sse", text]);
export const wsHeadKey = (store: number, id: number, query: WsQuery_Deserialize) =>
  typed<Out<"getWebsocketMessages">>()([...streamKey(store, id), "ws", query]);
/** Prefix of a WebSocket list's pages (`usePagedList` appends the page). */
export const wsPagesKey = (store: number, id: number, query?: WsQuery_Deserialize) =>
  (query === undefined ? ["ws-page", store, id] : ["ws-page", store, id, query]) as readonly unknown[];
export const wsPayloadKey = (store: number, id: number, index: number, original: boolean) =>
  typed<Uint8Array>()(["ws-payload", store, id, index, original]);
/** Prefix of an SSE list's pages. */
export const ssePagesKey = (store: number, id: number, text: string) => ["sse-page", store, id, text] as const;
/** A body's view-script results; `version` changes whenever a script does. */
export const flowViewsKey = (store: number, id: number, part: string, version: string) =>
  typed<Out<"getFlowViews">>()(["flow-views", store, id, part, version]);
/** Every decoded body; invalidated when schemas or mappings change. */
export const decodedRootKey = ["decoded"] as const;
export const decodedKey = (store: number, id: number, part: string, size: number, finished: boolean) =>
  typed<Out<"decodeBody">>()([...decodedRootKey, store, id, part, size, finished]);
/** Every flow's OpenAPI operation; invalidated when specs change. */
export const operationRootKey = ["operation"] as const;
export const operationKey = (store: number, id: number) => typed<Out<"flowOperation">>()([...operationRootKey, store, id]);
/** A held breakpoint's body (or WebSocket payload) as bytes; one shape for every reader. */
export const breakpointBodyKey = (id: number) => typed<Uint8Array>()(["breakpoint-body", id]);
/** The Rewrite dialog's loaded flow and rule, per opening. */
export const rewriteKey = (seq: number) => ["rewrite", seq] as const;

// ---- body viewers (keyed by the bytes or URL they work on) ----

export const jsonPrettyKey = (bytes: number) => typed<string>()(["json-pretty", bytes]);
export const jsonParseKey = (bytes: number) => ["json-parse", bytes] as const;
export const beautifyKey = (lang: string, bytes: number) => typed<string>()(["beautify", lang, bytes]);
export const imageMetaKey = (url: string) =>
  typed<{ orientation: number | null; animated: boolean }>()(["image-meta", url]);
export const fontKey = (url: string) => typed<FontFace>()(["font", url]);

// ---- the app ----

export const documentsKey = typed<Out<"listDocuments">>()(["documents"]);
export const proxyStatusKey = typed<Out<"getProxyStatus">>()(["proxy-status"]);
export const caInfoKey = typed<Out<"getCaInfo">>()(["ca-info"]);
export const sslSettingsKey = typed<Out<"getSslSettings">>()(["ssl-settings"]);
export const trustStatusKey = typed<Out<"getTrustStatus">>()(["trust-status"]);
export const systemProxyKey = typed<Out<"getSystemProxyStatus">>()(["system-proxy"]);
export const terminalEnvKey = typed<Out<"getTerminalEnv">>()(["terminal-env"]);
export const rulesKey = typed<Out<"listRules">>()(["rules"]);
export const ruleSettingsKey = typed<Out<"getRuleSettings">>()(["rule-settings"]);
export const scriptsKey = typed<Out<"listScripts">>()(["scripts"]);
export const scriptSettingsKey = typed<Out<"getScriptSettings">>()(["script-settings"]);
/** The script console, refetched whenever its newest sequence number moves. */
export const scriptConsoleKey = (latest: number) => typed<Out<"getScriptConsole">>()(["script-console", latest]);
export const deviceAccessKey = typed<Out<"getDeviceAccess">>()(["device-access"]);
export const simulatorKey = typed<Out<"getSimulatorStatus">>()(["simulator"]);
export const androidKey = typed<Out<"getAndroidStatus">>()(["android"]);
export const mcpTokenKey = typed<Out<"getMcpToken">>()(["mcp-token"]);
/** Prefix of the audit log's pages. */
export const mcpAuditKey = ["mcp-audit"] as const;
export const mcpAuditPageKey = (limit: number) => typed<Out<"getMcpAudit">>()([...mcpAuditKey, limit]);
export const proxySettingsKey = typed<Out<"getProxySettings">>()(["proxy-settings"]);
export const networkSettingsKey = typed<Out<"getNetworkSettings">>()(["network-settings"]);
export const storageSettingsKey = typed<Out<"getStorageSettings">>()(["storage-settings"]);
export const updateSettingsKey = typed<Out<"getUpdateSettings">>()(["update-settings"]);
export const appVersionKey = typed<Out<"appVersion">>()(["app-version"]);
/** Imported protobuf schemas, their message types and "Decode as…" mappings (M16). */
export const protoSchemasKey = typed<Out<"listProtoSchemas">>()(["proto-schemas"]);
export const protoTypesKey = typed<Out<"listProtoMessageTypes">>()(["proto-types"]);
export const protoMappingsKey = typed<Out<"listProtoMappings">>()(["proto-mappings"]);
/** The mock library and imported OpenAPI specs (M20). */
export const mockSourcesKey = typed<Out<"listMockSources">>()(["mock-sources"]);
export const openApiSpecsKey = typed<Out<"listOpenApiSpecs">>()(["openapi-specs"]);
// M21
export const interceptAppsKey = typed<Out<"listInterceptApps">>()(["intercept-apps"]);
export const interceptsKey = typed<Out<"listIntercepts">>()(["intercepts"]);
export const recordingSettingsKey = typed<Out<"getRecordingSettings">>()(["recording-settings"]);
export const autosaveSettingsKey = typed<Out<"getAutosaveSettings">>()(["autosave-settings"]);
