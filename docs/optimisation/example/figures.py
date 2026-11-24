"""Draw the figures of docs/optimisation from the run files (out/*.owd, numbers.json).

    python3 docs/optimisation/example/figures.py

Writes SVG files to docs/optimisation/figures/. Standard library only. Colours are the site's
design tokens (website/tokens.css): ink on a white sheet, and the option colours for lines.
Every figure is drawn by this script, not by OWD, and says so in its corner.
"""
import json
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
FIG = HERE.parent / "figures"

INK, INK2, RULE = "#1f1e1b", "#55524b", "#d8caa6"
RED, TEAL, GREEN, OCHRE, PLUM = "#b8432d", "#2c6f7c", "#5a7a24", "#a65f12", "#96487f"
RED_W, TEAL_W, OCHRE_W, GREEN_W = "#eea594", "#9fcad1", "#f1c37f", "#c6d98c"
WATER, GREY = "#dfeaf0", "#8c897f"
FONT = "'Public Sans', system-ui, sans-serif"


def load(name):
    return json.loads((OUT / f"{name}.owd").read_text(encoding="utf-8"))


class Svg:
    def __init__(self, w, h, title):
        self.w, self.h, self.title, self.parts = w, h, title, []

    def add(self, s):
        self.parts.append(s)

    def line(self, x1, y1, x2, y2, c=INK, w=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{c}" stroke-width="{w}"{d}/>')

    def path(self, pts, c=INK, w=1.5, fill="none", dash=None, close=False, opacity=None):
        d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in pts) + (" Z" if close else "")
        extra = (f' stroke-dasharray="{dash}"' if dash else "") + (f' fill-opacity="{opacity}"' if opacity else "")
        self.add(f'<path d="{d}" fill="{fill}" stroke="{c}" stroke-width="{w}" stroke-linejoin="round"{extra}/>')

    def text(self, x, y, s, size=12, c=INK, anchor="start", weight="normal", rot=None, italic=False):
        t = f' transform="rotate({rot} {x:.1f} {y:.1f})"' if rot is not None else ""
        st = ' font-style="italic"' if italic else ""
        s = s.replace("&", "&amp;").replace("<", "&lt;")
        self.add(f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" fill="{c}" text-anchor="{anchor}" font-weight="{weight}"{st}{t}>{s}</text>')

    def rect(self, x, y, w, h, fill="none", c=INK, sw=1, rx=0):
        self.add(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" fill="{fill}" stroke="{c}" stroke-width="{sw}" rx="{rx}"/>')

    def circle(self, x, y, r, fill="none", c=INK, sw=1, dash=None):
        d = f' stroke-dasharray="{dash}"' if dash else ""
        self.add(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.1f}" fill="{fill}" stroke="{c}" stroke-width="{sw}"{d}/>')

    def arrow(self, x1, y1, x2, y2, c=INK, w=1.2):
        self.line(x1, y1, x2, y2, c, w)
        a = math.atan2(y2 - y1, x2 - x1)
        p = [(x2, y2), (x2 - 8 * math.cos(a - 0.35), y2 - 8 * math.sin(a - 0.35)),
             (x2 - 8 * math.cos(a + 0.35), y2 - 8 * math.sin(a + 0.35))]
        self.path(p, c=c, w=1, fill=c, close=True)

    def save(self, name):
        head = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.w} {self.h}" '
                f'font-family="{FONT}" role="img" aria-label="{self.title}">\n'
                f'<title>{self.title}</title>\n<rect width="{self.w}" height="{self.h}" fill="#ffffff"/>\n')
        foot = (f'<text x="{self.w - 8}" y="{self.h - 8}" font-size="10" fill="{INK2}" text-anchor="end">'
                f'Drawn by us, not by OWD</text>\n</svg>\n')
        (FIG / name).write_text(head + "\n".join(self.parts) + "\n" + foot, encoding="utf-8")


class Plan:
    """A plan window: world metres to the sheet, north up."""

    def __init__(self, svg, box, x0, y0, w, h):
        self.svg = svg
        xmin, ymin, xmax, ymax = box
        self.k = min(w / (xmax - xmin), h / (ymax - ymin))
        self.ox = x0 + (w - self.k * (xmax - xmin)) / 2 - self.k * xmin
        self.oy = y0 + (h + self.k * (ymax - ymin)) / 2 + self.k * ymin
        self.box, self.frame = box, (x0, y0, w, h)
        Plan.count = getattr(Plan, "count", 0) + 1
        cid = f"plan{Plan.count}"
        svg.add(f'<clipPath id="{cid}"><rect x="{x0}" y="{y0}" width="{w}" height="{h}"/></clipPath>')
        svg.add(f'<g clip-path="url(#{cid})">')
        self.open = True

    def close(self):
        if self.open:
            self.svg.add("</g>")
            self.open = False

    def p(self, q):
        return (self.ox + self.k * q[0], self.oy - self.k * q[1])

    def clip(self, pts):
        return [[self.p(q) for q in pts]]

    def draw(self, pts, **kw):
        for r in self.clip(pts):
            if len(r) > 1:
                self.svg.path(r, **kw)

    def scale_bar(self, metres):
        self.close()
        x0, y0, w, h = self.frame
        x, y = x0 + 14, y0 + h - 14
        self.svg.line(x, y, x + metres * self.k, y, INK, 2)
        self.svg.text(x + metres * self.k + 6, y + 4, f"{metres:,} m", 11)
        self.svg.text(x0 + w - 14, y0 + 20, "N ↑", 12, anchor="end")


