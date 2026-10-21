import type { PresetKind } from "@/bindings";
import { backend } from "@/lib/backend";
import {
  androidKey,
  caInfoKey,
  deviceAccessKey,
  proxyStatusKey,
  queryClient,
  rulesKey,
  sslSettingsKey,
  systemProxyKey,
  trustStatusKey,
} from "@/lib/query";
import { GLOBAL_THROTTLE_RULE_ID, curlCommand, normalizeRule, withThrottle, type Rule, type ThrottlePreset } from "@/lib/rules";
import { useUi } from "@/store/ui";

/** Clears every flow. The view's structural delta then empties the table. */
export async function clearFlows() {
  useUi.getState().select(null);
  await backend.clearFlows();
  queryClient.removeQueries({ queryKey: ["body"] });
  queryClient.removeQueries({ queryKey: ["flow"] });
}

/** Pauses or resumes recording; traffic is forwarded either way. */
export async function setRecording(on: boolean) {
  queryClient.setQueryData(proxyStatusKey, await backend.setRecording(on));
}

/** Stops decrypting `host`; its next CONNECTs are tunnelled. */
export async function addSslBypass(host: string) {
  queryClient.setQueryData(sslSettingsKey, await backend.addSslBypass(host));
}

/** Accepts (or stops accepting) `host`'s upstream certificate unverified. */
export async function setInsecureUpstream(host: string, allow: boolean) {
  queryClient.setQueryData(sslSettingsKey, await backend.setInsecureUpstream(host, allow));
}

/** Replaces the root CA; clients must trust the new one. */
export async function regenerateCa() {
  queryClient.setQueryData(caInfoKey, await backend.regenerateCa());
  await queryClient.invalidateQueries({ queryKey: trustStatusKey });
}

/** Installs and trusts the CA (macOS prompts), then re-checks trust. */
export async function installCaTrust() {
  queryClient.setQueryData(trustStatusKey, await backend.installCaTrust());
}

export async function uninstallCaTrust() {
  queryClient.setQueryData(trustStatusKey, await backend.uninstallCaTrust());
}

/** Turns the system proxy on or off; the status says if it failed. */
export async function setSystemProxy(on: boolean) {
  queryClient.setQueryData(systemProxyKey, await backend.setSystemProxy(on));
}

/** Stops (or resumes) decrypting everything `app` sends. */
export async function setAppSslExcluded(app: string, excluded: boolean) {
  queryClient.setQueryData(sslSettingsKey, await backend.setAppSslExcluded(app, excluded));
}

/** Copies `export HTTPS_PROXY=… NODE_EXTRA_CA_CERTS=…` for a terminal. */
export async function copyTerminalEnv() {
  await navigator.clipboard.writeText(await backend.getTerminalEnv());
}

// ---- Rules ----

/** Refetches the rules list after a change. */
export async function refreshRules() {
  await queryClient.invalidateQueries({ queryKey: rulesKey });
}

/**
 * "Map Local…", "Breakpoint…", "Rewrite…": a rule pre-filled from the flow
 * (in Rust), opened unsaved in the Rules Editor.
 */
export async function openPreset(flow: number, preset: PresetKind) {
  const draft = normalizeRule(await backend.draftRule(flow, preset));
  useUi.getState().select(flow);
  useUi.getState().openRulesEditor({ draft });
}

/** Saves a rule (new if it has no id); resolves to the saved rule. */
export async function saveRule(rule: Rule): Promise<Rule> {
  const saved = rule.id ? await backend.updateRule(rule) : await backend.createRule(rule);
  await refreshRules();
  return normalizeRule(saved);
}

export async function deleteRule(id: string) {
  await backend.deleteRule(id);
  await refreshRules();
}

export async function setRuleEnabled(id: string, enabled: boolean) {
  await backend.setRuleEnabled(id, enabled);
  await refreshRules();
}

export async function reorderRules(ids: string[]) {
  queryClient.setQueryData(rulesKey, await backend.reorderRules(ids));
}

export async function deleteAgentRules(): Promise<number> {
  const n = await backend.deleteAgentRules();
  await refreshRules();
  return n;
}

/** Switches the toolbar's network conditions (the built-in throttle rule). */
export async function setThrottle(preset: ThrottlePreset | null) {
  const rules = await backend.listRules();
  const rule = rules.find((r) => r.id === GLOBAL_THROTTLE_RULE_ID);
  if (!rule) return;
  await backend.updateRule(withThrottle(normalizeRule(rule), preset));
  await refreshRules();
}

// ---- Flow menu ----

export async function copyFlowUrl(flow: number) {
  const detail = await backend.getFlowDetail(flow);
  const summary = detail ? null : await backend.getFlow(flow);
  const url = detail?.request.url ?? (summary ? `${summary.scheme}://${summary.host}${summary.path}${summary.query ? `?${summary.query}` : ""}` : "");
  if (url) await navigator.clipboard.writeText(url);
}

/** A client-side stub without bodies; the full exporter is M8. */
export async function copyFlowCurl(flow: number) {
  const detail = await backend.getFlowDetail(flow);
  if (detail) await navigator.clipboard.writeText(curlCommand(detail));
}

// ---- Device setup ----

/** Listens on every interface (or loopback only); the toolbar address follows. */
export async function setLanAccess(on: boolean) {
  queryClient.setQueryData(deviceAccessKey, await backend.setLanAccess(on));
  await queryClient.invalidateQueries({ queryKey: proxyStatusKey });
}

/** Picks the interface the toolbar and the QR codes use. */
export async function setLanInterface(name: string) {
  queryClient.setQueryData(deviceAccessKey, await backend.setLanInterface(name));
  await queryClient.invalidateQueries({ queryKey: proxyStatusKey });
}

/** Points an emulator's proxy at us, or clears it. */
export async function setAndroidProxy(serial: string, on: boolean) {
  queryClient.setQueryData(androidKey, await backend.setAndroidProxy(serial, on));
}
