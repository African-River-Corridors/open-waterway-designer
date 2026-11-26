"""Make the inputs of the worked example: a synthetic river, written from formulas.

    python3 docs/optimisation/example/make_inputs.py

Writes `inputs/centreline.geojson`, `inputs/river-polygon.geojson` and `inputs/dem.tif` beside this
file. Standard library only. No number here comes from a real river. The three files are committed,
so the example and its test never depend on this script's floating-point library; run it only to
change the river, then run `run.py` and update the document.

The river
---------
A sine-generated meander: the heading of the centreline at river chainage s is

    theta(s) = omega(s) * sin(2 pi s / M),   M = 1500 m,

with omega = 45 degrees along most of the river and rising to 110 degrees round s = 3750 m. The gentle
bends have a radius of about 300 m; the tight loop has bends of about 125 m and a narrow neck.
The river is 60 m wide. Its bed is a parabola 2.5 m deep at the centre.

The ground
----------
The natural water level rises 1 m per km of river: wl(s) = 0.001 s. The plain stands 2.5 m above
the water level of the river beside it, rises 2 mm per metre away from the river, and rises 20 mm per
metre beyond 400 m (the valley side). A rise 6 m high stands in the neck of the tight loop. A hollow
2 m deep lies beside the left bank near s = 5800 m, so the bank is low there. Levels are rounded to 1 cm, as a surveyed DEM is.
"""
import json
import math
import struct
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "inputs"

SHIFT = (500_000.0, 600_000.0)  # into UTM zone 30N, like the repo's other synthetic fixtures
EPSG = 32630
LENGTH_M = 7500.0
WAVELENGTH_M = 1500.0
OMEGA_BASE_DEG, OMEGA_TIGHT_DEG = 45.0, 110.0
TIGHT_AT_M, TIGHT_SPREAD_M = 3750.0, 450.0
WIDTH_M = 60.0
BED_DEPTH_M = 2.5
WATER_SLOPE = 0.001
PLAIN_ABOVE_WATER_M = 2.5
PLAIN_SLOPE = 0.002
VALLEY_SIDE_FROM_M, VALLEY_SIDE_SLOPE = 400.0, 0.02
RISE_HEIGHT_M, RISE_RADIUS_M = 6.0, 90.0
HOLLOW_AT_M, HOLLOW_OFFSET_M, HOLLOW_DEPTH_M, HOLLOW_RADIUS_M = 5800.0, 120.0, 2.0, 150.0
PIXEL_M = 10.0
MARGIN_M = 700.0


def omega(s):
    bump = math.exp(-((s - TIGHT_AT_M) ** 2) / (2.0 * TIGHT_SPREAD_M**2))
    return math.radians(OMEGA_BASE_DEG + (OMEGA_TIGHT_DEG - OMEGA_BASE_DEG) * bump)


def heading(s):
    return omega(s) * math.sin(2.0 * math.pi * s / WAVELENGTH_M)


def centreline(step=10.0, sub=100):
    """Points every `step` m of chainage, by the midpoint rule on `sub` parts of each step."""
    pts, x, y = [(0.0, 0.0)], 0.0, 0.0
    h = step / sub
    for i in range(int(round(LENGTH_M / step))):
        for j in range(sub):
            th = heading(i * step + (j + 0.5) * h)
            x += h * math.cos(th)
            y += h * math.sin(th)
        pts.append((x, y))
    return pts


def offset(pts, d):
    """The line `d` metres to the left of `pts` (right when negative), point by point."""
    out = []
    for i, (x, y) in enumerate(pts):
        th = heading(i * 10.0)
        out.append((x - d * math.sin(th), y + d * math.cos(th)))
    return out


def nearest(pts, p):
    """(distance, chainage) of the point of the polyline nearest to p; points are 10 m apart."""
    best = (float("inf"), 0.0)
    px, py = p
    for i in range(len(pts) - 1):
        (ax, ay), (bx, by) = pts[i], pts[i + 1]
        dx, dy = bx - ax, by - ay
        t = ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)
        t = 0.0 if t < 0.0 else 1.0 if t > 1.0 else t
        d = math.hypot(px - ax - t * dx, py - ay - t * dy)
        if d < best[0]:
            best = (d, (i + t) * 10.0)
    return best


def neck(pts):
    """The middle of the narrowest neck: the two closest points more than 600 m of river apart."""
    best = (float("inf"), None)
    for i in range(0, len(pts), 2):
        for j in range(i + 60, len(pts), 2):
            d = math.hypot(pts[i][0] - pts[j][0], pts[i][1] - pts[j][1])
            if d < best[0]:
                best = (d, (i, j))
    i, j = best[1]
    return ((pts[i][0] + pts[j][0]) / 2.0, (pts[i][1] + pts[j][1]) / 2.0), best[0], (i * 10.0, j * 10.0)