def along(coords):
    s = [0.0]
    for a, b in zip(coords, coords[1:]):
        s.append(s[-1] + math.hypot(b[0] - a[0], b[1] - a[1]))
    return s


def point_at(coords, s_list, s):
    for i in range(len(coords) - 1):
        if s_list[i + 1] >= s:
            t = (s - s_list[i]) / max(s_list[i + 1] - s_list[i], 1e-9)
            a, b = coords[i], coords[i + 1]
            return (a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])), math.atan2(b[1] - a[1], b[0] - a[0])
    a, b = coords[-2], coords[-1]
    return tuple(b), math.atan2(b[1] - a[1], b[0] - a[0])


def geometry(doc):
    res = doc["stages"]["optimise"]["results"]
    return {
        "river": doc["inputs"]["river_polygon"]["coordinates"][0],
        "centre": doc["inputs"]["centreline"]["coordinates"],
        "baseline": res["baseline"]["coordinates"],
        "line": res["design_line"]["line"]["coordinates"],
        "weirs": [w["chainage_m"] for w in res["cascade"]["weirs"]],
        "levels": [w["pool_level_m"] for w in res["cascade"]["weirs"]],
    }


def plan_base(pl, g, facts, labels=True):
    svg = pl.svg
    for r in pl.clip(g["river"]):
        svg.path(r, c=TEAL, w=0.8, fill=WATER)
    rise = facts["rise_at"]
    for rad, dash in ((90, None), (180, "4 3")):
        x, y = pl.p(rise)
        svg.circle(x, y, rad * pl.k, c=OCHRE, sw=1, dash=dash)
    hx, hy = pl.p(facts["hollow_at"])
    svg.circle(hx, hy, 150 * pl.k, c=GREEN, sw=1, dash="2 3")
    if labels:
        x, y = pl.p(rise)
        svg.text(x, y + 4, "rise", 11, OCHRE, "middle")
        svg.text(hx, hy - 150 * pl.k - 4, "low bank", 11, GREEN, "middle")


def weirs_on(pl, g, size=11):
    s = along(g["baseline"])
    for c, lvl in zip(g["weirs"], g["levels"]):
        q, th = point_at(g["baseline"], s, c)
        x, y = pl.p(q)
        nx, ny = -math.sin(th), math.cos(th)
        pl.svg.line(x - 14 * nx, y + 14 * ny, x + 14 * nx, y - 14 * ny, INK, 3)
        pl.svg.text(x + 18 * nx, y - 18 * ny + 4, f"weir {c:,.0f} m", size, anchor="middle")


def legend(svg, x, y, items):
    for i, (label, c, dash, w) in enumerate(items):
        yy = y + 18 * i
        svg.line(x, yy, x + 28, yy, c, w, dash)
        svg.text(x + 36, yy + 4, label, 11)


def fig_pipeline():
    svg = Svg(960, 300, "The chain of stages and what passes between them")
    boxes = [("Sizing", "vessel + rule set"), ("Alignment", "rolling circle"), ("Weir search", "sites, lifts, levels"),
             ("Pool search", "line in each pool"), ("Volumes", "sections, m³"), ("Checks", "pass or fail")]
    passes = ["B, D, R_min", "baseline", "pools, levels", "design line", "areas, volumes"]
    x, w, y, h, gap = 20, 128, 110, 64, 30
    for i, (a, b) in enumerate(boxes):
        bx = x + i * (w + gap)
        fill = "#f8f2e3" if i not in (2, 3) else "#f4d9d6" if False else "#e8dcbd"
        svg.rect(bx, y, w, h, fill=fill, rx=4)
        svg.text(bx + w / 2, y + 27, a, 14, anchor="middle", weight="600")
        svg.text(bx + w / 2, y + 46, b, 11, INK2, "middle")
        if i < len(boxes) - 1:
            svg.arrow(bx + w, y + h / 2, bx + w + gap, y + h / 2)
            svg.text(bx + w + gap / 2, y - 8 if i % 2 == 0 else y + h + 18, passes[i], 11, RED, "middle")
            svg.line(bx + w + gap / 2, y - 4 if i % 2 == 0 else y + h + 6, bx + w + gap / 2, y + h / 2, RED, 0.6, "2 2")
    ins = [(0, "vessel, draught, rules"), (1, "river centreline, polygon"), (2, "DEM, sites, lifts, weights"),
           (3, "DEM, river polygon, weights"), (4, "DEM, pool levels")]
    for i, t in ins:
        bx = x + i * (w + gap) + w / 2
        svg.arrow(bx, 40, bx, y)
        svg.text(bx, 32, t, 11, INK2, "middle")
    svg.text(x + 2 * (w + gap) + w + gap / 2, y + h + 58, "the optimise stage (owd optimise): the two searches of this document", 12, anchor="middle")
    svg.line(x + 2 * (w + gap), y + h + 40, x + 3 * (w + gap) + w, y + h + 40, INK, 1)
    svg.text(20, 286, "Top: what the engineer gives. Red: what one stage hands to the next.", 11, INK2)
    svg.save("pipeline.svg")


