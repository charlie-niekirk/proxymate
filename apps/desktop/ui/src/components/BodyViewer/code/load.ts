/**
 * Entry points to the lazily loaded parts of the body viewers, kept free of
 * static CodeMirror imports so that importing this module costs nothing.
 */
import type { ViewerId } from "../registry";
import type { CodeLanguage } from "./languages";

/** CodeMirror and the read-only code view. */
export const loadCodeView = () => import("./CodeMirrorView");

/** The editable CodeMirror (Compose, the Breakpoint Editor). */
export const loadCodeEditor = () => import("@/components/Compose/CodeMirrorEditor");

/** Every viewer component behind `Viewer`. */
export const loadViewers = () => import("../Viewers");

const GRAMMAR: Partial<Record<ViewerId, CodeLanguage>> = {
  json: "json",
  xml: "xml",
  html: "html",
  javascript: "javascript",
  css: "css",
};

/**
 * Starts loading what `viewer` will need while its body is still on its
 * way, so the chunks don't add up one after another afterwards.
 */
export function preloadViewer(viewer: ViewerId, code: boolean) {
  const swallow = () => {};
  loadViewers().catch(swallow);
  if (!code) return;
  const lang = GRAMMAR[viewer];
  import("./languages").then((m) => (lang ? m.loadLanguage(lang) : null)).catch(swallow);
  loadCodeView().catch(swallow);
}
