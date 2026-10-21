/** A multipart/* body split into its parts. */

export interface MultipartPart {
  headers: [string, string][];
  /** `name` from Content-Disposition. */
  name: string | null;
  filename: string | null;
  contentType: string | null;
  body: Uint8Array;
}

/** The `boundary` parameter of a multipart Content-Type. */
export function boundaryOf(contentType: string | null | undefined): string | null {
  const m = /;\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType ?? "");
  return m ? (m[1] ?? m[2] ?? null) : null;
}

function indexOf(hay: Uint8Array, needle: Uint8Array, from: number): number {
  const first = needle[0];
  outer: for (let i = hay.indexOf(first!, from); i !== -1 && i <= hay.length - needle.length; i = hay.indexOf(first!, i + 1)) {
    for (let j = 1; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
    return i;
  }
  return -1;
}

function param(value: string, name: string): string | null {
  const m = new RegExp(`;\\s*${name}\\*?=(?:"((?:[^"\\\\]|\\\\.)*)"|([^;]+))`, "i").exec(value);
  if (!m) return null;
  const raw = m[1] ?? m[2]!.trim();
  return raw.replace(/\\(.)/g, "$1");
}

/**
 * Splits `body` on `boundary`. Tolerates LF-only line ends and a missing
 * closing delimiter (a truncated capture keeps its last, partial part).
 */
export function parseMultipart(body: Uint8Array, boundary: string): MultipartPart[] {
  const enc = new TextEncoder();
  const delimiter = enc.encode(`--${boundary}`);
  const parts: MultipartPart[] = [];
  let pos = indexOf(body, delimiter, 0);
  while (pos !== -1) {
    let start = pos + delimiter.length;
    if (body[start] === 0x2d && body[start + 1] === 0x2d) break; // closing "--"
    while (body[start] === 0x20 || body[start] === 0x09) start++;
    if (body[start] === 0x0d) start++;
    if (body[start] === 0x0a) start++;
    const next = indexOf(body, delimiter, start);
    let end = next === -1 ? body.length : next;
    if (next !== -1) {
      if (body[end - 1] === 0x0a) end--;
      if (body[end - 1] === 0x0d) end--;
    }
    const chunk = body.subarray(start, Math.max(start, end));
    let headEnd = indexOf(chunk, enc.encode("\r\n\r\n"), 0);
    let sep = 4;
    // No headers at all: the blank line comes first.
    if (chunk[0] === 0x0d && chunk[1] === 0x0a) [headEnd, sep] = [0, 2];
    else if (chunk[0] === 0x0a) [headEnd, sep] = [0, 1];
    else if (headEnd === -1) {
      headEnd = indexOf(chunk, enc.encode("\n\n"), 0);
      sep = 2;
    }
    const headText = new TextDecoder().decode(headEnd === -1 ? chunk : chunk.subarray(0, headEnd));
    const headers = headText
      .split(/\r?\n/)
      .filter((l) => l.includes(":"))
      .map((l): [string, string] => [l.slice(0, l.indexOf(":")).trim(), l.slice(l.indexOf(":") + 1).trim()]);
    const header = (n: string) => headers.find(([k]) => k.toLowerCase() === n)?.[1] ?? null;
    const disposition = header("content-disposition") ?? "";
    parts.push({
      headers,
      name: param(disposition, "name"),
      filename: param(disposition, "filename"),
      contentType: header("content-type"),
      body: headEnd === -1 ? new Uint8Array() : chunk.subarray(headEnd + sep),
    });
    pos = next;
  }
  return parts;
}