def bbox(*lists, pad=120):
    xs = [p[0] for l in lists for p in l]
    ys = [p[1] for l in lists for p in l]
    return (min(xs) - pad, min(ys) - pad, max(xs) + pad, max(ys) + pad)


def fig_example_plan(base, facts):
    g = geometry(base)
    svg = Svg(960, 400, "Plan of the example river with the baseline, the design line and the weirs")
    pl = Plan(svg, bbox(g["river"], pad=260), 10, 10, 940, 350)
    plan_base(pl, g, facts)
    pl.draw(g["baseline"], c=TEAL, w=1.6, dash="6 3")
    pl.draw(g["line"], c=RED, w=1.8)
    weirs_on(pl, g)
    s = along(g["baseline"])
    for c in range(0, int(s[-1]), 1000):
        q, _ = point_at(g["baseline"], s, c)
        x, y = pl.p(q)
        svg.circle(x, y, 2.2, fill=INK)
    q, _ = point_at(g["baseline"], s, 0)
    x, y = pl.p(q)
    svg.text(x, y + 20, "0 m (downstream end)", 11, anchor="start")
    pl.scale_bar(1000)
    legend(svg, 330, 372, [("river polygon (60 m wide)", TEAL, None, 4)])
    legend(svg, 530, 372, [("baseline: rolling circle", TEAL, "6 3", 1.6)])
    legend(svg, 720, 372, [("design line after the pool search", RED, None, 1.8)])
    svg.save("example-plan.svg")


def loop_box(g, lo, hi, pad=160):
    s = along(g["baseline"])
    pts = [point_at(g["baseline"], s, c)[0] for c in range(int(lo), int(hi) + 1, 20)]
    return bbox(pts, pad=pad)


def fig_pool_plan(base, facts):
    g = geometry(base)
    svg = Svg(960, 470, "The pool from 3,000 m to 4,000 m in plan: the searched line against the baseline")
    pl = Plan(svg, loop_box(g, 2900, 4100, 230), 10, 10, 940, 420)
    plan_base(pl, g, facts)
    pl.draw(g["centre"], c=TEAL, w=0.6, dash="1 3")
    pl.draw(g["baseline"], c=TEAL, w=1.8, dash="6 3")
    pl.draw(g["line"], c=RED, w=2)
    weirs_on(pl, g)
    pl.scale_bar(200)
    legend(svg, 250, 446, [("baseline: rolling circle", TEAL, "6 3", 1.8)])
    legend(svg, 450, 446, [("searched line (used)", RED, None, 2)])
    legend(svg, 640, 446, [("rise: 90 m and 180 m rings", OCHRE, None, 1)])
    svg.save("pool-plan.svg")


