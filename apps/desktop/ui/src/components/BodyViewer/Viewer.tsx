import { FormView } from "./FormView";
import { GraphQlView } from "./GraphQlView";
import { HexView } from "./HexView";
import { JsonPretty, JsonTree } from "./JsonViews";
import { FontView, ImageView, MediaView, PdfView } from "./MediaViews";
import { MultipartView } from "./MultipartView";
import { Centered } from "./parts";
import type { ViewerId } from "./registry";
import { HtmlPreview, MarkupView, ScriptView, TextView } from "./TextViews";

export interface ViewerInput {
  viewer: ViewerId;
  /** The decoded body, for "bytes" viewers. */
  bytes: Uint8Array | null;
  /** Where to load the body from, for "url" viewers. */
  url: string | null;
  mime: string;
  size: number;
  contentType: string | null;
  highlight: boolean;
  /** Unique per body, for names that must not clash (fonts). */
  bodyId: string;
  onSave?: () => void;
}

/** Renders one body with one viewer from the registry. */
export function Viewer({ viewer, bytes, url, mime, size, contentType, highlight, bodyId, onSave }: ViewerInput) {
  if (url !== null) {
    switch (viewer) {
      case "image":
        return <ImageView url={url} mime={mime} size={size} onSave={onSave} />;
      case "video":
      case "audio":
        return <MediaView url={url} mime={mime} size={size} kind={viewer} />;
      case "pdf":
        return <PdfView url={url} size={size} />;
      case "font":
        return <FontView url={url} mime={mime} size={size} id={bodyId} />;
      default:
        break;
    }
  }
  if (bytes === null) return <Centered>Loading…</Centered>;
  switch (viewer) {
    case "json":
      return <JsonPretty bytes={bytes} highlight={highlight} />;
    case "json-tree":
      return <JsonTree bytes={bytes} />;
    case "graphql":
      return <GraphQlView bytes={bytes} contentType={contentType} />;
    case "xml":
      return <MarkupView bytes={bytes} html={false} highlight={highlight} />;
    case "html":
      return <MarkupView bytes={bytes} html highlight={highlight} />;
    case "html-preview":
      return <HtmlPreview bytes={bytes} />;
    case "form":
      return <FormView bytes={bytes} />;
    case "multipart":
      return <MultipartView bytes={bytes} contentType={contentType} bodyId={bodyId} />;
    case "javascript":
    case "css":
      return <ScriptView bytes={bytes} lang={viewer} highlight={highlight} />;
    case "hex":
      return <HexView bytes={bytes} />;
    default:
      return <TextView bytes={bytes} />;
  }
}
