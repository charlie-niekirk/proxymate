# Body-viewer zoo

One fixture per `ContentKind`, listed in `zoo.json`. Both the pm-testkit upstream
(`/zoo/<name>`, see `crates/pm-testkit/src/zoo.rs`) and the UI's mock backend
(`apps/desktop/ui/src/dev/zoo.ts`, used by `npm run dev` with `?zoo` and by the
Playwright viewer tests) serve them, so the real app and the snapshots see the same bytes.

`json-10mb` is generated (`big-json`), not stored. `json-base64` (M14) holds a
base64-encoded JSON field, for the "Decode base64 JSON field" view script.

## Provenance

Everything here was made for this repo, except `font.woff2`, which is Inter
(latin subset, regular) from `@fontsource/inter` 5.3.0 under the SIL Open Font
License 1.1 (`font.woff2.LICENSE`).

Media were generated with:

```sh
magick -size 160x100 radial-gradient:'rgba(255,138,0,1)-rgba(123,47,247,0)' -strip PNG32:image.png
magick -seed 7 -size 120x80 plasma:'#2b6cb0-#f6ad55' -strip -quality 80 photo.jpg   # + an EXIF APP1 with Orientation=6
magick -size 64x64 -delay 30 xc:'#ff8a00' xc:'#7b2ff7' xc:'#00b37e' xc:'#2b6cb0' -loop 0 -strip animated.gif
cwebp -q 80 -alpha_q 90 image.png -o image.webp
magick image.png -strip image.avif
magick -size 32x32 radial-gradient:'#ff8a00-#7b2ff7' -define icon:auto-resize=32,16 favicon.ico
magick -size 48x32 gradient:'#2b6cb0-#00b37e' -strip BMP3:image.bmp
ffmpeg -f lavfi -i testsrc=size=320x180:rate=15 -t 2 -pix_fmt yuv420p -c:v libx264 -profile:v baseline -crf 32 -movflags +faststart -an clip.mp4
ffmpeg -f lavfi -i sine=frequency=440:duration=1.5 -c:a libmp3lame -b:a 48k tone.mp3
```

`document.pdf`, `random.bin` (seeded), `upload.multipart` and the JSON files were
written by a small Python script; `page.html` and `logo.svg` deliberately try to run
scripts and load `http://zoo.invalid/…` resources, which the previews must block.