def fig_section(base, numbers):
    sec = base["stages"]["volumes"]["results"]["sections"]["stations"]
    ch = numbers["station.chainage_m"]["value"]
    st = min((s for s in sec if s.get("profile")), key=lambda s: abs(s["chainage_m"] - ch))
    p = st["profile"]
    hb, wl, depth = st["half_width_m"], st["water_level_m"], st["depth_m"]
    grade = wl - depth
    svg = Svg(960, 430, "The section at one station: ground, cut face, dredge and dry cut")
    xr = 80.0
    zlo, zhi = grade - 1.5, max(z for o, z in zip(p["offsets_m"], p["terrain_m"]) if abs(o) <= 80.0) + 1.0
    X = lambda o: 70 + (o + xr) / (2 * xr) * 860
    Z = lambda z: 370 - (z - zlo) / (zhi - zlo) * 320
    import csv
    rows = list(csv.DictReader(open(HERE / "station.csv", encoding="utf-8")))
    for r in rows:
        o, z, cut = float(r["offset_m"]), float(r["ground_m"]), float(r["cut_m"])
        if abs(o) <= xr and cut > 0:
            f = float(r["face_m"])
            svg.rect(X(o - 0.5), Z(z), X(o + 0.5) - X(o - 0.5), Z(f) - Z(z), fill=TEAL_W if r["in_river"] == "1" else OCHRE_W, c="none", sw=0)
    for z in range(int(math.ceil(zlo)), int(zhi) + 1):
        svg.line(70, Z(z), 930, Z(z), RULE, 0.5)
        svg.text(62, Z(z) + 4, f"{z}", 11, anchor="end")
    svg.text(22, 200, "level, m", 11, rot=-90, anchor="middle")
    for o in range(-80, 81, 20):
        svg.line(X(o), 370, X(o), 375, INK)
        svg.text(X(o), 390, f"{o}", 11, anchor="middle")
    svg.text(500, 408, "offset from the design line, m (left bank negative)", 11, anchor="middle")
    svg.line(70, Z(wl), 930, Z(wl), TEAL, 1.2, "8 4")
    svg.text(926, Z(wl) - 5, f"pool level L = {wl:.2f} m", 11, TEAL, "end")
    ground = [(X(o), Z(z)) for o, z in zip(p["offsets_m"], p["terrain_m"]) if abs(o) <= xr]
    svg.path(ground, c=INK, w=1.6)
    face = [(X(float(r["offset_m"])), Z(float(r["face_m"]))) for r in rows
            if r["face_m"] and abs(float(r["offset_m"])) <= xr and float(r["face_m"]) <= zhi]
    svg.path(face, c=RED, w=1.8)
    svg.text(X(0), Z(grade) + 16, f"bed level z_b = L − D = {grade:.2f} m", 11, RED, "middle")
    svg.line(X(-hb), Z(grade) - 4, X(-hb), Z(grade) + 24, RED, 0.6, "2 2")
    svg.line(X(hb), Z(grade) - 4, X(hb), Z(grade) + 24, RED, 0.6, "2 2")
    svg.text(X(0), Z(grade) + 32, f"bottom width 2b = {2 * hb:.1f} m", 11, RED, "middle")
    wet = [float(r["offset_m"]) for r in rows if r["in_river"] == "1" and abs(float(r["offset_m"])) <= xr]
    if wet:
        svg.line(X(min(wet)), 44, X(max(wet)), 44, TEAL, 4)
        svg.text((X(min(wet)) + X(max(wet))) / 2, 38, "inside the river polygon", 11, TEAL, "middle")
    svg.text(X(-74), Z(wl) + 16, "1 : 3 to the pool level,", 11, RED)
    svg.text(X(-74), Z(wl) + 30, "2 m berm, then 1 : 1.5", 11, RED)
    legend(svg, 80, 30, [("ground (DEM)", INK, None, 1.6)])
    legend(svg, 80, 48, [("cut face (Cut Model A)", RED, None, 1.8)])
    svg.rect(290, 22, 14, 12, fill=TEAL_W, c="none")
    svg.text(310, 33, f"dredge {st['dredge_area_m2']:.2f} m²", 11)
    svg.rect(290, 40, 14, 12, fill=OCHRE_W, c="none")
    svg.text(310, 51, f"dry cut {st['excav_area_m2']:.2f} m²", 11)
    svg.text(930, 30, f"station {st['chainage_m']:,.1f} m", 12, anchor="end", weight="600")
    svg.save("section.svg")


def arc(x, y, th, r, sgn, length, n=24):
    pts = [(x, y)]
    for i in range(1, n + 1):
        s = length * i / n
        if sgn == 0:
            pts.append((x + s * math.cos(th), y + s * math.sin(th)))
        else:
            k = sgn / r
            lx, ly = math.sin(k * s) / k, (1 - math.cos(k * s)) / k
            pts.append((x + math.cos(th) * lx - math.sin(th) * ly, y + math.sin(th) * lx + math.cos(th) * ly))
    return pts


def fig_primitives(numbers):
    rmin = numbers["size.min_bend_radius_m"]["value"]
    svg = Svg(960, 420, "The seven moves of the lattice from one state")
    k = 1.7
    ox, oy = 150, 210
    P = lambda q: (ox + k * q[0], oy - k * q[1])
    bs = 2 * math.pi / 16
    for i in range(16):
        a = i * bs
        svg.line(ox, oy, ox + 60 * math.cos(a), oy - 60 * math.sin(a), RULE, 1)
    svg.text(ox, oy - 72, "16 headings, 22.5° apart", 11, INK2, "middle")
    cols = [RED, OCHRE, GREEN]
    for m, c in zip((1.0, 1.5, 3.0), cols):
        r = rmin * m
        for sgn in (1, -1):
            pts = [P(q) for q in arc(0, 0, 0, r, sgn, r * bs)]
            svg.path(pts, c=c, w=2)
            svg.circle(pts[-1][0], pts[-1][1], 3, fill=c, c=c)
        up = m != 1.0
        end = P(arc(0, 0, 0, r, 1 if up else -1, r * bs)[-1])
        svg.text(end[0] + 8, end[1] + (-6 if up else 16), f"R = {m:g} R_min = {r:,.1f} m, arc {r * bs:,.1f} m", 11, c)
    st = [P(q) for q in arc(0, 0, 0, 1, 0, 24)]
    svg.path(st, c=INK, w=3)
    svg.circle(st[-1][0], st[-1][1], 3, fill=INK)
    svg.line(st[-1][0], st[-1][1], ox + 30, oy + 118, INK, 0.5, "2 2")
    svg.text(ox + 30, oy + 132, "straight, 24 m (twice the pitch)", 11, anchor="middle")
    svg.circle(ox, oy, 4, fill=INK)
    svg.text(ox - 10, oy - 10, "state (x, y, heading)", 11, anchor="end")
    gx, gy = 720, 300
    for i in range(6):
        svg.line(gx + i * 20.4, gy, gx + i * 20.4, gy + 102, RULE, 1)
        svg.line(gx, gy + i * 20.4, gx + 102, gy + i * 20.4, RULE, 1)
    svg.text(gx + 51, gy - 8, "positions are snapped to a 12 m grid", 11, INK2, "middle")
    svg.text(40, 30, "Each arc turns the heading by exactly one step of 22.5°. No arc is tighter than R_min.", 12)
    svg.text(40, 48, "After a left arc the next move is a left arc or the straight, never a right arc.", 12)
    svg.save("primitives.svg")


