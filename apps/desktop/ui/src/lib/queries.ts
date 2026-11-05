//! The engine reads behind each key in `lib/query.ts` (R4): one fetcher per
//! key, so every reader of a key gets the same shape, and each passes
//! TanStack Query's `signal` through to the backend. Components spread
//! these into `useQuery` and add `enabled` or `placeholderData` as needed.

import { queryOptions } from "@tanstack/react-query";

import type { BodyPart, WsQuery_Deserialize } from "@/bindings";
import { backend } from "@/lib/backend";
import {
  androidKey,
  appVersionKey,
  autosaveSettingsKey,
  breakpointBodyKey,
  caInfoKey,
  decodedKey,
  deviceAccessKey,
  flowDetailKey,
  flowKey,
  flowSummaryKey,
  flowViewsKey,
  interceptAppsKey,
  interceptsKey,
  mcpAuditPageKey,
  mcpTokenKey,
  mockSourcesKey,
  networkSettingsKey,
  openApiSpecsKey,
  operationKey,
  protoMappingsKey,
  protoSchemasKey,
  protoTypesKey,
  proxySettingsKey,
  proxyStatusKey,
  recordingSettingsKey,
  ruleSettingsKey,
  rulesKey,
  scriptConsoleKey,
  scriptSettingsKey,
  scriptsKey,
  simulatorKey,
  sseHeadKey,
  sslSettingsKey,
  storageSettingsKey,
  systemProxyKey,
  terminalEnvKey,
  trustStatusKey,
  updateSettingsKey,
  wsHeadKey,
  type InspectedFlow,
} from "@/lib/query";

