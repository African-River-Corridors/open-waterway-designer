"""Run the worked example of docs/optimisation and write its numbers.

    cargo build --release -p owd-cli
    python3 docs/optimisation/example/run.py [--owd PATH] [--only RUN]

Reads `runs.json` (the commands of every run). For each run it makes `out/<run>.owd` (not kept in
git), keeps what the tool printed in `runs/<run>.txt`, and writes `numbers.json` (every number the
document quotes) and `station.csv` (the section of one station, sample by sample). Then it draws the
figures (`figures.py`). Standard library only.

`--owd` names the `owd` binary (default: the release build of this checkout, else `owd` on PATH).
"""
import argparse
import csv
import json
import math
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
OUT = HERE / "out"
RUNS = HERE / "runs"
STATION_NEAR_M = 3500.0  # the station worked by hand: the one that keeps its profile nearest this


def find_owd(arg):
    if arg:
        return arg
    target = os.environ.get("CARGO_TARGET_DIR", str(REPO / "target"))
    built = Path(target) / "release" / "owd"
    return str(built) if built.exists() else "owd"


def fill(args, doc):
    subs = {"{doc}": str(doc), "{inputs}": str(HERE / "inputs"), "{dem}": str(OUT / "dem.tif")}
    out = []
    for a in args:
        for k, v in subs.items():
            a = a.replace(k, v)
        out.append(a)
    return out


def shown(args, name):
    """The command as the document prints it: short paths."""
    subs = {"{doc}": f"{name}.owd", "{inputs}": "inputs", "{dem}": "dem.tif"}
    out = []
    for a in args:
        for k, v in subs.items():
            a = a.replace(k, v)
        out.append(a)
    return "owd " + " ".join(out)


def call(owd, args, log):
    p = subprocess.run([owd] + args, capture_output=True, text=True)
    text = p.stdout + p.stderr
    log.append(text)
    if p.returncode not in (0, 1):
        sys.exit(f"owd {' '.join(args)} failed ({p.returncode}):\n{text}")


def run_one(owd, spec, name, base_doc):
    doc = OUT / f"{name}.owd"
    shutil.copy(base_doc, doc)
    log = [f"$ {shown(spec['optimise'] + spec['runs'][name], name)}\n"]
    call(owd, fill(spec["optimise"] + spec["runs"][name], doc), log)
    log.append(f"$ {shown(spec['volumes'], name)}\n")
    call(owd, fill(spec["volumes"], doc), log)
    home = str(OUT) + "/"
    (RUNS / f"{name}.txt").write_text("".join(log).replace(home, ""), encoding="utf-8")


def setup(owd, spec):
    OUT.mkdir(exist_ok=True)
    RUNS.mkdir(exist_ok=True)
    shutil.copy(HERE / "inputs" / "dem.tif", OUT / "dem.tif")
    doc = OUT / "example.owd"
    doc.unlink(missing_ok=True)
    log = []
    for step in spec["setup"]:
        log.append(f"$ {shown(step, 'example')}\n")
        call(owd, fill(step, doc), log)
    pairs = [f"{k}={json.dumps(v)}" for k, v in spec["settings"].items()]
    subprocess.run([sys.executable, str(HERE / "set_setting.py"), str(doc)] + pairs, check=True)
    log.append(f"$ python3 set_setting.py example.owd {' '.join(pairs)}\n")
    home = str(OUT) + "/"
    (RUNS / "setup.txt").write_text("".join(log).replace(home, ""), encoding="utf-8")
    return doc


def at(doc, pointer):
    v = doc
    for part in pointer.strip("/").split("/"):
        v = v[int(part)] if isinstance(v, list) else v[part]
    return v


def length(coords):
    return sum(math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(coords, coords[1:]))