def fig_long_section(base, other=None, name="long-section.svg", labels=("base run", "")):
    svg = Svg(960, 420, "Long section: the bed on the line, the pool levels and the weirs")
    def series(doc):
        st = doc["stages"]["volumes"]["results"]["sections"]["stations"]
        return ([s["chainage_m"] for s in st], [s.get("bed_m") for s in st], [s["water_level_m"] for s in st], [s["invert_m"] for s in st])
    ch, bed, wl, inv = series(base)
    smax = ch[-1]
    zlo, zhi = -5.0, 10.0
    X = lambda s: 60 + s / smax * 880
    Z = lambda z: 370 - (z - zlo) / (zhi - zlo) * 330
    for z in range(-4, 11, 2):
        svg.line(60, Z(z), 940, Z(z), RULE, 0.5)
        svg.text(52, Z(z) + 4, f"{z}", 11, anchor="end")
    for s in range(0, int(smax) + 1, 1000):
        svg.line(X(s), 370, X(s), 375, INK)
        svg.text(X(s), 390, f"{s:,}", 11, anchor="middle")
    svg.text(500, 408, "chainage along the design line, m (upstream to the right)", 11, anchor="middle")
    svg.text(18, 200, "level, m", 11, rot=-90, anchor="middle")
    svg.path([(X(s), Z(b)) for s, b in zip(ch, bed) if b is not None], c=INK, w=1.2)
    def steps(ch, v):
        pts = []
        for i, (s, z) in enumerate(zip(ch, v)):
            if i and v[i - 1] != z:
                pts.append((X(s), Z(v[i - 1])))
            pts.append((X(s), Z(z)))
        return pts
    if other is not None:
        ch2, _, wl2, inv2 = series(other)
        svg.path(steps(ch2, wl2), c=OCHRE, w=2, dash="6 3")
        svg.path(steps(ch2, inv2), c=OCHRE, w=1.2, dash="6 3")
    svg.path(steps(ch, wl), c=TEAL, w=2.2)
    svg.path(steps(ch, inv), c=RED, w=1.6)
    for w in base["stages"]["optimise"]["results"]["cascade"]["weirs"]:
        # weir chainages are on the baseline; the pool levels step where the stations change level
        pass
    prev = wl[0]
    for s, z in zip(ch, wl):
        if z != prev:
            svg.line(X(s), Z(prev) + 6, X(s), Z(z) - 6, INK, 3)
            svg.text(X(s), Z(z) - 12, f"+{z - prev:.1f} m", 11, anchor="middle")
            prev = z
    items = [("ground or river bed on the line (DEM)", INK, None, 1.2), (f"pool level, {labels[0]}", TEAL, None, 2.2), ("bed level = pool level − depth", RED, None, 1.6)]
    if other is not None:
        items.append((f"pool level and bed level, {labels[1]}", OCHRE, "6 3", 2))
    legend(svg, 80, 34, items)
    svg.save(name)


def bars(svg, x, y, w, h, groups, series, colours, vmax, unit, fmt="{:,.0f}"):
    n, m = len(groups), len(series)
    gw = w / n
    bw = gw * 0.7 / m
    for t in range(5):
        v = vmax * t / 4
        yy = y + h - h * t / 4
        svg.line(x, yy, x + w, yy, RULE, 0.5)
        svg.text(x - 6, yy + 4, fmt.format(v), 11, anchor="end")
    for i, (label, values, note) in enumerate(groups):
        for j, v in enumerate(values):
            bh = h * v / vmax
            bx = x + i * gw + gw * 0.15 + j * bw
            svg.rect(bx, y + h - bh, bw - 2, bh, fill=colours[j], c=INK, sw=0.6)
            svg.text(bx + bw / 2 - 1, y + h - bh - 4, fmt.format(v), 10, anchor="middle")
        svg.text(x + i * gw + gw / 2, y + h + 16, label, 11, anchor="middle")
        if note:
            svg.text(x + i * gw + gw / 2, y + h + 31, note, 10, INK2, "middle")
    for j, s in enumerate(series):
        svg.rect(x + 10 + j * 190, y - 26, 14, 12, fill=colours[j], c=INK, sw=0.6)
        svg.text(x + 30 + j * 190, y - 15, s, 11)
    svg.text(x - 44, y + h / 2, unit, 11, rot=-90, anchor="middle")


