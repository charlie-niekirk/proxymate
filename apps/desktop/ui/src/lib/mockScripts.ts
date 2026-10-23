import type {
  BodyPart,
  ConsoleEntry_Serialize,
  ConsoleLevel,
  FlowDetail_Serialize,
  ScriptSettings,
  ScriptTestResult_Serialize,
  ScriptView_Serialize,
  ViewResult_Serialize,
} from "@/bindings";
import type { Backend } from "@/lib/backend";
import { TEMPLATES, normalizeScript, type Script } from "@/lib/scripts";

type ScriptMethods = Pick<
  Backend,
  | "listScripts"
  | "createScript"
  | "updateScript"
  | "setScriptEnabled"
  | "deleteScript"
  | "deleteAgentScripts"
  | "testScript"
  | "getFlowViews"
  | "getScriptConsole"
  | "clearScriptConsole"
  | "subscribeScriptConsole"
  | "subscribeScripts"
  | "getScriptSettings"
  | "setScriptSettings"
  | "getScriptTypeDefinitions"
  | "exportScript"
  | "importScript"
>;

/** What the mock knows about a flow: its detail and decoded bodies. */
export type MockFlowSource = (
  store: number,
  id: number,
) => Promise<{ detail: FlowDetail_Serialize; bodies: Record<BodyPart, Uint8Array> } | null>;

/**
 * Scripts in the browser mock. There is no QuickJS here: view scripts run
 * as browser modules against a small stand-in for the `Proxymate` global,
 * which is enough for the templates (the real API is tested in Rust).
 * Rewrite and mock scripts are only listed.
 */
