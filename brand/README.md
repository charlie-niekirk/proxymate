# Proxymate: logo guidelines

## 1. The logo

**Idea.** The mark is called *Detour*. One line loops back through itself, because every request takes a detour through Proxymate. The over/under crossing is the moment Proxymate steps in. The **x** in the wordmark repeats the same crossing.

**Versions** (all in `svg/`, with PNGs in `png/`):

| Version | File | Use it for |
|---|---|---|
| Horizontal (primary) | `proxymate-horizontal-{ink,black,white,on-ink}.svg` | Headers, README, website, docs |
| Stacked | `proxymate-stacked-{ink,black,white,on-ink}.svg` | Square-ish spaces, splash screens, stickers |
| Symbol | `proxymate-symbol-{ink,signal,black,white}.svg` | Anything shown at **48 px or larger** |
| Symbol, small cut | `proxymate-symbol-small-*.svg` | **16–40 px**: favicons, menu bar, toolbar, tabs |
| Symbol on tile | `proxymate-symbol-signal-on-ink.svg` | Avatars and social profiles |
| Wordmark only | `proxymate-wordmark-{ink,black,white}.svg` | Only where the symbol already appears nearby |
| macOS app icon | `app-icon/proxymate.icns`, `proxymate-app-icon(-small).svg`, `-1024.png` | Dock and Finder |
| Web icons | `web/` (`favicon.ico`, `favicon.svg`, touch icon, PWA icons, `site.webmanifest`, `head-snippet.html`) | Websites and docs |

The small cut uses a heavier line, a bigger loop and a wider gap at the crossing, so the crossing still shows at 16–32 px. The app icon and the favicon use it at their small sizes automatically. `web/favicon.svg` follows the browser theme: ink on light tabs, Signal on dark tabs.

## 2. Clear space

- **Lockups:** keep clear space of **one x-height** of the wordmark on every side. That's the height of the "o".
- **Symbol on its own:** keep clear space of **the inner width of the loop** on every side.

Both rules scale with the logo. Never use a fixed distance.

## 3. Minimum size

| Version | Screen | Print |
|---|---|---|
| Horizontal | 120 px wide | 30 mm wide |
| Stacked | 64 px wide | 18 mm wide |
| Symbol (master) | 48 px | 12 mm |
| Symbol (small cut) | 16 px | 5 mm |

## 4. Colour

| Name | HEX | RGB | CMYK (approx.) | Pantone (nearest) |
|---|---|---|---|---|
| **Ink** | `#101820` | 16 24 32 | 100 79 44 93 | Black 6 C |
| **Signal** | `#C8F04B` | 200 240 75 | 22 0 80 0 | ≈ 374 C (check against a physical swatch) |
| Paper | `#F6F6F1` | 246 246 241 | 2 1 4 0 | n/a |

**Approved pairs:**

| Pair | Contrast |
|---|---|
| Ink on white or Paper | 17.9 : 1 / 16.5 : 1 |
| Signal symbol with a white wordmark, on Ink (`on-ink`) | 13.7 : 1 for Signal on Ink |
| White on Ink | 17.9 : 1 |
| Black on white | Use for one-colour print |

**Signal never goes on a light background** (1.3 : 1 on white): it's a colour for dark backgrounds only. On light backgrounds the logo is always Ink.

On photos, use the white or Ink version on a calm area, or put the symbol on its Ink tile.

## 5. Typography

- The wordmark is **custom lettering built from the symbol's own parts**: the same line, round ends, circles and the crossing. It is not a font, so there's nothing to license. Never retype it. Use the files.
- Companion type:
  - UI and docs: the system font (SF Pro on macOS), or **Inter** elsewhere (SIL OFL).
  - Code: **JetBrains Mono** (SIL OFL).

## 6. Don'ts

Don't:
- stretch, squash or rotate the logo;
- recolour it outside the palette, or put Signal on a light background;
- add shadows, outlines, gradients or glows;
- close the gap at the crossing, or use the master below 48 px (use the small cut instead);
- move or resize the parts of a lockup;
- retype the wordmark in a font.

## 7. Source and rebuilding

Every master is generated from code. There's no hand-edited artwork to keep in sync.

- `src/geom.py` outlines strokes made of lines and arcs into filled paths. Offset arcs stay true arcs, and the masters contain no strokes, masks or text.
- `src/detour.py` draws the symbol.
- `src/wordmark.py` draws the lettering.
- `src/build.py` writes every SVG.

Regenerate the SVGs with:

```bash
python3 brand/src/build.py
```

The PNG, `.icns` and web-icon exports were rendered from those SVGs with the logo-design skill's `render_png.py` and `export_variants.py`, and with `iconutil`.

---

## Handover notes

**Why this direction.** The category's marks lean on magnifiers, eyes, padlocks, globes, ⇄ arrows, `</>` brackets and blue. Detour avoids all of them. It tells the product's story in one gesture, and the crossing gives it one ownable detail that the wordmark repeats. Ink and Signal give a quiet, native-feeling Dock icon with one bright detail that reads as "live".

**Tests run:**
- `svg_audit.py`: symbol, small cut and stacked lockup score 100/100; horizontal lockup 99/100. It's informational only: an extra-wide lockup, which is why there's a stacked one.
- True-pixel renders at 16 and 32 px of the master, the small cut and the app icon. The small cut is clearly stronger at small sizes and is the one used there.
- Contrast checks for every approved pair (table in §4).
- An outline check: each filled outline was laid over a real stroke of the same centreline, and they matched.
- Mockups on a presentation board (`board/proxymate-identity.html`, slides in `board/slides/`).

**Not done or open:**
- **Trademark clearance is not checked.** Do a professional search, plus a reverse image search, before adopting the mark.
- **No side-by-side comparison against competitors' logos.** The skill's reference library was missing a file, so its shelf test couldn't run. Comparing the mark by eye with Proxyman's, Charles's, HTTP Toolkit's and mitmproxy's icons is worth doing.
- **The Pantone match for Signal is approximate.** Check it against a physical swatch before print.
- **The horizontal tails are not optically thinned.** The line has one weight on purpose, so the monoline idea stays pure.
- **The app icons are installed** in `apps/desktop/src-tauri/icons/`. Sizes of 48 px and below use the small cut (`proxymate-app-icon-small.svg`), and `icon.icns` is `app-icon/proxymate.icns`. Don't regenerate them with `npm run tauri icon`: it shrinks the 1024 px master for every size, and the small cut would be lost.
- **Windows and Linux use the macOS artwork** (a rounded tile with a margin). A full-bleed Windows variant is optional.
