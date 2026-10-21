/** EXIF orientation (1–8) of a JPEG, or null if it has none. */
export function jpegOrientation(b: Uint8Array): number | null {
  if (b[0] !== 0xff || b[1] !== 0xd8) return null;
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  let pos = 2;
  while (pos + 4 <= b.length && b[pos] === 0xff) {
    const marker = b[pos + 1]!;
    const len = view.getUint16(pos + 2);
    if (marker === 0xda) break; // start of scan: no more metadata
    if (marker === 0xe1 && String.fromCharCode(...b.subarray(pos + 4, pos + 8)) === "Exif") {
      return tiffOrientation(view, pos + 10, len - 8);
    }
    pos += 2 + len;
  }
  return null;
}

function tiffOrientation(view: DataView, start: number, length: number): number | null {
  if (start + 8 > view.byteLength) return null;
  const little = view.getUint16(start) === 0x4949;
  const u16 = (o: number) => view.getUint16(start + o, little);
  const u32 = (o: number) => view.getUint32(start + o, little);
  const ifd = u32(4);
  if (ifd + 2 > length) return null;
  const count = u16(ifd);
  for (let i = 0; i < count; i++) {
    const entry = ifd + 2 + i * 12;
    if (entry + 12 > length) return null;
    if (u16(entry) === 0x0112) {
      const value = u16(entry + 8);
      return value >= 1 && value <= 8 ? value : null;
    }
  }
  return null;
}

const ORIENTATIONS = [
  "",
  "Normal",
  "Mirrored",
  "Rotated 180°",
  "Mirrored vertically",
  "Mirrored, rotated 90° CCW",
  "Rotated 90° CW",
  "Mirrored, rotated 90° CW",
  "Rotated 90° CCW",
];

export function orientationLabel(o: number): string {
  return ORIENTATIONS[o] ?? `Unknown (${o})`;
}

/** True for animated GIF, APNG and animated WebP. */
export function isAnimated(b: Uint8Array, mime: string): boolean {
  const has = (sig: string, from = 0, to = b.length) => {
    const codes = [...sig].map((c) => c.charCodeAt(0));
    outer: for (let i = from; i <= Math.min(to, b.length) - codes.length; i++) {
      for (let j = 0; j < codes.length; j++) if (b[i + j] !== codes[j]) continue outer;
      return true;
    }
    return false;
  };
  switch (mime) {
    case "image/gif": {
      // More than one image descriptor after a graphic control extension.
      let frames = 0;
      for (let i = 0; i < b.length - 2 && frames < 2; i++) if (b[i] === 0x21 && b[i + 1] === 0xf9 && b[i + 2] === 0x04) frames++;
      return frames > 1 || has("NETSCAPE2.0");
    }
    case "image/png":
      return has("acTL", 8, 4096);
    case "image/webp":
      return has("ANIM", 12, 4096);
    default:
      return false;
  }
}
