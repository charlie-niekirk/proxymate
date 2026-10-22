import type { Header, RequestSpec_Deserialize } from "@/bindings";
import type { CodeLanguage } from "@/components/BodyViewer/code/languages";
import { cleanHeaders } from "@/lib/rules";

/** The Content-Type choices of the Compose body. */
export const CONTENT_TYPES = [
  { label: "None", value: null },
  { label: "JSON", value: "application/json" },
  { label: "Form", value: "application/x-www-form-urlencoded" },
  { label: "Text", value: "text/plain" },
  { label: "XML", value: "application/xml" },
  { label: "HTML", value: "text/html" },
  { label: "JavaScript", value: "application/javascript" },
  { label: "GraphQL", value: "application/graphql" },
] as const;

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] as const;

/** What Compose edits. A binary body (from Edit & Replay) stays base64. */
export interface ComposeDraft {
  method: string;
  url: string;
  headers: Header[];
  body: string;
  bodyBase64: boolean;
}

export function newDraft(): ComposeDraft {
  return { method: "GET", url: "https://", headers: [{ name: "accept", value: "*/*" }], body: "", bodyBase64: false };
}

export function draftOf(spec: { method: string; url: string; headers?: Header[]; body?: string | null; body_base64?: boolean }): ComposeDraft {
  return {
    method: spec.method,
    url: spec.url,
    headers: spec.headers ?? [],
    body: spec.body ?? "",
    bodyBase64: spec.body_base64 ?? false,
  };
}

export function specOf(draft: ComposeDraft): RequestSpec_Deserialize {
  return {
    method: draft.method.trim().toUpperCase(),
    url: draft.url.trim(),
    headers: cleanHeaders(draft.headers),
    body: draft.body === "" ? null : draft.body,
    body_base64: draft.bodyBase64,
  };
}

export function contentTypeOf(headers: Header[]): string | null {
  return headers.find((h) => h.name.trim().toLowerCase() === "content-type")?.value ?? null;
}

/** Sets (or, with null, removes) the Content-Type header, keeping its place. */
export function withContentType(headers: Header[], value: string | null): Header[] {
  const at = headers.findIndex((h) => h.name.trim().toLowerCase() === "content-type");
  if (value === null) return headers.filter((_, i) => i !== at);
  if (at < 0) return [...headers, { name: "content-type", value }];
  return headers.map((h, i) => (i === at ? { ...h, value } : h));
}

/** The editor language for a Content-Type. */
export function languageFor(contentType: string | null): CodeLanguage {
  const mime = (contentType ?? "").split(";")[0]!.trim().toLowerCase();
  if (mime === "application/json" || mime.endsWith("+json")) return "json";
  if (mime === "application/graphql") return "graphql";
  if (mime.endsWith("/xml") || mime.endsWith("+xml")) return "xml";
  if (mime === "text/html") return "html";
  if (mime.endsWith("javascript")) return "javascript";
  if (mime === "text/css") return "css";
  return "text";
}
