import { describe, expect, it } from "vitest";

import { isAnimated, jpegOrientation, orientationLabel } from "./exif";

function jpeg(orientation: number, little: boolean): Uint8Array {
  const tiff = little
    ? [0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0, 0, 0, 0, 0]
    : [0x4d, 0x4d, 0, 0x2a, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation, 0, 0, 0, 0, 0, 0];
  const app1 = [..."Exif"].map((c) => c.charCodeAt(0)).concat([0, 0], tiff);
  const len = app1.length + 2;
  return new Uint8Array([0xff, 0xd8, 0xff, 0xe1, len >> 8, len & 0xff, ...app1, 0xff, 0xda, 0, 2]);
}

describe("jpegOrientation", () => {
  it("reads big- and little-endian EXIF", () => {
    expect(jpegOrientation(jpeg(6, false))).toBe(6);
    expect(jpegOrientation(jpeg(3, true))).toBe(3);
    expect(orientationLabel(6)).toBe("Rotated 90° CW");
  });

  it("returns null without EXIF", () => {
    expect(jpegOrientation(new Uint8Array([0xff, 0xd8, 0xff, 0xda, 0, 2]))).toBeNull();
    expect(jpegOrientation(new Uint8Array([0x89, 0x50]))).toBeNull();
  });
});

describe("isAnimated", () => {
  it("spots animation chunks", () => {
    const gif = new Uint8Array([..."GIF89a"].map((c) => c.charCodeAt(0)).concat([0x21, 0xf9, 0x04, 0, 0, 0x21, 0xf9, 0x04]));
    expect(isAnimated(gif, "image/gif")).toBe(true);
    expect(isAnimated(gif.subarray(0, 9), "image/gif")).toBe(false);
    const png = new Uint8Array(40);
    png.set([..."acTL"].map((c) => c.charCodeAt(0)), 20);
    expect(isAnimated(png, "image/png")).toBe(true);
    expect(isAnimated(png, "image/jpeg")).toBe(false);
  });
});
