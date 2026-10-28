import { FormView } from "./FormView";
import { GraphQlView } from "./GraphQlView";
import { HexView } from "./HexView";
import { JsonPretty, JsonTree } from "./JsonViews";
import { FontView, ImageView, MediaView, PdfView } from "./MediaViews";
import { MultipartView } from "./MultipartView";
import { Centered } from "./parts";
import { HtmlPreview, MarkupView, ScriptView, TextView } from "./TextViews";
import type { ViewerInput } from "./Viewer";

// The GraphQL operations list is also shown for GET requests, from `BodyViewer`.
export { Operations } from "./GraphQlView";
// Decoded viewers (M16) load their own data, from `BodyViewer`.
export { DecodedView } from "./DecodedView";

/** Renders one body with one viewer from the registry (loaded lazily by `Viewer`). */
export function ViewerSwitch({ viewer, bytes, url, mime, size, contentType, highlight, bodyId, onSave }: ViewerInput) {
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
