"""The Detour symbol: one line that loops back through itself, crossing under/over at X."""
import math
from geom import Arc, Line, outline

R2 = math.sqrt(2)

def detour(sw=24, r=56, cy=89, tail=207, R=40, gap=8, x0=20, x1=236, cx=128):
    X = cy + r*R2                      # the two 45° tangents of the loop meet here
    T1 = (cx + r/R2, cy + r/R2); T2 = (cx - r/R2, cy + r/R2)
    t0 = X - tail                      # where the diagonal reaches the tail line (negative)
    k = R*math.tan(math.radians(22.5))  # fillet tangent length
    gd = sw/2 + gap                    # under-strand cut, measured along it from X
    # fillet between the tail (heading right) and the 45° diagonal (heading up-right): centre above the tail
    fa = Arc((cx+t0-k, tail-R), R, 90, 45)
    fb = Arc((cx-t0+k, tail-R), R, 135, 90)
    under_a = [Line((x0, tail), fa.start()), fa, Line(fa.end(), (cx-gd/R2, X+gd/R2))]
    loop = Arc((cx, cy), r, 45, -225)  # T1 -> over the top -> T2, visually anticlockwise
    over = [Line((cx+gd/R2, X-gd/R2), T1), loop, Line(T2, fb.start()), fb, Line(fb.end(), (x1, tail))]
    return [outline(under_a, sw, "round", "butt"), outline(over, sw, "butt", "round")], (under_a, over)

def svg(paths, fill="#101820", title="Proxymate", vb="0 0 256 256", w=256, h=256, extra=""):
    body = "".join(f'<path d="{d}"/>' for d in paths)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" width="{w}" height="{h}" role="img">'
            f'<title>{title}</title>{extra}<g fill="{fill}">{body}</g></svg>\n')
