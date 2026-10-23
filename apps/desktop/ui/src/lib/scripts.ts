import type {
  ScriptKind,
  ScriptView_Serialize,
  ScriptViewKind,
  Script_Serialize,
  ViewConfig_Serialize,
  ViewResult_Serialize,
} from "@/bindings";
import type { ViewerId } from "@/components/BodyViewer/registry";
import { emptyMatcher } from "@/lib/rules";

// The templates are plain .js files so the engine's tests run them too
// (pm-core `scripts::tests::templates_run`).
import decodeBase64Json from "./script-templates/decode-base64-json.js?raw";
import decodeJwt from "./script-templates/decode-jwt.js?raw";
import addHeader from "./script-templates/add-header.js?raw";
import modifyJson from "./script-templates/modify-json.js?raw";
import mockJson from "./script-templates/mock-json.js?raw";

/**
 * Scripts (M14) on the UI side: templates, editor completions, and how the
 * view scripts' output joins the body viewer's dropdown. The script API
 * itself is `crates/pm-core/src/scripts/proxymate.d.ts`.
 */

export type Script = Script_Serialize;
export type ScriptView = ScriptView_Serialize;
export type ViewResult = ViewResult_Serialize;

export const KIND_LABELS: Record<ScriptKind, string> = {
  view: "View",
  rewrite: "Rewrite",
  mock: "Mock",
};

/** List order: views first, then rewrites, then mocks. */
export const KIND_ORDER: ScriptKind[] = ["view", "rewrite", "mock"];

export const KIND_HELP: Record<ScriptKind, string> = {
  view: "Adds views of matching bodies to the inspector's viewer menu (onView).",
  rewrite: "Changes requests (onRequest) and responses (onResponse) when a rule runs it.",
  mock: "Answers requests without the upstream (onMock) when a rule runs it.",
};

export function emptyViewConfig(): ViewConfig_Serialize {
  return { matcher: emptyMatcher(), parts: [], kinds: [] };
}

export interface Template {
  id: string;
  name: string;
  kind: ScriptKind;
  description: string;
  source: string;
}

export const TEMPLATES: Template[] = [
  {
    id: "decode-base64-json",
    name: "Decode base64 JSON field",
    kind: "view",
    description: "Finds string fields that hold base64-encoded JSON and shows them decoded.",
    source: decodeBase64Json,
  },
  {
    id: "decode-jwt",
    name: "Decode JWT",
    kind: "view",
    description: "Decodes JSON Web Tokens in the Authorization header and in body fields.",
    source: decodeJwt,
  },
  {
    id: "add-header",
    name: "Add a header",
    kind: "rewrite",
    description: "Adds a request header (and shows how to change the response).",
    source: addHeader,
  },
  {
    id: "modify-json",
    name: "Modify a JSON field",
    kind: "rewrite",
    description: "Changes a field of a JSON response.",
    source: modifyJson,
  },
  {
    id: "mock-json",
    name: "Mock JSON",
    kind: "mock",
    description: "Answers with JSON built from the request.",
    source: mockJson,
  },
];

/** A blank script of `kind`, from its first template. */
export function newScript(kind: ScriptKind, template?: Template): Script {
  const t = template ?? TEMPLATES.find((x) => x.kind === kind)!;
  return {
    id: "",
    name: template ? template.name : `New ${KIND_LABELS[kind].toLowerCase()} script`,
    kind: t.kind,
    enabled: true,
    source: t.source,
    origin: { kind: "user" },
    view: t.kind === "view" ? emptyViewConfig() : null,
    updated_at_ms: 0,
  };
}

/** Fills in fields a deserialize-shaped script may omit. */
export function normalizeScript(script: Script): Script {
  return {
    ...script,
    // Rust takes a number; specta types an f64 as possibly null.
    updated_at_ms: script.updated_at_ms ?? 0,
    view:
      script.kind === "view"
        ? { ...emptyViewConfig(), ...script.view, matcher: { ...emptyMatcher(), ...script.view?.matcher } }
        : null,
  };
}

// ---- Editor completions ----

export interface Completion {
  label: string;
  detail: string;
}

/**
 * The globals the editor completes, with their members. Hand-kept: a vitest
 * checks it against `proxymate.d.ts` and `scripts/api_surface.txt`.
 */
export const GLOBAL_COMPLETIONS: Record<string, { detail: string; members: Completion[] }> = {
  Proxymate: {
    detail: "Proxymate helpers",
    members: [
      { label: "base64Decode", detail: "(text) → Uint8Array; standard or URL-safe" },
      { label: "base64DecodeText", detail: "(text) → string" },
      { label: "base64DecodeJson", detail: "(text) → any; big integers as BigInt" },
      { label: "base64Encode", detail: "(data, { urlSafe }?) → string" },
      { label: "decompress", detail: '(bytes, "gzip" | "deflate" | "br" | "zstd") → Uint8Array' },
      { label: "parseJson", detail: "(text) → any; big integers as BigInt" },
      { label: "stringifyJson", detail: "(value, space?) → string" },
    ],
  },
  crypto: {
    detail: "hashes, HMAC and UUIDs",
    members: [
      { label: "sha1", detail: '(data, "hex" | "base64" | "bytes"?) → string' },
      { label: "sha256", detail: '(data, "hex" | "base64" | "bytes"?) → string' },
      { label: "sha512", detail: '(data, "hex" | "base64" | "bytes"?) → string' },
      { label: "hmac", detail: '("sha256", key, data, encoding?) → string' },
      { label: "randomUUID", detail: "() → string" },
    ],
  },
  console: {
    detail: "the script console",
    members: ["debug", "log", "info", "warn", "error", "trace"].map((label) => ({ label, detail: "(...args)" })),
  },
  performance: { detail: "timing", members: [{ label: "now", detail: "() → milliseconds" }] },
  TextEncoder: { detail: "class: string → UTF-8 bytes", members: [] },
  TextDecoder: { detail: "class: UTF-8 bytes → string", members: [] },
  URL: { detail: "class: WHATWG URL", members: [] },
  URLSearchParams: { detail: "class: query strings", members: [] },
  DOMException: { detail: "class", members: [] },
  atob: { detail: "(base64) → binary string", members: [] },
  btoa: { detail: "(binary string) → base64", members: [] },
};