export function createMockScripts(now: number, flowSource: MockFlowSource, usedBy: (id: string) => string[]): ScriptMethods {
  const decode = TEMPLATES.find((t) => t.id === "decode-base64-json")!;
  let scripts: Script[] = [
    { ...normalizeScript({ ...decodeScript(decode.source), updated_at_ms: now }), id: "mock-view-base64" },
    {
      ...normalizeScript({
        id: "mock-agent-mock",
        name: "Checkout stub",
        kind: "mock",
        enabled: true,
        source: TEMPLATES.find((t) => t.id === "mock-json")!.source,
        origin: { kind: "agent", client: "claude-code", session: null, created_at_ms: now },
        view: null,
        updated_at_ms: now,
      }),
    },
  ];
  let settings: ScriptSettings = { traffic_timeout_ms: 1000, view_timeout_ms: 3000 };
  let console: ConsoleEntry_Serialize[] = [];
  let seq = 0;
  let onScripts: ((s: Script[]) => void) | null = null;
  let onConsole: ((seq: number) => void) | null = null;
  const push = () => onScripts?.(sorted());
  const sorted = () => {
    const rank = { view: 0, rewrite: 1, mock: 2 } as const;
    return [...scripts].sort((a, b) => rank[a.kind] - rank[b.kind] || a.name.localeCompare(b.name));
  };
  const find = (id: string) => {
    const s = scripts.find((x) => x.id === id);
    if (!s) throw new Error(`no script with id ${id}`);
    return s;
  };
  const check = (script: Script) => {
    const entry = { view: "onView", rewrite: "onRequest", mock: "onMock" }[script.kind];
    const ok = script.kind === "rewrite" ? /\b(onRequest|onResponse)\b/.test(script.source) : script.source.includes(entry);
    if (!ok) throw new Error(`script ${JSON.stringify(script.name)}: a ${script.kind} script must export ${entry}`);
  };
  const log = (script: Script, level: ConsoleLevel, message: string, flowId: number | null) => {
    console = [
      ...console,
      { seq: ++seq, time_ms: Date.now(), script_id: script.id, script_name: script.name, level, message, flow_id: flowId },
    ].slice(-2000);
    onConsole?.(seq);
  };

  async function runView(script: Script, store: number, id: number, part: BodyPart): Promise<ViewResult_Serialize> {
    const started = performance.now();
    const flow = await flowSource(store, id);
    const result = (views: ScriptView_Serialize[], error: string | null): ViewResult_Serialize => ({
      script_id: script.id,
      script_name: script.name,
      views,
      error,
      duration_ms: performance.now() - started,
    });
    if (!flow) return result([], "no such flow");
    // A browser that won't import blob modules just shows no script views.
    const mod = await importSource(script.source).catch(() => null);
    if (!mod) return result([], null);
    try {
      const message = (p: BodyPart) => {
        const bytes = flow.bodies[p];
        const text = new TextDecoder().decode(bytes);
        let body: unknown = text;
        try {
          body = JSON.parse(text);
        } catch {
          // Not JSON: the text.
        }
        const headers = Object.fromEntries(
          (p === "request" ? flow.detail.request.headers : (flow.detail.response?.headers ?? [])).map((h) => [
            h.name.toLowerCase(),
            h.value,
          ]),
        );
        return { headers, body, text, rawBody: bytes };
      };
      const out: unknown = await (mod.onView as (c: unknown, f: unknown) => unknown)(
        { scriptId: script.id, scriptName: script.name, flowId: id, phase: "view", rule: null, part, shared: {} },
        { id, part, request: message("request"), response: flow.detail.response ? message("response") : null },
      );
      const list = out === null || out === undefined ? [] : Array.isArray(out) ? out : [out];
      return result(
        list.map((v: { title?: string; kind?: string; body?: unknown }, i) => {
          const json = typeof v.body === "object" && v.body !== null;
          return {
            title: v.title ?? `View ${i + 1}`,
            kind: (v.kind ?? (json ? "json" : "text")) as ScriptView_Serialize["kind"],
            text: json ? JSON.stringify(v.body, null, 2) : String(v.body ?? ""),
            base64: null,
            mime: null,
            truncated: false,
          };
        }),
        null,
      );
    } catch (e) {
      const message = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
      log(script, "error", message, id);
      return result([], message);
    }
  }

  return {
    async listScripts() {
      return sorted();
    },
    async createScript(script) {
      const created = normalizeScript({ ...(script as Script), id: `mock-${Date.now()}`, updated_at_ms: Date.now() });
      check(created);
      scripts = [...scripts, created];
      push();
      return created;
    },
    async updateScript(script) {
      find(script.id);
      const updated = normalizeScript({ ...(script as Script), updated_at_ms: Date.now() });
      check(updated);
      scripts = scripts.map((s) => (s.id === script.id ? updated : s));
      push();
      return updated;
    },
    async setScriptEnabled(id, enabled) {
      const updated = { ...find(id), enabled };
      scripts = scripts.map((s) => (s.id === id ? updated : s));
      push();
      return updated;
    },
    async deleteScript(id) {
      const script = find(id);
      const rules = usedBy(id);
      if (rules.length > 0) throw new Error(`script ${JSON.stringify(script.name)} is used by rule ${JSON.stringify(rules[0])}`);
      scripts = scripts.filter((s) => s.id !== id);
      push();
    },
    async deleteAgentScripts() {
      const before = scripts.length;
      scripts = scripts.filter((s) => s.origin.kind !== "agent" || usedBy(s.id).length > 0);
      push();
      return before - scripts.length;
    },
    async testScript(store, flow, spec): Promise<ScriptTestResult_Serialize> {
      const saved = spec.script_id ? scripts.find((s) => s.id === spec.script_id) : undefined;
      const kind = spec.kind ?? saved?.kind ?? "view";
      const script = { ...(saved ?? decodeScript("")), source: spec.source ?? saved?.source ?? "", kind };
      const base: ScriptTestResult_Serialize = {
        kind,
        error: null,
        request_before: null,
        request_body_before: null,
        request: null,
        request_body: null,
        response_before: null,
        response_body_before: null,
        response: null,
        response_body: null,
        views: [],
        console: [],
        duration_ms: 0,
        notes: [],
      };
      if (kind !== "view") return { ...base, notes: ["(mock) rewrite and mock scripts run only in the app"] };
      const r = await runView(script, store, flow, spec.part ?? "response");
      return { ...base, views: r.views, error: r.error, duration_ms: r.duration_ms };
    },
    async getFlowViews(store, id, part) {
      const results: ViewResult_Serialize[] = [];
      for (const script of sorted()) {
        if (script.kind !== "view" || !script.enabled) continue;
        const parts = script.view?.parts ?? [];
        if (parts.length > 0 && !parts.includes(part)) continue;
        const r = await runView(script, store, id, part);
        if (r.views.length > 0 || r.error) results.push(r);
      }
      return results;
    },
    async getScriptConsole(since, limit) {
      return console.filter((e) => since === null || e.seq > since).slice(-limit);
    },
    async clearScriptConsole() {
      console = [];
      onConsole?.(seq);
    },
    async subscribeScriptConsole(cb) {
      onConsole = cb;
      setTimeout(() => cb(seq), 0);
    },
    async subscribeScripts(cb) {
      onScripts = cb;
      setTimeout(push, 0);
    },
    async getScriptSettings() {
      return settings;
    },
    async setScriptSettings(next) {
      settings = next;
      return next;
    },
    async getScriptTypeDefinitions() {
      return "// proxymate.d.ts (the app serves the real file)\ndeclare function onView(context: any, flow: any): any;\n";
    },
    async exportScript(id, path) {
      return path ?? `/Users/you/Documents/${find(id).name}.js`;
    },
    async importScript() {
      return null;
    },
  };
}

function decodeScript(source: string): Script {
  return {
    id: "",
    name: "Decode base64 JSON field",
    kind: "view",
    enabled: true,
    source,
    origin: { kind: "user" },
    view: null,
    updated_at_ms: null,
  };
}

/** The `Proxymate` stand-in the templates need. */
function installGlobals() {
  const g = globalThis as Record<string, unknown>;
  if (g.Proxymate) return;
  const bytes = (text: string) => {
    const std = text.replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
    const binary = atob(std + "=".repeat((4 - (std.length % 4)) % 4));
    return Uint8Array.from(binary, (c) => c.charCodeAt(0));
  };
  g.Proxymate = {
    base64Decode: bytes,
    base64DecodeText: (t: string) => new TextDecoder().decode(bytes(t)),
    base64DecodeJson: (t: string) => JSON.parse(new TextDecoder().decode(bytes(t))) as unknown,
    parseJson: (t: string) => JSON.parse(t) as unknown,
    stringifyJson: (v: unknown, s?: number) => JSON.stringify(v, null, s),
  };
}

async function importSource(source: string): Promise<Record<string, unknown>> {
  installGlobals();
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  try {
    return (await import(/* @vite-ignore */ url)) as Record<string, unknown>;
  } finally {
    URL.revokeObjectURL(url);
  }
}