def pool_rows(doc):
    return doc["stages"]["optimise"]["results"]["design_line"]["details"]["pools"]


def fig_pool_costs(base):
    svg = Svg(960, 400, "Search cost of each pool: the searched line against the baseline")
    groups = []
    for p in pool_rows(base):
        note = "searched line used" if p["used"] else "baseline kept"
        groups.append((f"{p['start_m']:,.0f} to {p['end_m']:,.0f} m", [p["cost"], p["baseline_cost"]], note))
    bars(svg, 90, 60, 840, 270, groups, ["searched line", "baseline (rolling circle)"], [RED_W, TEAL_W], 140000, "search cost (m³ at weight 1)")
    svg.save("pool-costs.svg")


def fig_length(runs):
    svg = Svg(960, 400, "Cost per metre: dredge, dry cut and pools re-routed")
    groups = []
    for label, name in (("0 (base)", "base"), ("300 per m", "length-300"), ("1,000 per m", "length-1000")):
        v = runs[name]["stages"]["volumes"]["results"]
        d = runs[name]["stages"]["optimise"]["results"]["design_line"]
        n = d["details"]["n_replaced"]
        from_len = sum(math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(d["line"]["coordinates"], d["line"]["coordinates"][1:]))
        groups.append((f"cost per metre {label}", [v["dredge_m3"], v["excav_m3"]], f"{n} of 5 pools re-routed; line {from_len:,.0f} m"))
    bars(svg, 100, 60, 820, 270, groups, ["dredge, m³", "dry cut, m³"], [TEAL_W, OCHRE_W], 600000, "volume, m³")
    svg.save("sensitivity-length.svg")


def fig_two_lines(runs, facts, a, b, lo, hi, name, title, la, lb, pad=200):
    ga, gb = geometry(runs[a]), geometry(runs[b])
    svg = Svg(960, 470, title)
    pl = Plan(svg, loop_box(ga, lo, hi, pad), 10, 10, 940, 420)
    plan_base(pl, ga, facts)
    pl.draw(ga["baseline"], c=TEAL, w=1.2, dash="2 3")
    pl.draw(ga["line"], c=RED, w=2)
    pl.draw(gb["line"], c=PLUM, w=2, dash="7 3")
    weirs_on(pl, ga)
    pl.scale_bar(200)
    legend(svg, 180, 446, [("baseline", TEAL, "2 3", 1.2)])
    legend(svg, 300, 446, [(la, RED, None, 2)])
    legend(svg, 560, 446, [(lb, PLUM, "7 3", 2)])
    svg.save(name)


def fig_bank(runs):
    svg = Svg(960, 400, "Each pool's greatest height above bank-full, with and without the limit")
    def rows(n):
        return runs[n]["stages"]["optimise"]["results"]["cascade"]["flood"]["pools"]
    x, y, w, h = 90, 60, 840, 270
    lo, hi = -3.0, 1.0
    Z = lambda v: y + h - (v - lo) / (hi - lo) * h
    for t in range(-3, 2):
        svg.line(x, Z(t), x + w, Z(t), RULE if t else INK, 0.5 if t else 1.2)
        svg.text(x - 6, Z(t) + 4, f"{t:+d}" if t else "0", 11, anchor="end")
    svg.text(x + 6, Z(0) - 6, "bank-full (an estimate); the limit of the second run is 0 m above it", 11)
    gw = w / 5
    for i, (pa, pb) in enumerate(zip(rows("base"), rows("bank-0"))):
        for j, (p, c) in enumerate(((pa, TEAL_W), (pb, OCHRE_W))):
            v = p["max_above_bank_m"]
            bx = x + i * gw + gw * 0.2 + j * gw * 0.3
            top, bot = min(Z(v), Z(0)), max(Z(v), Z(0))
            svg.rect(bx, top, gw * 0.3 - 2, bot - top, fill=c, c=INK, sw=0.6)
            svg.text(bx + gw * 0.15 - 1, (top - 4) if v > 0 else (bot + 12), f"{v:+.2f}", 10, anchor="middle")
        svg.text(x + i * gw + gw / 2, y + h + 16, f"{pa['start_m']:,.0f} to {pa['end_m']:,.0f} m", 11, anchor="middle")
        svg.text(x + i * gw + gw / 2, y + h + 31, f"level {pa['level_m']:.1f} m → {pb['level_m']:.1f} m", 10, INK2, "middle")
    svg.rect(x + 10, y - 26, 14, 12, fill=TEAL_W, c=INK, sw=0.6)
    svg.text(x + 30, y - 15, "base run (no limit)", 11)
    svg.rect(x + 200, y - 26, 14, 12, fill=OCHRE_W, c=INK, sw=0.6)
    svg.text(x + 220, y - 15, "limit 0 m above bank-full", 11)
    svg.text(x - 44, y + h / 2, "pool level above bank-full, m", 11, rot=-90, anchor="middle")
    svg.save("sensitivity-bank.svg")


