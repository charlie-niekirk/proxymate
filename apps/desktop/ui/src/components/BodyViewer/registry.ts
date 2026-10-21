import type { ContentKind } from "@/bindings";

/**
 * The viewer registry: every way a body can be shown, and which ones fit
 * each content kind (best first). The pane picks the first, or the one the
 * user last chose for that kind; Text and Hex are always there as fallbacks.
 */

export type ViewerId =
  | "json"
  | "json-tree"
  | "graphql"
  | "xml"
  | "html"
  | "html-preview"
  | "form"
  | "multipart"
  | "javascript"
  | "css"
  | "text"
  | "image"
  | "video"
  | "audio"
  | "pdf"
  | "font"
  | "hex";

export interface ViewerDef {
  label: string;
  /** "bytes" viewers get the decoded body; "url" viewers load it from `bodyUrl`. */
  source: "bytes" | "url";
  /** Syntax-highlighted text, which falls back to plain text for huge bodies. */
  code: boolean;
}

export const VIEWERS: Record<ViewerId, ViewerDef> = {
  json: { label: "JSON", source: "bytes", code: true },
  "json-tree": { label: "JSON Tree", source: "bytes", code: false },
  graphql: { label: "GraphQL", source: "bytes", code: true },
  xml: { label: "XML", source: "bytes", code: true },
  html: { label: "HTML", source: "bytes", code: true },
  "html-preview": { label: "Preview", source: "bytes", code: false },
  form: { label: "Form", source: "bytes", code: false },
  multipart: { label: "Multipart", source: "bytes", code: false },
  javascript: { label: "JavaScript", source: "bytes", code: true },
  css: { label: "CSS", source: "bytes", code: true },
  text: { label: "Text", source: "bytes", code: true },
  image: { label: "Image", source: "url", code: false },
  video: { label: "Video", source: "url", code: false },
  audio: { label: "Audio", source: "url", code: false },
  pdf: { label: "PDF", source: "url", code: false },
  font: { label: "Font", source: "url", code: false },
  hex: { label: "Hex", source: "bytes", code: false },
};

const BY_KIND: Record<ContentKind, ViewerId[]> = {
  none: ["text", "hex"],
  json: ["json", "json-tree", "text", "hex"],
  graphql: ["graphql", "json", "json-tree", "text", "hex"],
  xml: ["xml", "text", "hex"],
  html: ["html", "html-preview", "text", "hex"],
  formurlencoded: ["form", "text", "hex"],
  multipart: ["multipart", "text", "hex"],
  javascript: ["javascript", "text", "hex"],
  css: ["css", "text", "hex"],
  text: ["text", "json", "hex"],
  image: ["image", "hex"],
  svg: ["image", "xml", "text", "hex"],
  video: ["video", "hex"],
  audio: ["audio", "hex"],
  font: ["font", "hex"],
  pdf: ["pdf", "hex"],
  binary: ["hex", "text"],
};

export function viewersFor(kind: ContentKind): ViewerId[] {
  return BY_KIND[kind];
}

/** The user's choice for this kind if it applies, else the best viewer. */
export function pickViewer(kind: ContentKind, preferred: ViewerId | undefined): ViewerId {
  const options = viewersFor(kind);
  return preferred && options.includes(preferred) ? preferred : options[0]!;
}
