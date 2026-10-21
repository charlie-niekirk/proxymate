import type { ContentKind } from "@/bindings";

/**
 * Content detection in the UI, for bodies Rust never sees on their own
 * (multipart parts) and for the mock backend. A simplified mirror of
 * `pm-core/src/io/content.rs`, which stays the source of truth for flows.
 */

export interface Sniffed {
  kind: ContentKind;
  mime: string;
}

/** `type/subtype` of a Content-Type value, lower-cased. */
export function essence(contentType: string | null | undefined): string {
  return (contentType ?? "").split(";")[0]!.trim().toLowerCase();
}

export function kindFromContentType(contentType: string | null | undefined): ContentKind {
  const mime = essence(contentType);
  const [top = "", sub = ""] = mime.split("/");
  if (!mime) return "none";
  if (sub === "json" || sub === "graphql-response+json") return "json";
  if (sub === "graphql" || sub === "graphql+json") return "graphql";
  if (sub.endsWith("+json")) return "json";
  if (mime === "text/html" || mime === "application/xhtml+xml") return "html";
  if (mime === "image/svg+xml") return "svg";
  if (sub === "xml" || sub.endsWith("+xml")) return "xml";
  if (mime === "application/x-www-form-urlencoded") return "formurlencoded";
  if (top === "multipart") return "multipart";
  if (["javascript", "ecmascript", "x-javascript"].includes(sub)) return "javascript";
  if (mime === "text/css") return "css";
  if (top === "text") return "text";
  if (top === "image") return "image";
  if (top === "video") return "video";
  if (top === "audio") return "audio";
  if (top === "font" || ["application/font-woff", "application/font-woff2", "application/x-font-ttf"].includes(mime)) return "font";
  if (mime === "application/pdf") return "pdf";
  return "binary";
}

const MAGIC: [number, number[], ContentKind, string][] = [
  [0, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], "image", "image/png"],
  [0, [0xff, 0xd8, 0xff], "image", "image/jpeg"],
  [0, [0x47, 0x49, 0x46, 0x38], "image", "image/gif"],
  [0, [0x25, 0x50, 0x44, 0x46, 0x2d], "pdf", "application/pdf"],
  [0, [0x1a, 0x45, 0xdf, 0xa3], "video", "video/webm"],
  [0, [0x4f, 0x67, 0x67, 0x53], "audio", "audio/ogg"],
  [0, [0x49, 0x44, 0x33], "audio", "audio/mpeg"],
  [0, [0x77, 0x4f, 0x46, 0x46], "font", "font/woff"],
  [0, [0x77, 0x4f, 0x46, 0x32], "font", "font/woff2"],
  [0, [0x1f, 0x8b], "binary", "application/gzip"],
  [0, [0x50, 0x4b, 0x03, 0x04], "binary", "application/zip"],
];

function at(bytes: Uint8Array, offset: number, sig: number[] | string): boolean {
  const s = typeof sig === "string" ? [...sig].map((c) => c.charCodeAt(0)) : sig;
  return s.every((b, i) => bytes[offset + i] === b);
}

function sniffMagic(b: Uint8Array): Sniffed | null {
  for (const [offset, sig, kind, mime] of MAGIC) if (at(b, offset, sig)) return { kind, mime };
  if (at(b, 0, "RIFF") && at(b, 8, "WEBP")) return { kind: "image", mime: "image/webp" };
  if (at(b, 0, "RIFF") && at(b, 8, "WAVE")) return { kind: "audio", mime: "audio/wav" };
  if (at(b, 4, "ftyp")) {
    const brand = String.fromCharCode(...b.subarray(8, 12));
    if (brand === "avif" || brand === "avis") return { kind: "image", mime: "image/avif" };
    if (brand.startsWith("M4A")) return { kind: "audio", mime: "audio/mp4" };
    return { kind: "video", mime: "video/mp4" };
  }
  if (at(b, 0, [0, 0, 1, 0]) && (b[4] ?? 0) > 0) return { kind: "image", mime: "image/x-icon" };
  if (at(b, 0, "BM") && b.length > 18) return { kind: "image", mime: "image/bmp" };
  return null;
}

/** True if the bytes are not printable UTF-8 text. */
export function looksBinary(head: Uint8Array): boolean {
  if (head.includes(0)) return true;
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(head);
  } catch {
    // Allow a multi-byte sequence cut at the end.
    try {
      new TextDecoder("utf-8", { fatal: true }).decode(head.subarray(0, Math.max(0, head.length - 3)));
    } catch {
      return true;
    }
  }
  let controls = 0;
  for (const c of head) if (c < 0x20 && c !== 9 && c !== 10 && c !== 13 && c !== 12 && c !== 27) controls++;
  return controls * 100 > head.length;
}

function sniffText(head: Uint8Array): Sniffed | null {
  if (looksBinary(head)) return null;
  const text = new TextDecoder().decode(head.subarray(0, 1024)).replace(/^\uFEFF/, "").trimStart();
  const lower = text.slice(0, 256).toLowerCase();
  if (/^\{\s*["}]/.test(text) || /^\[\s*["{[\]\-0-9tfn]/.test(text) || text === "{" || text === "[") {
    return { kind: "json", mime: "application/json" };
  }
  if (/^<(!doctype html|html|head|body)/.test(lower)) return { kind: "html", mime: "text/html" };
  if (lower.startsWith("<svg") || (lower.startsWith("<?xml") && text.toLowerCase().includes("<svg"))) {
    return { kind: "svg", mime: "image/svg+xml" };
  }
  if (lower.startsWith("<?xml") || /^<[a-z]/.test(lower)) return { kind: "xml", mime: "application/xml" };
  return { kind: "text", mime: "text/plain" };
}

/** Detects a body from its Content-Type and first bytes, like the Rust detector. */
export function sniff(bytes: Uint8Array, contentType?: string | null): Sniffed {
  const head = bytes.subarray(0, 16 * 1024);
  if (head.length === 0) return { kind: "none", mime: essence(contentType) };
  const magic = sniffMagic(head);
  if (magic) return magic;
  const header = kindFromContentType(contentType);
  const mime = essence(contentType);
  if (header === "none" || header === "text" || mime === "application/octet-stream") {
    const text = sniffText(head);
    if (!text) return { kind: "binary", mime: "application/octet-stream" };
    return header === "text" && text.kind === "text" ? { kind: "text", mime } : text;
  }
  return { kind: header, mime };
}