class Ground:
    def __init__(self, pts):
        self.pts = pts
        self.coarse = [(p, i * 10.0) for i, p in enumerate(pts)][::5]
        self.rise_at, self.neck_width, self.neck_chainages = neck(pts)
        i = int(HOLLOW_AT_M / 10.0)
        self.hollow_at = offset(pts, HOLLOW_OFFSET_M)[i]

    def plain_water(self, p):
        """The water level the plain at p stands above: the levels of the river round it, each
        weighted by 1 / distance^4, so it is the nearest river's level close to the river and
        blends smoothly across a neck."""
        num = den = 0.0
        for (q, s) in self.coarse:
            d2 = (p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2 + 1.0
            w = 1.0 / (d2 * d2)
            num += w * WATER_SLOPE * s
            den += w
        return num / den

    def z(self, p):
        d, s = nearest(self.pts, p)
        half = WIDTH_M / 2.0
        if d < half:
            r = d / half
            return WATER_SLOPE * s - BED_DEPTH_M * (1.0 - r * r)
        z = self.plain_water(p) + PLAIN_ABOVE_WATER_M + PLAIN_SLOPE * (d - half)
        z += VALLEY_SIDE_SLOPE * max(0.0, d - VALLEY_SIDE_FROM_M)
        r2 = (p[0] - self.rise_at[0]) ** 2 + (p[1] - self.rise_at[1]) ** 2
        z += RISE_HEIGHT_M * math.exp(-r2 / (2.0 * RISE_RADIUS_M**2))
        h2 = (p[0] - self.hollow_at[0]) ** 2 + (p[1] - self.hollow_at[1]) ** 2
        z -= HOLLOW_DEPTH_M * math.exp(-h2 / (2.0 * HOLLOW_RADIUS_M**2))
        return z


def write_geotiff(path, rows, x0, y1, pixel):
    """A float32 GeoTIFF, deflate strips of 16 rows, pixel-is-area, EPSG 32630."""
    h, w = len(rows), len(rows[0])
    rps = 16
    strips = []
    for a in range(0, h, rps):
        raw = b"".join(struct.pack(f"<{w}f", *r) for r in rows[a:a + rps])
        strips.append(zlib.compress(raw, 9))
    n = len(strips)
    entries = 13
    ifd_at = 8
    extra_at = ifd_at + 2 + 12 * entries + 4
    scale = struct.pack("<3d", pixel, pixel, 0.0)
    tie = struct.pack("<6d", 0.0, 0.0, 0.0, x0, y1, 0.0)
    keys = struct.pack("<16H", 1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, EPSG)
    off_at = extra_at + len(scale) + len(tie) + len(keys)
    cnt_at = off_at + 4 * n
    data_at = cnt_at + 4 * n
    offsets, at = [], data_at
    for s in strips:
        offsets.append(at)
        at += len(s)

    def e(tag, typ, count, value):
        if typ == 3 and count == 1:
            return struct.pack("<HHIHH", tag, typ, count, value, 0)
        return struct.pack("<HHII", tag, typ, count, value)

    ifd = struct.pack("<H", entries)
    ifd += e(256, 4, 1, w) + e(257, 4, 1, h) + e(258, 3, 1, 32) + e(259, 3, 1, 8) + e(262, 3, 1, 1)
    ifd += e(273, 4, n, off_at) + e(277, 3, 1, 1) + e(278, 4, 1, rps) + e(279, 4, n, cnt_at)
    ifd += e(339, 3, 1, 3)
    ifd += e(33550, 12, 3, extra_at) + e(33922, 12, 6, extra_at + len(scale))
    ifd += e(34735, 3, 16, extra_at + len(scale) + len(tie))
    ifd += struct.pack("<I", 0)
    with open(path, "wb") as f:
        f.write(b"II*\x00" + struct.pack("<I", ifd_at) + ifd + scale + tie + keys)
        f.write(struct.pack(f"<{n}I", *offsets) + struct.pack(f"<{n}I", *(len(s) for s in strips)))
        for s in strips:
            f.write(s)


def geojson(kind, coords):
    crs = {"type": "name", "properties": {"name": f"urn:ogc:def:crs:EPSG::{EPSG}"}}
    return json.dumps({"type": kind, "crs": crs, "coordinates": coords})


def main():
    OUT.mkdir(exist_ok=True)
    pts = centreline()
    ground = Ground(pts)
    sh = lambda p: [round(p[0] + SHIFT[0], 3), round(p[1] + SHIFT[1], 3)]
    left, right = offset(pts, WIDTH_M / 2.0), offset(pts, -WIDTH_M / 2.0)
    ring = left + right[::-1] + [left[0]]
    (OUT / "centreline.geojson").write_text(geojson("LineString", [sh(p) for p in pts]) + "\n")
    (OUT / "river-polygon.geojson").write_text(geojson("Polygon", [[sh(p) for p in ring]]) + "\n")

    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    x0 = math.floor((min(xs) - MARGIN_M) / PIXEL_M) * PIXEL_M
    y1 = math.ceil((max(ys) + MARGIN_M) / PIXEL_M) * PIXEL_M
    w = int(math.ceil((max(xs) + MARGIN_M - x0) / PIXEL_M))
    h = int(math.ceil((y1 - (min(ys) - MARGIN_M)) / PIXEL_M))
    rows = []
    for r in range(h):
        y = y1 - (r + 0.5) * PIXEL_M
        rows.append([round(ground.z((x0 + (c + 0.5) * PIXEL_M, y)), 2) for c in range(w)])
    write_geotiff(OUT / "dem.tif", rows, x0 + SHIFT[0], y1 + SHIFT[1], PIXEL_M)
    facts = {
        "river_length_m": LENGTH_M,
        "river_width_m": WIDTH_M,
        "neck_width_m": round(ground.neck_width, 1),
        "neck_between_chainages_m": list(ground.neck_chainages),
        "rise_at": sh(ground.rise_at),
        "hollow_at": sh(ground.hollow_at),
        "dem_pixels": [w, h],
        "dem_pixel_m": PIXEL_M,
        "water_slope_m_per_m": WATER_SLOPE,
    }
    (OUT / "facts.json").write_text(json.dumps(facts, indent=2) + "\n")
    print(json.dumps(facts))


if __name__ == "__main__":
    main()
