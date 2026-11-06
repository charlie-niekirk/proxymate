"""Build every Proxymate brand master from code: python3 brand/src/build.py (writes into brand/)."""
import math, os, sys
sys.path.insert(0, os.path.dirname(__file__))
from detour import detour
from wordmark import wordmark

OUT = os.path.join(os.path.dirname(__file__), "..")
INK, SIGNAL, PAPER, WHITE = "#101820", "#C8F04B", "#F6F6F1", "#FFFFFF"

def bbox_symbol(sw, r, cy, tail, x0, x1):
    return (x0 - sw/2, cy - r - sw/2, x1 + sw/2, tail + sw/2)

def symbol(small=False):
    """Master (≥ 48 px) or small cut (≤ 32 px: heavier line, wider crossing gap, bigger loop)."""
    p = dict(sw=32, r=50, R=30, gap=13, x0=34, x1=222) if small else dict(sw=24, r=56, R=40, gap=8, x0=20, x1=236)
    sw, r = p["sw"], p["r"]
    drop = 34 if small else 38                       # vertical run from the crossing to the tail line
    height = (r + sw/2) + r*math.sqrt(2) + drop + sw/2
    top = (256 - height)/2 - 3                       # optical centre: a touch above the geometric one
    cy = top + r + sw/2
    tail = cy + r*math.sqrt(2) + drop
    paths, _ = detour(cy=cy, tail=tail, **p)
    return paths, bbox_symbol(sw, r, cy, tail, p["x0"], p["x1"]), p

def doc(inner, vb, title, w=None, h=None):
    x, y, vw, vh = math.floor(vb[0]), math.floor(vb[1]), math.ceil(vb[2]), math.ceil(vb[3])
    w = w or vw; h = h or vh
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x:g} {y:g} {vw:g} {vh:g}" width="{w:g}" height="{h:g}" '
            f'role="img"><title>{title}</title>{inner}</svg>\n')

def g(paths, fill, tr=""):
    t = f' transform="{tr}"' if tr else ""
    return f'<g fill="{fill}"{t}>' + "".join(f'<path d="{d}"/>' for d in paths) + "</g>"

def write(name, s):
    os.makedirs(os.path.dirname(os.path.join(OUT, name)), exist_ok=True)
    open(os.path.join(OUT, name), "w").write(s)

sym, sbox, sp = symbol()
small, _, _ = symbol(small=True)
wm, wm_w = wordmark()

# --- symbol masters
for col, hexv in (("ink", INK), ("signal", SIGNAL), ("black", "#000000"), ("white", WHITE)):
    write(f"svg/proxymate-symbol-{col}.svg", doc(g(sym, hexv), (0, 0, 256, 256), "Proxymate"))
    write(f"svg/proxymate-symbol-small-{col}.svg", doc(g(small, hexv), (0, 0, 256, 256), "Proxymate"))
write("svg/proxymate-symbol-signal-on-ink.svg",
      doc(f'<rect width="256" height="256" rx="57" fill="{INK}"/>' + g(sym, SIGNAL, "translate(128 128) scale(.74) translate(-128 -128)"),
          (0, 0, 256, 256), "Proxymate"))

# --- lockups. Symbol scaled so its line weight equals the letters' (22); its tail sits on the baseline.
S = 22 / sp["sw"]
sym_w = (sbox[2] - sbox[0]) * S; sym_h = (sbox[3] - sbox[1]) * S
GAP = 44                                              # symbol → wordmark
PAD = 0
def horizontal(sym_fill, wm_fill):
    tx = -sbox[0]*S; ty = -sbox[3]*S                  # symbol bottom edge on baseline y=0
    inner = g(sym, sym_fill, f"translate({tx:.2f} {ty:.2f}) scale({S:.3f})") + g(wm, wm_fill, f"translate({sym_w + GAP:.2f} 0)")
    top = -sym_h; bottom = 47                         # descenders
    W = sym_w + GAP + wm_w
    return doc(inner, (-PAD, top - PAD, W + 2*PAD, bottom - top + 2*PAD), "Proxymate")
def stacked(sym_fill, wm_fill):
    s2 = S * 1.5                                      # symbol larger over the word
    sw2 = (sbox[2]-sbox[0])*s2; sh2 = (sbox[3]-sbox[1])*s2
    tx = (wm_w - sw2)/2 - sbox[0]*s2; ty = -sh2 - 72 - sbox[1]*s2 - 100
    inner = g(sym, sym_fill, f"translate({tx:.2f} {ty:.2f}) scale({s2:.3f})") + g(wm, wm_fill)
    top = -100 - 72 - sh2; bottom = 47
    return doc(inner, (0, top, wm_w, bottom - top), "Proxymate")
def wordonly(fill):
    return doc(g(wm, fill), (0, -133, wm_w, 180), "Proxymate")

for name, sf, wf in (("ink", INK, INK), ("black", "#000000", "#000000"), ("white", WHITE, WHITE), ("on-ink", SIGNAL, WHITE)):
    write(f"svg/proxymate-horizontal-{name}.svg", horizontal(sf, wf))
    write(f"svg/proxymate-stacked-{name}.svg", stacked(sf, wf))
    if name != "on-ink": write(f"svg/proxymate-wordmark-{name}.svg", wordonly(wf))

# --- macOS app icon (1024 template: 824 tile, 100 margin). Flat ink tile, Signal line.
def app_icon(paths, scale):
    t = f"translate(512 {512 - 6}) scale({scale}) translate(-128 -128)"
    return doc(f'<rect x="100" y="100" width="824" height="824" rx="185" fill="{INK}"/>' + g(paths, SIGNAL, t),
               (0, 0, 1024, 1024), "Proxymate")
write("app-icon/proxymate-app-icon.svg", app_icon(sym, 2.35))
write("app-icon/proxymate-app-icon-small.svg", app_icon(small, 2.55))
print("symbol bbox", [round(v, 1) for v in sbox], "lockup scale", round(S, 3), "wordmark width", round(wm_w, 1))