def fig_flood(runs):
    svg = Svg(960, 400, "Flood cost: flooded land per pool at three prices per hectare")
    names = [("0 (base)", "base"), ("3,000 per ha", "flood-3000"), ("20,000 per ha", "flood-20000")]
    pools = runs["base"]["stages"]["optimise"]["results"]["cascade"]["flood"]["pools"]
    groups = []
    for i, p in enumerate(pools):
        vals, lv = [], []
        for _, n in names:
            q = runs[n]["stages"]["optimise"]["results"]["cascade"]["flood"]["pools"][i]
            vals.append(q["flooded_ha"])
            lv.append(f"{q['level_m']:.1f}")
        groups.append((f"{p['start_m']:,.0f} to {p['end_m']:,.0f} m", vals, "levels " + " / ".join(lv) + " m"))
    bars(svg, 90, 60, 840, 270, groups, [f"flood cost {l}" for l, _ in names], [TEAL_W, GREEN_W, OCHRE_W], 8.0, "flooded land, ha", "{:.2f}")
    svg.save("sensitivity-flood.svg")


def closing_path(a, b, r):
    """The shortest arc-straight-arc path from pose a to pose b (x, y, heading), as the engine
    builds it: four pairs of turns, total turn at most half a circle."""
    best = None
    for s1, s2 in ((1, 1), (-1, -1), (1, -1), (-1, 1)):
        n0, n1 = (-math.sin(a[2]), math.cos(a[2])), (-math.sin(b[2]), math.cos(b[2]))
        c0 = (a[0] + s1 * r * n0[0], a[1] + s1 * r * n0[1])
        c1 = (b[0] + s2 * r * n1[0], b[1] + s2 * r * n1[1])
        dx, dy = c1[0] - c0[0], c1[1] - c0[1]
        dd = math.hypot(dx, dy)
        if s1 == s2:
            ln, phi = dd, math.atan2(dy, dx)
        else:
            if dd < 2 * r:
                continue
            ln = math.sqrt(dd * dd - 4 * r * r)
            phi = math.atan2(dy, dx) + math.atan2(2 * s1 * r, ln)
        a1 = (s1 * (phi - a[2])) % (2 * math.pi)
        a2 = (s2 * (b[2] - phi)) % (2 * math.pi)
        if a1 + a2 > math.pi + 1e-9:
            continue
        total = r * (a1 + a2) + ln
        if best is None or total < best[0]:
            best = (total, (s1, r * a1), ln, (s2, r * a2), c0, c1)
    return best


def fig_closing():
    svg = Svg(960, 360, "The closing path: arc, straight, arc onto the end pose")
    r = 100.0
    a = (150.0, 60.0, math.radians(75))
    b = (800.0, 150.0, math.radians(55))
    _, (s1, l1), ln, (s2, l2), c0, c1 = closing_path(a, b, r)
    P = lambda q: (q[0] + 20, 320 - q[1])
    for c in (c0, c1):
        x, y = P(c)
        svg.circle(x, y, r, c=RULE, sw=1, dash="3 3")
        svg.circle(x, y, 2, fill=INK2, c=INK2)
        svg.text(x, y + 16, "R_min", 11, INK2, "middle")
    first = arc(a[0], a[1], a[2], r, s1, l1)
    th1 = a[2] + s1 * l1 / r
    mid = arc(first[-1][0], first[-1][1], th1, 1, 0, ln, 2)
    last = arc(mid[-1][0], mid[-1][1], th1, r, s2, l2)
    svg.path([P(q) for q in first + mid + last], c=RED, w=2.4)
    for q in (first[-1], mid[-1]):
        x, y = P(q)
        svg.circle(x, y, 3, fill=RED, c=RED)
    for p, lab, end in ((a, "state the search has reached", "start"), (b, "end pose of the pool: point and heading", "end")):
        x, y = P(p)
        svg.circle(x, y, 4, fill=INK)
        svg.arrow(x, y, x + 50 * math.cos(p[2]), y - 50 * math.sin(p[2]))
        svg.text(x + (14 if end == "start" else -14), y + 5, lab, 11, anchor=end)
    m = P(mid[1])
    svg.text(m[0], m[1] - 12, "straight, tangent to both circles", 11, RED, "middle")
    svg.text(40, 26, "Tried from every state within 3 R_min of the end pose. Both arcs have radius R_min. Total turn at most 180°.", 12)
    svg.save("closing-path.svg")


