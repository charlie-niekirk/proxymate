import { toast } from "sonner";

import type { FlowSelection, PresetKind, RequestSpec_Deserialize, SaveProgress_Serialize, ScriptKind } from "@/bindings";
import { backend } from "@/lib/backend";
import {
  androidKey,
  caInfoKey,
  deviceAccessKey,
  documentsKey,
  proxyStatusKey,
  queryClient,
  rulesKey,
  scriptsKey,
  sslSettingsKey,
  systemProxyKey,
  trustStatusKey,
} from "@/lib/query";
import { GLOBAL_THROTTLE_RULE_ID, normalizeRule, withThrottle, type Rule, type ThrottlePreset } from "@/lib/rules";
import { KIND_LABELS, newScript, normalizeScript, type Script } from "@/lib/scripts";
import { LIVE_STORE, useUi } from "@/store/ui";

/** Clears every live flow. The view's structural delta then empties the table. */
export async function clearFlows() {
  if (useUi.getState().store === LIVE_STORE) useUi.getState().select(null);
  await backend.clearFlows();
  queryClient.removeQueries({ queryKey: ["body", LIVE_STORE] });
  queryClient.removeQueries({ queryKey: ["flow", LIVE_STORE] });
}

/** An error's message, for display. */
export const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** A failed action, as a toast: what was being done, then why. */
export function reportError(what: string, err: unknown) {
  toast.error(what, { description: errorText(err) });
}

