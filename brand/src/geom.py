"""Stroke geometry for the Proxymate marks: lines and circular arcs of constant width,
outlined analytically into filled paths (offsets of arcs are arcs), so masters have no strokes."""
import math

def f(x):
    s = f"{x:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s

class Line:
    def __init__(s, p0, p1): s.p0, s.p1 = p0, p1
    def start(s): return s.p0
    def end(s): return s.p1
    def dir0(s): return _norm((s.p1[0]-s.p0[0], s.p1[1]-s.p0[1]))
    dir1 = dir0
    def offset(s, h):
        dx, dy = s.dir0(); n = (-dy, dx)
        return Line((s.p0[0]+n[0]*h, s.p0[1]+n[1]*h), (s.p1[0]+n[0]*h, s.p1[1]+n[1]*h))
    def reverse(s): return Line(s.p1, s.p0)
    def d(s): return f"L{f(s.p1[0])} {f(s.p1[1])}"
    def stroke_d(s): return d_line(s)

def d_line(s): return s.d()

class Arc:
    """Centre c, radius r, from angle t0 to t1 (degrees, screen space: y down). t1>t0 = sweep 1."""
    def __init__(s, c, r, t0, t1): s.c, s.r, s.t0, s.t1 = c, r, t0, t1
    def pt(s, t): a = math.radians(t); return (s.c[0]+s.r*math.cos(a), s.c[1]+s.r*math.sin(a))
    def start(s): return s.pt(s.t0)
    def end(s): return s.pt(s.t1)
    def _tan(s, t):
        a = math.radians(t); sg = 1 if s.t1 > s.t0 else -1
        return (-math.sin(a)*sg, math.cos(a)*sg)
    def dir0(s): return s._tan(s.t0)
    def dir1(s): return s._tan(s.t1)
    def offset(s, h):
        rr = s.r - h if s.t1 > s.t0 else s.r + h
        assert rr > 0, "offset radius collapses"
        return Arc(s.c, rr, s.t0, s.t1)
    def reverse(s): return Arc(s.c, s.r, s.t1, s.t0)
    def d(s):
        span = abs(s.t1 - s.t0)
        if span >= 359.99:   # split full circles
            mid = (s.t0 + s.t1) / 2
            return Arc(s.c, s.r, s.t0, mid).d() + Arc(s.c, s.r, mid, s.t1).d()
        large = 1 if span > 180 else 0; sweep = 1 if s.t1 > s.t0 else 0
        e = s.end()
        return f"A{f(s.r)} {f(s.r)} 0 {large} {sweep} {f(e[0])} {f(e[1])}"

def _norm(v):
    l = math.hypot(*v); return (v[0]/l, v[1]/l)

def polyline(*pts):
    return [Line(pts[i], pts[i+1]) for i in range(len(pts)-1)]

def outline(segs, w, cap0="round", cap1="round"):
    """Closed outline of an open stroke (consistent winding for every stroke, so nonzero fill unions)."""
    h = w / 2
    left = [s.offset(h) for s in segs]
    right = [s.offset(-h).reverse() for s in reversed(segs)]
    p = left[0].start()
    d = f"M{f(p[0])} {f(p[1])}"
    for s in left: d += s.d()
    d += _cap(left[-1].end(), right[0].start(), h, cap1)
    for s in right: d += s.d()
    d += _cap(right[-1].end(), left[0].start(), h, cap0)
    return d + "Z"

def _cap(a, b, h, kind):
    if kind == "round": return f"A{f(h)} {f(h)} 0 0 0 {f(b[0])} {f(b[1])}"
    return f"L{f(b[0])} {f(b[1])}"

def ring(c, r, w):
    """Annulus: outer drawn visually counter-clockwise (same as strokes), inner clockwise -> a hole."""
    ro, ri = r + w/2, r - w/2
    o = Arc(c, ro, 0, -360); i = Arc(c, ri, 0, 360)
    po, pi = o.start(), i.start()
    return f"M{f(po[0])} {f(po[1])}{o.d()}ZM{f(pi[0])} {f(pi[1])}{i.d()}Z"

def stroked(segs):
    """Centerline path, for checking outlines against a real stroke."""
    p = segs[0].start(); return f"M{f(p[0])} {f(p[1])}" + "".join(s.d() for s in segs)