/** Entry points, completed as snippets at the top level. */
export const ENTRY_POINTS: Record<ScriptKind, { label: string; snippet: string }[]> = {
  rewrite: [
    { label: "onRequest", snippet: "export function onRequest(context, url, request) {\n  ${}\n  return request;\n}" },
    {
      label: "onResponse",
      snippet: "export function onResponse(context, url, request, response) {\n  ${}\n  return response;\n}",
    },
  ],
  mock: [{ label: "onMock", snippet: "export function onMock(context, url, request) {\n  ${}\n  return null;\n}" }],
  view: [{ label: "onView", snippet: "export function onView(context, flow) {\n  ${}\n  return [];\n}" }],
};

/** Fields of the messages and `context`, completed after a `.`. */
export const MESSAGE_FIELDS: Completion[] = [
  { label: "method", detail: "request: upper-case method" },
  { label: "url", detail: "request: absolute URL" },
  { label: "scheme", detail: "request" },
  { label: "host", detail: "request" },
  { label: "port", detail: "request" },
  { label: "path", detail: "request: without the query" },
  { label: "query", detail: "request: decoded parameters" },
  { label: "status", detail: "response" },
  { label: "headers", detail: "lower-case names; repeated → string[]" },
  { label: "body", detail: "parsed JSON, form fields, text, or null" },
  { label: "text", detail: "the body as UTF-8" },
  { label: "rawBody", detail: "the body's bytes" },
  { label: "bodyTooLarge", detail: "over 16 MB: no body" },
  { label: "shared", detail: "context: kept between runs" },
  { label: "flowId", detail: "context" },
  { label: "part", detail: "context (views): request or response" },
  { label: "request", detail: "flow (views)" },
  { label: "response", detail: "flow (views)" },
];

// ---- View scripts in the body viewer ----

/** One script view, as an entry of the viewer dropdown. */
export interface ScriptViewOption {
  /** `script:<script id>:<index>`. */
  value: string;
  label: string;
  scriptId: string;
  scriptName: string;
  view: ScriptView;
}

export const SCRIPT_VALUE_PREFIX = "script:";

export function isScriptValue(value: string): boolean {
  return value.startsWith(SCRIPT_VALUE_PREFIX);
}

/** Every view the scripts produced, in script order. */
export function scriptViewOptions(results: readonly ViewResult[]): ScriptViewOption[] {
  return results.flatMap((r) =>
    r.views.map((view, i) => ({
      value: `${SCRIPT_VALUE_PREFIX}${r.script_id}:${i}`,
      label: r.views.length > 1 || view.title !== r.script_name ? `${view.title}` : r.script_name,
      scriptId: r.script_id,
      scriptName: r.script_name,
      view,
    })),
  );
}

/** Scripts that failed on this body. */
export function scriptErrors(results: readonly ViewResult[]): { scriptId: string; scriptName: string; error: string }[] {
  return results
    .filter((r) => r.error !== null)
    .map((r) => ({ scriptId: r.script_id, scriptName: r.script_name, error: r.error! }));
}

/**
 * What the dropdown shows: the script view the user picked (when this body
 * has it), else the first script view unless the user chose to stay with
 * the built-in viewers, else the built-in viewer.
 */
export function chooseViewer(
  builtin: ViewerId,
  options: readonly ScriptViewOption[],
  picked: string | null,
  optedOut: boolean,
): string {
  if (picked && options.some((o) => o.value === picked)) return picked;
  if (!optedOut && options.length > 0) return options[0]!.value;
  return builtin;
}

export function viewerFor(kind: ScriptViewKind): ViewerId {
  switch (kind) {
    case "json":
      return "json";
    case "xml":
      return "xml";
    case "html":
      return "html";
    case "hex":
      return "hex";
    case "image":
      return "image";
    case "text":
      return "text";
  }
}

/** A script view's content as bytes, for the built-in viewers. */
export function scriptViewBytes(view: ScriptView): Uint8Array {
  if (view.base64 !== null) {
    const binary = atob(view.base64);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  }
  return new TextEncoder().encode(view.text ?? "");
}

/** The MIME type a script view is shown as. */
export function scriptViewMime(view: ScriptView): string {
  switch (view.kind) {
    case "json":
      return "application/json";
    case "xml":
      return "application/xml";
    case "html":
      return "text/html";
    case "image":
      return view.mime ?? "image/png";
    case "hex":
      return "application/octet-stream";
    case "text":
      return "text/plain";
  }
}

/** A safe file name for an exported script. */
export function scriptFileName(name: string): string {
  return `${name.replace(/[^\p{L}\p{N} ._-]/gu, "-").trim() || "script"}.js`;
}