def pins_of(doc):
    """The JSON pointers of the numbers a run is read for, as (key, pointer)."""
    o, v = "/stages/optimise/results", "/stages/volumes/results"
    pins = [(f"cascade.{k}", f"{o}/cascade/{k}") for k in ("total_cost", "reach_cost", "structure_cost")]
    res = at(doc, o)
    pins.append(("base_level_m", f"{o}/base_level_m"))
    for i, _ in enumerate(res["cascade"]["weirs"]):
        for k in ("chainage_m", "pool_level_m", "lift_m", "structure_cost"):
            pins.append((f"weir{i + 1}.{k}", f"{o}/cascade/weirs/{i}/{k}"))
    flood = res["cascade"].get("flood")
    if flood:
        for k in ("flooded_ha", "flood_cost", "cut_cost"):
            pins.append((f"flood.{k}", f"{o}/cascade/flood/{k}"))
        for i, _ in enumerate(flood["pools"]):
            for k in ("level_m", "flooded_ha", "max_above_bank_m"):
                pins.append((f"flood.pool{i + 1}.{k}", f"{o}/cascade/flood/pools/{i}/{k}"))
    line = f"{o}/design_line"
    for k in ("r_min_m", "measured_min_radius_m", "radius_check_chord_m", "radius_ok"):
        pins.append((f"line.{k}", f"{line}/checks/{k}"))
    pins.append(("line.no_self_crossing", f"{line}/checks/no_self_crossing/ok"))
    pins.append(("line.n_replaced", f"{line}/details/n_replaced"))
    details = res["design_line"]["details"]
    if "land_footprint_width_m" in details:
        pins.append(("land.footprint_width_m", f"{line}/details/land_footprint_width_m"))
    for i, p in enumerate(details["pools"]):
        for k in ("start_m", "end_m", "water_level_m", "cost", "baseline_cost", "used", "goal_closed",
                  "expansions", "land_area_m2", "baseline_land_area_m2", "line_length_m"):
            if k in p:
                pins.append((f"pool{i + 1}.{k}", f"{line}/details/pools/{i}/{k}"))
    for i, w in enumerate(details.get("weirs", [])):
        for k in ("at_m", "free", "straight_through_m", "straight_required_m"):
            if k in w:
                pins.append((f"site{i + 1}.{k}", f"{line}/details/weirs/{i}/{k}"))
    for i, j in enumerate(details["joins"]):
        for k in ("at_m", "measured_min_radius_m", "radius_ok"):
            pins.append((f"join{i + 1}.{k}", f"{line}/details/joins/{i}/{k}"))
    for k in ("dredge_m3", "excav_m3", "total_m3", "widening_m3", "cut_through_m3", "max_face_h_m", "stations"):
        pins.append((f"volumes.{k}", f"{v}/{k}"))
    return pins


def face(o, bench, hb, grade, wl, depth, slope=3.0, berm=2.0, above=1.5):
    """Cut Model A: the level of the cut face at distance o from the line; None is no cut."""
    if o <= hb:
        return grade
    r = o - hb
    if r <= slope * depth:
        return grade + r / slope
    if not bench:
        return None
    r2 = r - slope * depth
    return wl if r2 <= berm else wl + (r2 - berm) / above


def station_by_hand(st):
    """The section of one station from its stored profile, by the formulas of the document."""
    p = st["profile"]
    hb, wl, depth = st["half_width_m"], st["water_level_m"], st["depth_m"]
    grade = wl - depth
    reach = hb + 3.0 * depth
    bench = {}
    for side in (-1, 1):
        inside = [abs(o) for o, z in zip(p["offsets_m"], p["terrain_m"])
                  if (o < 0) == (side < 0) and z is not None and abs(o) <= reach and z > wl]
        bench[side] = bool(inside) and reach - min(inside) > 2.0
    rows, dredge, dry = [], 0.0, 0.0
    for o, z, wet in zip(p["offsets_m"], p["terrain_m"], p["wet"]):
        side = -1 if o < 0 else 1
        f = face(abs(o), bench[side], hb, grade, wl, depth)
        cut = max(z - f, 0.0) if (f is not None and z is not None) else 0.0
        if wet:
            dredge += cut
        else:
            dry += cut
        rows.append([o, z, int(wet), "" if f is None else round(f, 4), round(cut, 4)])
    return rows, dredge, dry, bench, reach


