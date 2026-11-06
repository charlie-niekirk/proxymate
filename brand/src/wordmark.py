"""The proxymate wordmark: lowercase monoline built from the symbol's parts (lines, circles, round ends).
The x carries the symbol's over/under crossing. Coordinates: baseline y=0, x-height 100, y down."""
import math
from geom import Arc, Line, outline, ring, polyline, f

W = 22          # stroke
H = W/2
XH = 100
TOP, BOT = -XH + H, -H          # centreline top/bottom of x-height strokes
DESC = 36                       # centreline end of descenders
RB = 40                         # bowl centreline radius (outer 51: 1 % overshoot)
BY = -50                        # bowl centre
GAP = 9                         # crossing gap in the x

def letter_p(x):   # x = left outer edge
    s = x + H; c = (s + RB, BY)
    return [outline([Line((s, TOP), (s, DESC))], W), ring(c, RB, W)], s + RB + RB + H

def letter_o(x):
    c = (x + H + RB, BY); return [ring(c, RB, W)], x + 2*(RB + H)

def letter_a(x):
    c = (x + H + RB, BY); s = c[0] + RB
    return [ring(c, RB, W), outline([Line((s, TOP), (s, BOT))], W)], s + H

def letter_r(x):
    s = x + H; rr = 36
    shoulder = Arc((s + rr, BY + 4), rr, 180, 270)      # rises from the stem into the top
    e = shoulder.end()
    return [outline([Line((s, BOT), (s, TOP))], W), outline([shoulder, Line(e, (e[0] + 8, e[1]))], W, "butt")], e[0] + 8 + H

def letter_x(x):
    hw = (TOP - BOT) / -2 / math.tan(math.radians(60))   # 60° diagonals
    x0, x1 = x + H, x + H + 2*hw
    cx, cy = (x0 + x1)/2, (TOP + BOT)/2
    a = math.radians(60); u = (math.cos(a), -math.sin(a))  # up-right
    g = H + GAP
    rise1 = [Line((x0, BOT), (cx - u[0]*g, cy - u[1]*g))]
    rise2 = [Line((cx + u[0]*g, cy + u[1]*g), (x1, TOP))]
    fall = [Line((x0, TOP), (x1, BOT))]
    return [outline(rise1, W, "round", "butt"), outline(rise2, W, "butt", "round"), outline(fall, W)], x1 + H

def letter_y(x):
    hw = (BOT - TOP) / math.tan(math.radians(75))     # arms at 75°
    x0, x1 = x + H, x + H + 2*hw
    v = ((x0 + x1)/2, BOT)
    k = (DESC - BOT) / math.tan(math.radians(75))
    return [outline([Line((x0, TOP), v)], W),
            outline([Line((x1, TOP), (v[0] - k, DESC))], W)], x1 + H

def letter_m(x):
    s = x + H; ra = 30; cy = -XH + H + ra       # arch tops reach the x-height
    a1 = Arc((s + ra, cy), ra, 180, 360); a2 = Arc((s + 3*ra, cy), ra, 180, 360)
    return [outline([Line((s, BOT), (s, cy)), a1, Line(a1.end(), (s + 2*ra, BOT))], W),
            outline([Line((s + 2*ra, cy + 0.01), (s + 2*ra, cy)), a2, Line(a2.end(), (s + 4*ra, BOT))], W)], s + 4*ra + H

def letter_t(x):
    s = x + H + 16; rr = 26
    hook = Arc((s + rr, BOT - rr), rr, 180, 90)
    stem = [Line((s, -132 + H), (s, BOT - rr)), hook, Line(hook.end(), (s + rr + 10, BOT))]
    bar = [Line((s - 16, TOP), (s + 34, TOP))]
    return [outline(stem, W), outline(bar, W)], s + 34 + H

def letter_e(x):
    c = (x + H + RB, BY)
    # the bar's right end follows the bowl's outer edge, so no corner pokes out
    xe = c[0] + math.sqrt((RB + H)**2 - H**2)
    bar = (f"M{f(c[0]-RB)} {f(BY-H)}L{f(c[0]-RB)} {f(BY+H)}L{f(xe)} {f(BY+H)}"
           f"A{f(RB+H)} {f(RB+H)} 0 0 0 {f(xe)} {f(BY-H)}Z")
    bowl = [Arc(c, RB, 0, -315)]                      # over the top, round the left, to the lower right
    return [bar, outline(bowl, W, "butt", "round")], c[0] + RB + H

GLYPHS = dict(p=letter_p, r=letter_r, o=letter_o, x=letter_x, y=letter_y, m=letter_m, a=letter_a, t=letter_t, e=letter_e)
# Optical spacing between outer edges (round-round tighter, straight-straight looser, diagonals tightest)
SPACE = {"pr": 16, "ro": 8, "ox": 9, "xy": 20, "ym": 8, "ma": 16, "at": 6, "te": 8}

def wordmark(text="proxymate"):
    x, paths, prev = 0.0, [], None
    for ch in text:
        if prev: x += SPACE.get(prev + ch, 12)
        ps, x = GLYPHS[ch](x); paths += ps; prev = ch
    return paths, x     # x = total width

if __name__ == "__main__":
    from detour import svg
    ps, w = wordmark()
    print(w)
    open("/private/tmp/claude-501/-Users-cniekirk-Projects-proxymate/a36263b8-fe55-4cf1-a0e7-e6910048d3dd/scratchpad/logo/wm1.svg", "w").write(
        svg(ps, fill="#000", vb=f"-20 -150 {w+40} 210", w=int(w+40), h=210))
