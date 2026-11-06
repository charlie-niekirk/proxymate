//! Every engine write that changes cached state (R4), with what it does to
//! the cache: a setter puts its answer under its key, the rest invalidate
//! what they made stale. Components run them with `useMutation`; non-React
//! code (`lib/actions.ts`, the command palette) with `runMutation`, so both
//! paths keep the cache the same way.

import { MutationObserver, mutationOptions, type DataTag, type QueryKey } from "@tanstack/react-query";

import type {
  AutoSaveSettings_Deserialize,
  NetworkSettings_Deserialize,
  ProtoMapping_Deserialize,
  ProxySettings_Serialize,
  RecordingSettings_Deserialize,
  RuleSettings,
  ScriptSettings_Deserialize,
  SslSettings,
  StorageSettings_Serialize,
  UpdateSettings,
  WsSend,
} from "@/bindings";
import { backend } from "@/lib/backend";
import {
  androidKey,
  autosaveSettingsKey,
  caInfoKey,
  decodedRootKey,
  deviceAccessKey,
  mcpAuditKey,
  mcpTokenKey,
  mockSourcesKey,
  networkSettingsKey,
  openApiSpecsKey,
  operationRootKey,
  protoMappingsKey,
  protoSchemasKey,
  protoTypesKey,
  proxySettingsKey,
  proxyStatusKey,
  queryClient,
  recordingSettingsKey,
  ruleSettingsKey,
  rulesKey,
  scriptSettingsKey,
  simulatorKey,
  sslSettingsKey,
  storageSettingsKey,
  streamKey,
  systemProxyKey,
  terminalEnvKey,
  trustStatusKey,
  updateSettingsKey,
  wsPagesKey,
} from "@/lib/query";

const invalidate = (keys: readonly QueryKey[]) =>
  Promise.all(keys.map((queryKey) => queryClient.invalidateQueries({ queryKey }))).then(() => undefined);

/**
 * A write whose answer is the new value under `key`; `also` lists keys it
 * makes stale besides (refetched whether it succeeded or not).
 */
function setter<TVars, TData>(
  key: DataTag<readonly unknown[], TData>,
  write: (vars: TVars) => Promise<TData>,
  also: readonly QueryKey[] = [],
) {
  return mutationOptions<TData, Error, TVars>({
    mutationKey: ["set", ...key],
    mutationFn: write,
    onSuccess: (data) => {
      queryClient.setQueryData(key, data);
    },
    onSettled: () => invalidate(also),
  });
}

/** A write that answers nothing useful; `stale` lists the keys it changes. */
function changer<TVars, TData = void>(name: string, write: (vars: TVars) => Promise<TData>, stale: readonly QueryKey[]) {
  return mutationOptions<TData, Error, TVars>({
    mutationKey: [name],
    mutationFn: write,
    onSettled: () => invalidate(stale),
  });
}

/** Changing the proxy's port or interfaces moves what the toolbar and setup screens show. */
const LISTENING = [proxyStatusKey, deviceAccessKey, terminalEnvKey] as const;
/** Schemas and mappings changed: everything decoded with them is stale. */
const PROTOS = [protoSchemasKey, protoTypesKey, protoMappingsKey, decodedRootKey] as const;