def collect(spec):
    numbers = {}

    def put(key, value, **how):
        numbers[key] = {"value": value, **how}

    for name in spec["runs"]:
        doc = json.loads((OUT / f"{name}.owd").read_text(encoding="utf-8"))
        for key, pointer in pins_of(doc):
            put(f"{name}.{key}", at(doc, pointer), run=name, pointer=pointer)
        res = doc["stages"]["optimise"]["results"]
        put(f"{name}.line.length_m", length(res["design_line"]["line"]["coordinates"]),
            derived="the length of the design line's polyline")
        dry = at(doc, "/stages/volumes/results/excav_m3")
        put(f"{name}.volumes.dry_change_pct", 0.0, derived="filled below")
        numbers[f"{name}.volumes.dry_change_pct"]["_dry"] = dry

        pools = res["design_line"]["details"]["pools"]
        put(f"{name}.search.final_cost", sum(p["cost"] if p["used"] else p["baseline_cost"] for p in pools),
            derived="the search cost of the design line: each pool's cost if its line is used, else its baseline's")
        put(f"{name}.search.dearer", sum(1 for p in pools if p["cost"] > p["baseline_cost"]),
            derived="the number of pools whose searched line costs more than the baseline")
        put(f"{name}.search.pools", len(pools), derived="the number of pools")
        flood = res["cascade"].get("flood")
        if flood:
            put(f"{name}.flood.worst_above_bank_m", max(p["max_above_bank_m"] for p in flood["pools"]),
                derived="the greatest height of a pool above bank-full")

    base = json.loads((OUT / "base.owd").read_text(encoding="utf-8"))
    base_dry = at(base, "/stages/volumes/results/excav_m3")
    for name in spec["runs"]:
        e = numbers[f"{name}.volumes.dry_change_pct"]
        e["value"] = 100.0 * (e.pop("_dry") / base_dry - 1.0)
        e["derived"] = "dry cut of the run over dry cut of the base run, less one, in percent"

    s = "/stages/sizing/results"
    for k in ("loa_m", "beam_m", "draught_m", "bottom_width_straight_m", "min_bend_radius_m",
              "design_depth_m", "bottom_width_at_rmin_m"):
        put(f"size.{k}", at(base, f"{s}/{k}"), run="base", pointer=f"{s}/{k}")
    a = "/stages/alignment/results"
    for k in ("measured_min_radius_m", "radius_check_chord_m", "radius_ok"):
        put(f"align.{k}", at(base, f"{a}/checks/{k}"), run="base", pointer=f"{a}/checks/{k}")
    put("align.length_m", length(at(base, f"{a}/line/coordinates")),
        derived="the length of the rolling circle's polyline")
    put("river.length_m", length(base["inputs"]["centreline"]["coordinates"]),
        derived="the length of the river centreline's polyline")
    facts = json.loads((HERE / "inputs" / "facts.json").read_text(encoding="utf-8"))
    put("river.neck_width_m", facts["neck_width_m"], derived="inputs/facts.json (make_inputs.py)")

    # The reaches of the volumes stage, and the station worked by hand.
    sec = "/stages/volumes/results/sections"
    for i, r in enumerate(at(base, f"{sec}/by_reach")):
        for k in ("dredge_m3", "excav_m3", "stations"):
            put(f"base.reach{i + 1}.{k}", r[k], run="base", pointer=f"{sec}/by_reach/{i}/{k}")
    stations = at(base, f"{sec}/stations")
    kept = [i for i, st in enumerate(stations) if st.get("profile")]
    i = min(kept, key=lambda i: abs(stations[i]["chainage_m"] - STATION_NEAR_M))
    st = stations[i]
    for k in ("chainage_m", "half_width_m", "water_level_m", "invert_m", "depth_m", "dredge_area_m2",
              "excav_area_m2", "max_face_h_m", "bench_left", "bench_right"):
        put(f"station.{k}", st[k], run="base", pointer=f"{sec}/stations/{i}/{k}")
    rows, dredge, dry, bench, reach = station_by_hand(st)
    put("station.by_hand.dredge_area_m2", dredge, derived="station.csv: the sum of the cut of the wet samples, times 1 m")
    put("station.by_hand.dry_area_m2", dry, derived="station.csv: the sum of the cut of the dry samples, times 1 m")
    put("station.by_hand.slope_reach_m", reach, derived="half-width + 3 x depth")
    put("station.by_hand.samples", len(rows), derived="station.csv: the number of rows")
    put("station.by_hand.cut_samples", sum(1 for r in rows if r[4] > 0), derived="station.csv: rows with a cut")
    # The area of a slice, for the volume worked by hand: this station's areas times the spacing.
    put("station.slice_m3", (st["dredge_area_m2"] + st["excav_area_m2"]) * 25.0,
        derived="(dredge + dry area of the station) x 25 m")
    for a_, b_, what in ((dredge, st["dredge_area_m2"], "dredge"), (dry, st["excav_area_m2"], "dry cut")):
        if abs(a_ - b_) > 1e-6 * max(1.0, abs(b_)):
            sys.exit(f"the station by hand gives {a_} m2 of {what}; the tool gives {b_}")
    with open(HERE / "station.csv", "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["offset_m", "ground_m", "in_river", "face_m", "cut_m"])
        w.writerows(rows)
    rmin = numbers["size.min_bend_radius_m"]["value"]
    for m in (1.0, 1.5, 3.0):
        put(f"lattice.radius_m.{m:g}", rmin * m, derived="R_min x the radius multiple")
        put(f"lattice.arc_m.{m:g}", rmin * m * 2.0 * math.pi / 16.0, derived="radius x 2 pi / 16")
    depth, hb = numbers["size.design_depth_m"]["value"], numbers["size.bottom_width_straight_m"]["value"] / 2.0
    put("size.half_width_m", hb, derived="half of the bottom width")
    put("size.top_width_m", 2.0 * (hb + 3.0 * depth), derived="bottom width + 2 x 3 x depth")
    put("size.prism_area_m2", (2.0 * hb + 3.0 * depth) * depth, derived="(bottom width + 3 x depth) x depth")
    sites = len(json.loads((HERE / "runs.json").read_text())["optimise"][5].split(","))
    put("cascade.sites", sites, derived="the candidate sites of the example")
    put("cascade.levels", int(round(8.5 / 0.5)) + 1, derived="levels 0 to 8.5 m in steps of 0.5 m")
    put("cascade.moves_bound", (sites + 1) * sites // 2 * (int(round(8.5 / 0.5)) + 1) * 4,
        derived="pairs of sites x levels x lifts")
    n_st = numbers["base.volumes.stations"]["value"]
    line_len = numbers["base.line.length_m"]["value"]
    put("base.volumes.station_spacing_m", line_len / (n_st - 1), derived="design line length / (stations - 1)")
    put("base.volumes.length_counted_m", n_st * 25.0, derived="stations x 25 m")
    put("base.volumes.length_over_pct", 100.0 * (n_st * 25.0 / line_len - 1.0), derived="stations x 25 m over the line length, less one, in percent")
    put("base.cascade.structures", sum(numbers[f"base.weir{i}.structure_cost"]["value"] for i in range(1, 5)), derived="the sum of the four structure costs")
    g = lambda k: numbers[k]["value"]
    saved = g("base.flood.flooded_ha") - g("flood-20000.flood.flooded_ha")
    put("flood-20000.cut_added_m3", g("flood-20000.flood.cut_cost") - g("base.flood.cut_cost"),
        derived="the weir search's cut at 20,000 per ha less the base run's")
    put("flood-20000.flooded_saved_ha", saved, derived="flooded land of the base run less that of the run at 20,000 per ha")
    put("flood-3000.saving_if_lowered", 3000.0 * saved, derived="3,000 x the hectares saved")
    put("flood-20000.saving_if_lowered", 20000.0 * saved, derived="20,000 x the hectares saved")
    import derivations
    for k, v in derivations.curvature_check().items():
        put(f"curvature.{k}", v, derived="derivations.py: the curvature of a shifted line, formula against finite differences")
    (HERE / "numbers.json").write_text(json.dumps(numbers, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return numbers


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--owd")
    ap.add_argument("--only", help="run one run again (the others must be in out/)")
    ap.add_argument("--no-figures", action="store_true")
    ap.add_argument("--collect", action="store_true", help="read out/ again; run nothing")
    args = ap.parse_args()
    owd = find_owd(args.owd)
    spec = json.loads((HERE / "runs.json").read_text(encoding="utf-8"))
    base_doc = OUT / "example.owd" if args.collect else setup(owd, spec)
    for name in spec["runs"]:
        if not args.collect and args.only in (None, name):
            print(f"run {name} ...", flush=True)
            run_one(owd, spec, name, base_doc)
    numbers = collect(spec)
    print(f"{len(numbers)} numbers written to numbers.json")
    if not args.no_figures and (HERE / "figures.py").exists():
        subprocess.run([sys.executable, str(HERE / "figures.py")], check=True)


if __name__ == "__main__":
    main()