/** Pins or unpins live flows: pinned flows are kept when old flows are evicted. */
export async function pinFlows(ids: number[], pinned: boolean) {
  await backend.pinFlows(ids, pinned);
  for (const id of ids) void queryClient.invalidateQueries({ queryKey: ["flow", LIVE_STORE, id] });
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

/**
 * Replaces the root CA; clients must trust the new one. With
 * `removeOldTrust`, the old CA is also removed from the keychain.
 */
export async function regenerateCa(removeOldTrust = false) {
  try {
    queryClient.setQueryData(caInfoKey, await backend.regenerateCa(removeOldTrust));
  } finally {
    // A failure to untrust the old CA still leaves the new one in place.
    await queryClient.invalidateQueries({ queryKey: caInfoKey });
    await queryClient.invalidateQueries({ queryKey: trustStatusKey });
  }
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
 * "Map Local…", "Breakpoint…": a rule pre-filled from a flow of `store`
 * (in Rust), opened unsaved in the Rules Editor. "Rewrite…" opens the
 * Rewrite dialog instead.
 */
export async function openPreset(store: number, flow: number, preset: PresetKind) {
  if (preset === "rewrite") return openRewrite(store, flow);
  const draft = normalizeRule(await backend.draftRule(store, flow, preset));
  useUi.getState().select(flow);
  useUi.getState().openRulesEditor({ draft });
}

/**
 * Opens the Rewrite dialog on a flow of `store`, or reopens a saved `rule`
 * visually on a flow it matches: the selected one if it does, else the
 * newest.
 */
export async function openRewrite(store: number, flow: number | null, rule: Rule | null = null) {
  if (flow === null && rule) {
    const matching = await backend.flowsMatchingRule(store, rule, 500);
    const selected = useUi.getState().selectedId;
    flow = selected !== null && matching.includes(selected) ? selected : (matching[0] ?? null);
    if (flow === null) {
      reportError("No flow to edit this rule on", "no recorded flow in this tab matches it: capture one first");
      return;
    }
  }
  if (flow === null) return;
  useUi.getState().openRewrite({ store, flow, rule });
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

export async function copyFlowUrl(store: number, flow: number) {
  const detail = await backend.getFlowDetail(store, flow);
  const summary = detail ? null : await backend.getFlow(store, flow);
  const url = detail?.request.url ?? (summary ? `${summary.scheme}://${summary.host}${summary.path}${summary.query ? `?${summary.query}` : ""}` : "");
  if (url) await navigator.clipboard.writeText(url);
}

/** A shell command (built in Rust, bodies included) that sends the request again. */
export async function copyFlowCurl(store: number, flow: number) {
  await navigator.clipboard.writeText(await backend.getCurl(store, flow));
}

// ---- Replay and compose ----

/** Shows a flow the app just sent: the live tab, with it selected. */
function showSent(id: number) {
  useUi.getState().setStore(LIVE_STORE);
  useUi.getState().select(id);
}

/** Sends a recorded request again, as it was sent; the new flow is selected. */
export async function replayFlow(store: number, flow: number) {
  showSent(await backend.replayFlow(store, flow, true));
}

/** Opens Compose pre-filled from a recorded request (body decoded). */
export async function editAndReplay(store: number, flow: number) {
  useUi.getState().openCompose(await backend.getRequestSpec(store, flow));
}

/** Sends a composed request; the new flow is selected. */
export async function sendComposed(spec: RequestSpec_Deserialize, applyRules: boolean) {
  showSent(await backend.sendRequest(spec, applyRules));
}

// ---- Documents: sessions and HAR ----

export async function refreshDocuments() {
  const documents = await backend.listDocuments();
  useUi.getState().setDocuments(documents);
  queryClient.setQueryData(documentsKey, documents);
}

/** Opens a session or HAR (asking for the file without `path`) in a new tab. */
export async function openDocument(path: string | null = null, har = false) {
  const doc = await backend.openDocument(path, har);
  if (!doc) return;
  await refreshDocuments();
  useUi.getState().setStore(doc.store);
}

export async function closeDocument(store: number) {
  await backend.closeDocument(store);
  if (useUi.getState().store === store) useUi.getState().setStore(LIVE_STORE);
  queryClient.removeQueries({ queryKey: ["body", store] });
  queryClient.removeQueries({ queryKey: ["flow", store] });
  await refreshDocuments();
}

/** A file name for what the active tab holds, with `extension`. */
function suggestedName(extension: string) {
  const { store, documents } = useUi.getState();
  const doc = documents.find((d) => d.store === store);
  if (doc) return `${doc.name.replace(/\.(har|proxymate)$/i, "")}.${extension}`;
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `Proxymate ${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}.${pad(d.getMinutes())}.${extension}`;
}

/** Saves the active tab's flows (all of them, by default) as a `.proxymate` session. */
export async function saveSession(selection: FlowSelection = { kind: "all" }, path: string | null = null) {
  const store = useUi.getState().store;
  await runSave("Saving session", (onProgress) =>
    backend.saveSession(store, selection, path, suggestedName("proxymate"), onProgress),
  );
}

/** Exports flows of the active tab as HAR: what the table shows, by default. */
export async function exportHar(selection?: FlowSelection, path: string | null = null) {
  const { store, view } = useUi.getState();
  const which: FlowSelection = selection ?? (view !== null ? { kind: "view", view } : { kind: "all" });
  await runSave("Exporting HAR", (onProgress) => backend.exportHar(store, which, path, suggestedName("har"), onProgress));
}

/** Runs a background save, mirroring its progress in the footer. */
async function runSave(label: string, start: (onProgress: (p: SaveProgress_Serialize) => void) => Promise<string | null>) {
  const { setSave } = useUi.getState();
  const onProgress = (progress: SaveProgress_Serialize) => {
    setSave({ label, progress });
    if (progress.finished && !progress.error) {
      // The "saved" note stays a moment, unless another save replaced it.
      setTimeout(() => {
        if (useUi.getState().save?.progress === progress) setSave(null);
      }, 4000);
    }
  };
  try {
    await start(onProgress);
  } catch (err) {
    setSave({
      label,
      progress: { done: 0, total: 0, finished: true, error: err instanceof Error ? err.message : String(err), path: "" },
    });
  }
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

// ---- Scripts (M14) ----

/** Refetches the scripts list after a change (the engine also pushes it). */
export async function refreshScripts() {
  await queryClient.invalidateQueries({ queryKey: scriptsKey });
}

/** Saves a script (new if it has no id); resolves to the saved script. */
export async function saveScript(script: Script): Promise<Script> {
  const saved = script.id ? await backend.updateScript(script) : await backend.createScript(script);
  await refreshScripts();
  return normalizeScript(saved);
}

export async function deleteScript(id: string) {
  await backend.deleteScript(id);
  await refreshScripts();
}

export async function setScriptEnabled(id: string, enabled: boolean) {
  await backend.setScriptEnabled(id, enabled);
  await refreshScripts();
}

export async function deleteAgentScripts(): Promise<number> {
  const n = await backend.deleteAgentScripts();
  await refreshScripts();
  return n;
}

/**
 * The flow menu's "Script ›": a new script of `kind` from its template,
 * named after the flow; a view script also matches the flow's host and path.
 */
export async function newScriptForFlow(store: number, flow: number, kind: ScriptKind) {
  const summary = await backend.getFlow(store, flow);
  const script = newScript(kind);
  if (summary) {
    script.name = `${KIND_LABELS[kind]} · ${summary.host}${summary.path}`;
    if (script.view) {
      script.view.matcher = {
        ...script.view.matcher,
        host: { kind: "exact", value: summary.host, case_insensitive: false },
        path: { kind: "exact", value: summary.path, case_insensitive: false },
      };
    }
  }
  useUi.getState().openScripts({ draft: script });
}

/**
 * A rule that runs `script` on requests like the selected flow (its
 * method, host and path), opened unsaved in the Rules Editor.
 */
export async function ruleForScript(store: number, flow: number, script: Script) {
  const draft = normalizeRule(await backend.draftRule(store, flow, script.kind === "mock" ? "map_local" : "rewrite"));
  const target = draft.name.replace(/^[^·]*·\s*/, "");
  useUi.getState().openRulesEditor({
    draft: {
      ...draft,
      name: `${script.name} · ${target}`,
      origin: { kind: "user" },
      actions: [{ type: "Script", script_id: script.id }],
    },
  });
}

/** Saves a script as a `.js` file (the app asks where). */
export async function exportScript(id: string) {
  const path = await backend.exportScript(id, null);
  if (path) toast.success("Script exported", { description: path });
}

/** Imports a `.js` file as a new script; resolves to it (null if cancelled). */
export async function importScript(): Promise<Script | null> {
  const script = await backend.importScript(null);
  await refreshScripts();
  return script ? normalizeScript(script) : null;
}

/** Copies `proxymate.d.ts`, for an editor or an agent. */
export async function copyScriptTypes() {
  await navigator.clipboard.writeText(await backend.getScriptTypeDefinitions());
  toast.success("Copied proxymate.d.ts");
}