export const mutations = {
  // ---- capture and proxy ----
  setRecording: setter(proxyStatusKey, (on: boolean) => backend.setRecording(on)),
  setSystemProxy: setter(systemProxyKey, (on: boolean) => backend.setSystemProxy(on)),
  setProxySettings: setter(proxySettingsKey, (s: ProxySettings_Serialize) => backend.setProxySettings(s), LISTENING),
  setNetworkSettings: setter(networkSettingsKey, (s: NetworkSettings_Deserialize) => backend.setNetworkSettings(s), [
    proxyStatusKey,
  ]),
  setStorageSettings: setter(storageSettingsKey, (s: StorageSettings_Serialize) => backend.setStorageSettings(s)),
  setRecordingSettings: setter(recordingSettingsKey, (s: RecordingSettings_Deserialize) => backend.setRecordingSettings(s)),
  neverRecordHost: setter(recordingSettingsKey, (host: string) => backend.neverRecordHost(host)),
  setAutosaveSettings: setter(autosaveSettingsKey, (s: AutoSaveSettings_Deserialize) => backend.setAutosaveSettings(s)),
  setUpdateSettings: setter(updateSettingsKey, (s: UpdateSettings) => backend.setUpdateSettings(s)),

  // ---- HTTPS ----
  setSslSettings: setter(sslSettingsKey, (s: SslSettings) => backend.setSslSettings(s)),
  addSslBypass: setter(sslSettingsKey, (host: string) => backend.addSslBypass(host)),
  setInsecureUpstream: setter(sslSettingsKey, ({ host, allow }: { host: string; allow: boolean }) =>
    backend.setInsecureUpstream(host, allow),
  ),
  setAppSslExcluded: setter(sslSettingsKey, ({ app, excluded }: { app: string; excluded: boolean }) =>
    backend.setAppSslExcluded(app, excluded),
  ),
  // A failure to untrust the old CA still leaves the new one in place: refetch both either way.
  regenerateCa: setter(caInfoKey, (removeOldTrust: boolean) => backend.regenerateCa(removeOldTrust), [
    caInfoKey,
    trustStatusKey,
  ]),
  installCaTrust: setter(trustStatusKey, () => backend.installCaTrust()),
  uninstallCaTrust: setter(trustStatusKey, () => backend.uninstallCaTrust()),

  // ---- rules and scripts ----
  reorderRules: setter(rulesKey, (ids: string[]) => backend.reorderRules(ids)),
  setRuleSettings: setter(ruleSettingsKey, (s: RuleSettings) => backend.setRuleSettings(s)),
  setScriptSettings: setter(scriptSettingsKey, (s: ScriptSettings_Deserialize) => backend.setScriptSettings(s)),

  // ---- devices ----
  setLanAccess: setter(deviceAccessKey, (on: boolean) => backend.setLanAccess(on), [proxyStatusKey]),
  setLanInterface: setter(deviceAccessKey, (name: string) => backend.setLanInterface(name), [proxyStatusKey]),
  setAndroidProxy: setter(androidKey, ({ serial, on }: { serial: string; on: boolean }) =>
    backend.setAndroidProxy(serial, on),
  ),
  installSimulatorCert: changer("install-simulator-cert", (udids: string[]) => backend.installSimulatorCert(udids), [
    simulatorKey,
  ]),

  // ---- WebSocket (M19): the message list refetches ahead of the next delta ----
  wsSend: mutationOptions<void, Error, { store: number; id: number; message: WsSend }>({
    mutationKey: ["ws-send"],
    mutationFn: ({ id, message }) => backend.wsSend(id, message),
    onSettled: (_data, _error, { store, id }) => invalidate([streamKey(store, id), wsPagesKey(store, id)]),
  }),
  wsClose: mutationOptions<void, Error, { store: number; id: number }>({
    mutationKey: ["ws-close"],
    mutationFn: ({ id }) => backend.wsClose(id),
    onSettled: (_data, _error, { store, id }) => invalidate([streamKey(store, id), wsPagesKey(store, id)]),
  }),

  // ---- MCP ----
  regenerateMcpToken: setter(mcpTokenKey, () => backend.regenerateMcpToken()),
  clearMcpAudit: changer("clear-mcp-audit", () => backend.clearMcpAudit(), [mcpAuditKey]),

  // ---- protobuf (M16) and mocking (M20) ----
  setProtoMappings: changer(
    "set-proto-mappings",
    (mappings: ProtoMapping_Deserialize[]) => backend.setProtoMappings(mappings),
    PROTOS,
  ),
  importProtoSchema: changer("import-proto-schema", (paths: string[] | null) => backend.importProtoSchema(paths), PROTOS),
  removeProtoSchema: changer("remove-proto-schema", (id: number) => backend.removeProtoSchema(id), PROTOS),
  createMockSource: changer(
    "create-mock-source",
    ({ store, name }: { store: number; name: string | null }) => backend.createMockSource(store, name),
    [mockSourcesKey],
  ),
  createMockSourceFromFile: changer(
    "create-mock-source-from-file",
    (path: string | null) => backend.createMockSourceFromFile(path),
    [mockSourcesKey],
  ),
  removeMockSource: changer("remove-mock-source", (id: string) => backend.removeMockSource(id), [mockSourcesKey]),
  importOpenApiSpec: changer("import-openapi-spec", (path: string | null) => backend.importOpenApiSpec(path), [
    openApiSpecsKey,
    operationRootKey,
  ]),
  removeOpenApiSpec: changer("remove-openapi-spec", (id: number) => backend.removeOpenApiSpec(id), [
    openApiSpecsKey,
    operationRootKey,
  ]),
};

/**
 * Runs a mutation outside React (actions, commands): the same cache effects
 * as `useMutation`, and it shows in the mutation cache while it runs.
 */
export function runMutation<TData, TVars>(
  options: ReturnType<typeof mutationOptions<TData, Error, TVars>>,
  vars: TVars,
): Promise<TData> {
  return new MutationObserver<TData, Error, TVars>(queryClient, options).mutate(vars);
}
