import { useMemo } from "react";

import type { BodyPart } from "@/bindings";
import { bodyText } from "@/lib/http";

/** The body as UTF-8 text, or null if it is binary. */
export function useText(bytes: Uint8Array): string | null {
  return useMemo(() => bodyText(bytes), [bytes]);
}

/** `image/svg+xml` → `SVG`, `font/woff2` → `WOFF2`. */
export function formatLabel(mime: string | null | undefined): string {
  if (!mime) return "Unknown";
  const sub = mime.split("/")[1] ?? mime;
  const special: Record<string, string> = { "svg+xml": "SVG", "x-icon": "ICO", "vnd.microsoft.icon": "ICO", jpeg: "JPEG", mpeg: "MP3", "x-msvideo": "AVI", quicktime: "MOV" };
  return special[sub] ?? sub.replace(/^x-/, "").toUpperCase();
}

/** Above this, only the head of a body is loaded until the user asks for all of it. */
export const LOAD_LIMIT = 20 * 1024 * 1024;
/** Above this, code viewers show plain text until the user asks for highlighting. */
export const HIGHLIGHT_LIMIT = 2 * 1024 * 1024;

const EXTENSIONS: Record<string, string> = {
  "application/json": "json", "text/html": "html", "application/xml": "xml", "text/xml": "xml", "text/plain": "txt",
  "text/css": "css", "text/javascript": "js", "application/javascript": "js", "image/png": "png", "image/jpeg": "jpg",
  "image/gif": "gif", "image/webp": "webp", "image/avif": "avif", "image/svg+xml": "svg", "image/x-icon": "ico",
  "image/bmp": "bmp", "video/mp4": "mp4", "video/webm": "webm", "audio/mpeg": "mp3", "audio/ogg": "ogg",
  "application/pdf": "pdf", "font/woff2": "woff2", "font/woff": "woff", "font/ttf": "ttf", "font/otf": "otf",
};

/** A file name for "Save body…": the URL's last segment, with an extension that matches the body. */
export function suggestedFileName(url: string, mime: string, part: BodyPart): string {
  let last = "body";
  try {
    last = decodeURIComponent(new URL(url).pathname.split("/").filter(Boolean).pop() ?? "body");
  } catch {
    /* keep "body" */
  }
  last = last.replace(/[^\w.@+-]/g, "_") || "body";
  const ext = EXTENSIONS[mime];
  const base = part === "request" ? `${last}-request` : last;
  return ext && !base.toLowerCase().endsWith(`.${ext}`) ? `${base}.${ext}` : base;
}