def fig_shift():
    svg = Svg(960, 380, "The sideways shift: the line is the seed line plus an offset d(s)")
    def seed(t):
        return (80 + 800 * t, 250 - 120 * math.sin(math.pi * t) + 30 * math.sin(3 * math.pi * t))
    def d(t):
        return 46 * math.sin(math.pi * t) ** 2 * math.cos(1.4 * math.pi * t)
    n = 200
    s_pts, o_pts = [], []
    for i in range(n + 1):
        t = i / n
        p, q = seed(t), seed(min(t + 1e-4, 1.0001))
        th = math.atan2(q[1] - p[1], q[0] - p[0])
        nx, ny = math.sin(th), -math.cos(th)
        s_pts.append(p)
        o_pts.append((p[0] + d(t) * nx, p[1] + d(t) * ny))
    band = [(p[0] + 60 * (o[0] - p[0]) / (abs(d(i / n)) + 1e-9) * 0, p[1]) for i, (p, o) in enumerate(zip(s_pts, o_pts))]
    svg.path(s_pts, c=TEAL, w=1.8, dash="6 3")
    svg.path(o_pts, c=RED, w=2.2)
    for t in (0.2, 0.35, 0.5, 0.65, 0.8):
        i = int(t * n)
        p, o = s_pts[i], o_pts[i]
        svg.line(p[0], p[1], o[0], o[1], INK, 0.8)
        svg.circle(p[0], p[1], 2.5, fill=TEAL, c=TEAL)
        svg.circle(o[0], o[1], 2.5, fill=RED, c=RED)
    i = int(0.65 * n)
    p, o = s_pts[i], o_pts[i]
    svg.text((p[0] + o[0]) / 2 + 8, (p[1] + o[1]) / 2 + 4, "d(s)", 12, italic=True)
    i = int(0.2 * n)
    p, o = s_pts[i], o_pts[i]
    q = s_pts[i + 1]
    th = math.atan2(q[1] - p[1], q[0] - p[0])
    svg.arrow(p[0], p[1], p[0] + 60 * math.cos(th), p[1] + 60 * math.sin(th), TEAL)
    svg.text(p[0] + 66 * math.cos(th), p[1] + 66 * math.sin(th) + 14, "T (along the seed)", 11, TEAL)
    svg.arrow(p[0], p[1], p[0] + 60 * math.sin(th), p[1] - 60 * math.cos(th), INK)
    svg.text(p[0] + 64 * math.sin(th) + 4, p[1] - 64 * math.cos(th), "N (to the left)", 11)
    legend(svg, 80, 40, [("seed line r₀(s): arcs and straights, curvature k₀(s)", TEAL, "6 3", 1.8), ("shifted line r(s) = r₀(s) + d(s) N(s), curvature k(s)", RED, None, 2.2)])
    svg.text(80, 350, "The offsets d at each station are the unknowns. The limit is on k(s), the curvature of the red line, not of the seed.", 12)
    svg.save("shift.svg")


def main():
    FIG.mkdir(exist_ok=True)
    numbers = json.loads((HERE / "numbers.json").read_text(encoding="utf-8"))
    facts = json.loads((HERE / "inputs" / "facts.json").read_text(encoding="utf-8"))
    spec = json.loads((HERE / "runs.json").read_text(encoding="utf-8"))
    runs = {n: load(n) for n in spec["runs"]}
    base = runs["base"]
    fig_pipeline()
    fig_example_plan(base, facts)
    fig_section(base, numbers)
    fig_primitives(numbers)
    fig_pool_plan(base, facts)
    fig_long_section(base)
    fig_pool_costs(base)
    fig_length(runs)
    fig_two_lines(runs, facts, "base", "length-1000", 900, 2100, "sensitivity-length-plan.svg",
                  "Cost per metre 1,000: the pool from 1,000 m to 2,000 m cuts a corner", "base run", "cost per metre 1,000", 260)
    fig_two_lines(runs, facts, "base", "land", 2900, 4100, "sensitivity-land.svg",
                  "Land cost 3.25 m³ per m²: the line in the pool from 3,000 m to 4,000 m", "base run", "land cost 3.25 m³ per m²", 230)
    fig_flood(runs)
    fig_long_section(base, runs["flood-20000"], "sensitivity-flood-levels.svg", ("base run", "flood cost 20,000 per ha"))
    fig_bank(runs)
    fig_two_lines(runs, facts, "base", "free-seams-0", 1900, 4100, "sensitivity-free-seams.svg",
                  "Free seams with no weir straight: the line through the weir site at 3,000 m", "base run (pinned at every weir site)", "free seams, weir straight 0", 230)
    fig_closing()
    fig_shift()
    print(f"figures written to {FIG}")


if __name__ == "__main__":
    main()