export const queries = {
  // ---- flows ----
  /** The inspector's flow: summary and detail together; null once the store has no such flow. */
  inspected: (store: number, id: number | null) =>
    queryOptions({
      queryKey: flowKey(store, id),
      enabled: id !== null,
      queryFn: async ({ signal }): Promise<InspectedFlow | null> => {
        const [flow, detail] = await Promise.all([
          backend.getFlow(store, id!, signal),
          backend.getFlowDetail(store, id!, signal),
        ]);
        return flow ? { flow, detail } : null;
      },
    }),
  flowSummary: (store: number, id: number) =>
    queryOptions({ queryKey: flowSummaryKey(store, id), queryFn: ({ signal }) => backend.getFlow(store, id, signal) }),
  flowDetail: (store: number, id: number) =>
    queryOptions({ queryKey: flowDetailKey(store, id), queryFn: ({ signal }) => backend.getFlowDetail(store, id, signal) }),
  sseHead: (store: number, id: number, text: string) =>
    queryOptions({
      queryKey: sseHeadKey(store, id, text),
      queryFn: ({ signal }) => backend.getSseEvents(store, id, text, 0, 0, signal),
    }),
  wsHead: (store: number, id: number, query: WsQuery_Deserialize) =>
    queryOptions({
      queryKey: wsHeadKey(store, id, query),
      queryFn: ({ signal }) => backend.getWebsocketMessages(store, id, query, 0, 0, signal),
    }),
  flowViews: (store: number, id: number, part: BodyPart, version: string) =>
    queryOptions({
      queryKey: flowViewsKey(store, id, part, version),
      queryFn: ({ signal }) => backend.getFlowViews(store, id, part, signal),
    }),
  decoded: (store: number, id: number, part: BodyPart, size: number, finished: boolean) =>
    queryOptions({
      queryKey: decodedKey(store, id, part, size, finished),
      queryFn: ({ signal }) => backend.decodeBody(store, id, part, signal),
    }),
  operation: (store: number, id: number) =>
    queryOptions({ queryKey: operationKey(store, id), queryFn: ({ signal }) => backend.flowOperation(store, id, signal) }),
  breakpointBody: (id: number) =>
    queryOptions({
      queryKey: breakpointBodyKey(id),
      queryFn: async ({ signal }) => new Uint8Array(await backend.getBreakpointBody(id, signal)),
    }),

  // ---- the app ----
  proxyStatus: () => queryOptions({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() }),
  caInfo: () => queryOptions({ queryKey: caInfoKey, queryFn: () => backend.getCaInfo() }),
  sslSettings: () => queryOptions({ queryKey: sslSettingsKey, queryFn: () => backend.getSslSettings() }),
  trustStatus: () => queryOptions({ queryKey: trustStatusKey, queryFn: () => backend.getTrustStatus() }),
  systemProxy: () => queryOptions({ queryKey: systemProxyKey, queryFn: () => backend.getSystemProxyStatus() }),
  terminalEnv: () => queryOptions({ queryKey: terminalEnvKey, queryFn: () => backend.getTerminalEnv() }),
  rules: () => queryOptions({ queryKey: rulesKey, queryFn: () => backend.listRules() }),
  ruleSettings: () => queryOptions({ queryKey: ruleSettingsKey, queryFn: () => backend.getRuleSettings() }),
  scripts: () => queryOptions({ queryKey: scriptsKey, queryFn: () => backend.listScripts() }),
  scriptSettings: () => queryOptions({ queryKey: scriptSettingsKey, queryFn: () => backend.getScriptSettings() }),
  scriptConsole: (latest: number) =>
    queryOptions({
      queryKey: scriptConsoleKey(latest),
      queryFn: ({ signal }) => backend.getScriptConsole(null, 2000, signal),
    }),
  deviceAccess: () => queryOptions({ queryKey: deviceAccessKey, queryFn: () => backend.getDeviceAccess() }),
  simulator: () => queryOptions({ queryKey: simulatorKey, queryFn: () => backend.getSimulatorStatus() }),
  android: () => queryOptions({ queryKey: androidKey, queryFn: () => backend.getAndroidStatus() }),
  mcpToken: () => queryOptions({ queryKey: mcpTokenKey, queryFn: () => backend.getMcpToken() }),
  mcpAudit: (limit: number) =>
    queryOptions({ queryKey: mcpAuditPageKey(limit), queryFn: ({ signal }) => backend.getMcpAudit(null, limit, signal) }),
  proxySettings: () => queryOptions({ queryKey: proxySettingsKey, queryFn: () => backend.getProxySettings() }),
  networkSettings: () => queryOptions({ queryKey: networkSettingsKey, queryFn: () => backend.getNetworkSettings() }),
  storageSettings: () => queryOptions({ queryKey: storageSettingsKey, queryFn: () => backend.getStorageSettings() }),
  updateSettings: () => queryOptions({ queryKey: updateSettingsKey, queryFn: () => backend.getUpdateSettings() }),
  appVersion: () => queryOptions({ queryKey: appVersionKey, queryFn: () => backend.appVersion() }),
  protoSchemas: () => queryOptions({ queryKey: protoSchemasKey, queryFn: () => backend.listProtoSchemas() }),
  protoTypes: () => queryOptions({ queryKey: protoTypesKey, queryFn: () => backend.listProtoMessageTypes() }),
  protoMappings: () => queryOptions({ queryKey: protoMappingsKey, queryFn: () => backend.listProtoMappings() }),
  mockSources: () => queryOptions({ queryKey: mockSourcesKey, queryFn: () => backend.listMockSources() }),
  openApiSpecs: () => queryOptions({ queryKey: openApiSpecsKey, queryFn: () => backend.listOpenApiSpecs() }),
  interceptApps: () => queryOptions({ queryKey: interceptAppsKey, queryFn: () => backend.listInterceptApps() }),
  intercepts: () => queryOptions({ queryKey: interceptsKey, queryFn: () => backend.listIntercepts() }),
  recordingSettings: () => queryOptions({ queryKey: recordingSettingsKey, queryFn: () => backend.getRecordingSettings() }),
  autosaveSettings: () => queryOptions({ queryKey: autosaveSettingsKey, queryFn: () => backend.getAutosaveSettings() }),
};
