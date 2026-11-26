# Open Waterway Designer (OWD)

Navigation channel design for inland waterways, as code. Give it a river and a vessel class;
it sizes the channel to a named standard, finds an alignment that the vessel can sail, and
measures the cut and dredge volumes. Formerly `channel_designer`.

> **Screening tool.** Not for construction. No design warranty. Check every number against the
> standard and your own design basis before you rely on it.

## What it does

| Layer | Module | What you get |
|---|---|---|
| **Rules** | `owd.rules` | Channel width, bend widening, minimum bend radius and depth from the selected Rule Sets and a vessel class, on the rules engine shared with open-vessel-designer: `pianc-wg141` (qualities A, B, C) and `gb50139` (the Chinese standard GB 50139-2014). Every Rule cites its clause, table and page, and is unverified until a second reader checks it. Select more than one set and the strictest Rule governs; load a Rule Set file of your own at run time (`owd rules`, `owd size --rule-set-file`). Width methods stay pluggable (`register_width_method`, or the `owd.width_methods` entry-point group); they are design assumptions, not verified rules. |
| **Alignment** | `owd.alignment` | Centreline from two bank lines, river width, meander analysis, a tight/gentle/straight bend catalogue (exact reachability, not a closed form), rolling-circle straightening to R_min, channel limits with bend widening, and a least-cost route search that shifts the line sideways where that cuts less ground while holding R_min. |
| **Volumes** | `owd.volumes` | Cut Model A and B cross-sections (wet batter, clearance-prism gate, benched face above the waterline), a station-by-station section integrator, a raster DEM integrator, weir-cascade pool levels, and the plan excavation footprint. Quantities only — m², m³ and face heights. |
| **Terrain** | `owd.terrain` | Long profile of any line over a terrain sampler: chainage, levels, ascent and descent. Pure; it reads no files and fetches nothing. |
| **Optimise** *(optional)* | `owd.optimise` | Weir sites and pool levels by an exact search over (site, level), and the least-cost channel through each pool by an A* search over arcs that never bend tighter than R_min. Costs are weights you supply. Core dependencies only. |
| **Drawings** | `owd.draw` | Engineering drawings of a design line option, drawn by the Rust core through [`otd`](https://github.com/African-River-Corridors/open-technical-drawer), the [Drawing Format Standard](https://github.com/African-River-Corridors/drawing-format-standard) in Rust: the cross-section gallery and the typical section (ISO 7200 title block, one scale, dredge and dry cut hatched, areas, dimensions), as drawing documents, SVG, PDF and DXF. The same sheets as `owd draw`. Every sheet says "Screening — not for construction". |

Vessel classes and dimensions come from
[`vessel_designer`](https://github.com/African-River-Corridors/open-vessel-designer) (Open Vessel Designer, OVD).

Pricing is out of scope. The package measures quantities; what they cost is your call.

## Install

Python 3.12 or later.

```
pip install open-waterway-designer
```

The PyPI release is still to come ([#2](https://github.com/African-River-Corridors/open-waterway-designer/issues/2)); until it ships, install from git as shown below.

From 0.2.0 the calculations run in a Rust core (`owd._core`) that ships inside the
package. Each release has one **abi3 wheel** per platform (Windows, macOS, Linux), so
`pip install` needs **no compiler and no Rust**. The same wheel runs on every Python from 3.12.
CI proves it: the `Wheels` workflow installs the wheel on a Windows machine with no compiler on
the `PATH`, and refuses any source build (`--only-binary :all:`).

Installing from source (a checkout or the git URL) builds the core with
[maturin](https://www.maturin.rs/), so it **does** need a Rust toolchain ([rustup.rs](https://rustup.rs)):

```
pip install "open-waterway-designer @ git+https://github.com/African-River-Corridors/open-waterway-designer"
```

Your scripts do not change. The 0.2.0 functions and classes have the names, arguments and results of
0.1.x; see [CHANGELOG.md](CHANGELOG.md) for the few things that moved, and what is deprecated.

### Your own river, DEM and water level

A study starts with `owd new`, which makes an empty design. The river, the DEM and the water level all go in
with `owd import`. There is no `owd new --dem`.

```
owd new study.owd --title "My river"
owd import study.owd --polygon river.geojson --centreline centre.geojson --dem lidar.tif --water-level 38
owd bed study.owd --depth 3 --season dry
owd fits study.owd
```

- **The river.** Give the river polygon and the centreline, or the two bank lines (`--left-bank`, `--right-bank`).
  GeoJSON or a GeoPackage layer (`FILE#LAYER`), in WGS84 or WGS84 / UTM.
  If your banks are named as map sides, or run upstream, add `--downstream LON,LAT`, the point the river flows
  towards: the banks are run towards it and named looking downstream.
- **The DEM.** A GeoTIFF in the river's CRS or in WGS84. The design keeps its relative path and its SHA-256, never a
  copy, so keep the file next to the design. Without a DEM of your own, `owd fetch-dem` gets Copernicus GLO-30.
- **The water level.** `--water-level 38` is one level for the whole river, in metres. Add `--water-slope` for a
  level that falls along the river, or give `--water-table FILE` (lines of `chainage_m,level_m`). Without one,
  `owd water-level` sets a default from SWORD and SWOT.
- **A DEM with the river bed in it.** If your DEM holds surveyed bathymetry below the water level, add
  `--holds-bathymetry` to the import. `owd fits` then reads the depth from the DEM, with no `owd bed`. OWD does
  not check the DEM, and every output says the depth came from it.
- **The depth.** `owd bed` gives an assumed depth where there is no survey. With the DEM, the water level and a
  depth, `owd fits` gives the full answer (width, radius and depth).

Then size and design for the vessel you choose (below): `owd size`, `owd align`, `owd volumes`.

### Project folders

Keep one study in one folder, so the design and its DEM open in one step:

```
<project folder>/
  owd-project.json        the small text file that says what is here
  design/<name>.owd       the design file(s)
  terrain/<name>.tif      the DEM (GeoTIFF)
  inputs/                 source data: centreline, river polygon, bank lines, water levels by pool
  exports/                what OWD writes out: GeoJSON, CSV, sheets, reports
```

```
owd project init reach-a --title "Reach A screening"
owd project status reach-a
owd run reach-a
```

- `owd project init` makes the folders and `owd-project.json`. It never overwrites one.
- `owd project status` lists what the text file names, says whether each file is there, and checks the DEM
  against the hash the design recorded.
- Every command that takes a design file takes the project folder in its place.
- In the web app, **Open project** asks for the folder, opens the design and loads the DEM in one step. Save
  writes back into the folder, and exports go into `exports/`. Chrome and Edge have the folder dialog; in
  Firefox and Safari use Open and Load… as before. Nothing is uploaded.

Paths in `owd-project.json` are from the folder, with forward slashes, and may not leave it. The design still
names its DEM from its own folder (`../terrain/<name>.tif`). The full description is in
[docs/project-folder.md](docs/project-folder.md).

### Channel rules

`owd size` sizes the channel to the Rule Sets you select (`--rule-set pianc-wg141 --set-input se_quality=B`); with
two sets the strictest Rule governs. `owd waterway` marks canal and river reaches, so a Rule can apply on part of
the river. After the volumes stage the design line has a verdict and its constraint stretches
(`owd export --csv --stretches`), and `owd trace` prints the full Rule Trace. Size for any class of the shared vessel
catalogue (`--vessel CEMT-VIb-motor`), or for a vessel of your own: `owd size design.owd --custom-vessel "My barge"
--loa 140 --beam 29 --draught 4.7 --formation motor` (a convoy also takes `--abreast` and `--in-line`; GB 50139 reads
`--gb-class`). Every output labels it `Custom: My barge, 140 × 29 × 4.7 m`, and `owd fits` adds it as one more row.
Designs are document format 0.5, with one design line each (spec 0017); a format 0.4 or 0.3 file opens and is
saved as 0.5: a stage that was up to date when it was saved stays up to date, with its numbers, and the design is
turned to run from the river mouth (the lower end, by the water level or the DEM). An older file is refused and must
be rebuilt. `owd import
--mouth downstream` (or `--mouth LON,LAT`) states the river mouth, and every chainage then counts from it. See [docs/rules/channel-rules.md](docs/rules/channel-rules.md).

`owd fits` reads the river on the reference line: one smooth line in the water, made from the banks, that belongs
to no vessel (ADR 0014). `owd envelope design.owd --png envelope.png` gives the largest vessel the river allows: a
series of single-hull sizes from 10 × 2 m to 50 × 10 m at a 2 m draught under PIANC WG 141 quality C, each with its
own fairway fit, and the largest size that passes each kilometre (`--sizes`, `--draught`, `--rule-set`, `--json`,
`--csv`).

### Drawings

After the volumes stage, `owd draw design.owd --out sheets/` writes, for each sheet, the drawing document, the
normalised content, SVG, PDF and DXF. `owd draw design.owd --document` prints the drawing documents only. In Python,
`owd.draw("design.owd", out="sheets/")` gives the same files, byte for byte. Sheets are drawn by
[`otd`](https://github.com/African-River-Corridors/open-technical-drawer), the Drawing Format Standard in Rust. Every
sheet is marked "Screening — not for construction".

### Reports

`owd report design.owd --kind all --format all -o reports/` writes four reports, each as PDF, HTML and Word, through
the [Open Report Designer](https://github.com/African-River-Corridors/open-report-designer): the screening report
(with the Rule Trace as an appendix), the quantity schedule, the section pack and a run report for testers. Every
report has document control, the screening notice on every page and a "Data and limits" box. The app's Sheets view
has a **Report…** button that makes the same files. See [docs/reports.md](docs/reports.md).

Optional extras:

| Extra | Adds | For |
|---|---|---|
| `[dem]` | rasterio, scipy, affine | nothing the package needs any more: the core reads GeoTIFFs itself. Kept so existing install commands work. |
| `[dev]` | pytest | running the tests |

The `[drawings]` extra is gone: drawings are part of the package now (spec 0004).

## Worked example

A synthetic meandering river (built in code — no data files), sized for a CEMT class IV
vessel under PIANC quality B, straightened, re-routed and measured. This is
`examples/quickstart.py`; the tests run it.

```python
from owd.alignment.meanders import find_meanders
from owd.alignment.rolling_circle import rolling_circle
from owd.alignment.route import optimise_route
from owd.rules import channel_shape
from owd.synthetic import synthetic_river
from owd.volumes.cut_model import widened_hb_from_shape
from owd.volumes.sections import section_volumes

river = synthetic_river()
shape = channel_shape("CEMT_IV", draught_m=2.5, rule_sets=["pianc-wg141"],
                      set_inputs={"se_quality": "B"})

tight = find_meanders(river.centreline, max_radius=shape.min_bend_radius_m)
rolled = rolling_circle(river.centreline, shape.min_bend_radius_m, step=10.0)

route = optimise_route(
    rolled.centreline, river.terrain, river.polygon,
    grade=lambda s: river.water_level(s) - shape.design_depth_m,
    bottom_width=shape.bottom_width_straight_m, depth=shape.design_depth_m,
    side_slope=3.0, r_min=shape.min_bend_radius_m, corridor_sweep=(40, 80, 120))

def hb(xy):
    return widened_hb_from_shape(xy, shape)

for name, line in (("rolling circle", rolled.centreline), ("least-cost", route.line)):
    v = section_volumes(line, river.terrain, river.polygon, river.water_level,
                        shape.design_depth_m, hb, model="b", maxw_m=150.0)
```

Output:

```
pianc-wg141@0.3.0 + owd@0.3.0 x CEMT_IV: bottom 30.4 m, R_min 255 m, depth 3.25 m
8 bends tighter than R_min; 16 arcs spliced in
rolling circle: dredge   562,780 m3   dry cut   941,841 m3   tallest face 17.3 m
    least-cost: dredge   513,163 m3   dry cut   776,725 m3   tallest face 13.3 m
```

The numbers describe the synthetic river only.

To use your own river, pass your own geometry in a projected CRS (metres): a centreline or two
bank lines as shapely `LineString`s, the wetted area as a `Polygon`, and a terrain sampler. For a
GeoTIFF DEM in the same CRS:

```
from owd.volumes.raster import RasterTerrain
with RasterTerrain("my_dem.tif") as terrain:
    v = section_volumes(line, terrain, river_polygon, water_level, depth, half_width)
```

## Optional: optimise weirs, pool levels and the channel through each pool

`owd.optimise` adds two searches. Both take typed config objects and a terrain
sampler, and both minimise weights you supply, never prices:

- **Cascade** (`search_cascade`) — where the weirs go and how high each pool sits. An exact
  shortest path over (candidate site, pool level). The reach cost is yours; the usual one is
  earthworks per level from `build_level_table` + `earthworks_reach_cost`.
- **Lattice** (`optimise_pools`) — the channel through each pool, pinned to your design line at
  both ends. A* over left/right arcs at R_min and wider, so R_min holds by construction.

This is `examples/optimise_pools.py`; the tests run it.

```python
cfg = CascadeConfig(candidate_sites_m=[500, 1000, 1500, 2000, 2500], lifts_m=(0.5, 1.0),
                    terminal_m=3000.0, terminal_min_level_m=0.5, min_spacing_m=1000.0)
table = build_level_table(line, river.terrain, river.polygon, cascade_levels(cfg),
                          depth, half_width, model="b", maxw_m=120.0)
best = search_cascade(cfg, earthworks_reach_cost(table, wet_weight=1.0, dry_weight=1.5),
                      structure_cost=lambda lift: 1.0e5 + 2.0e5 * lift).best

spec = SectionSpec(half_width=half_width, side_slope_h_per_v=3.0, dry_weight=1.5,
                   rake_half_m=80.0, rake_step_m=5.0, ds_step_m=10.0)
lattice = LatticeConfig(r_min_m=r_min, pitch_m=20.0, n_heading=24, radius_multiples=(1, 2, 4))
out = optimise_pools(line, river.terrain, river.polygon, best.pools(), depth_m=depth,
                     section=spec, cfg=lattice, corridor_half_m=180.0, cell_m=5.0)
```

Output, re-measured with Cut Model B (a straight design line across a gently meandering
synthetic river):

```
weirs: 1000 m -> pool 0.5 m
  straight: dredge    21,988 m3   dry cut   934,659 m3
 optimised: dredge   141,201 m3   dry cut   423,598 m3
```

**Paying for length (`length_weight`).** Water deeper than the channel bed costs nothing to cut,
so without a length cost the lattice treats extra length through deep water as free. The setting
`length_weight` (`owd optimise --length-weight X`, or the optimise settings in the app) adds a cost
for each metre of design line, the sailing distance. Its unit is the unit of the cut cost: one m3
of cut at weight 1 costs 1. A weight of 10 means one metre of line costs as much as 10 m3 of cut.

To choose it, compare it with the cut of one metre of typical channel through dry ground. A 40 m
channel 5.5 m below the ground with 1:3 side slopes cuts about 310 m3 per metre. A weight well below
that (a few, up to a few tens) only stops the line wandering through free water. A weight near it
or above makes the search cut corners and meander necks to save length. That is a trade of cut for
length, and it can drive large cuts. The default is 0 (off). The weir cascade does not read it.

**Paying for land (`land_weight`).** Ground that the pool drowns deeper than the channel bed needs
no cut, whether it is river or floodplain. So without a land cost a line over drowned land beside
the river costs the same as a line in the river, and the search can leave the river for hundreds of
metres. The setting `land_weight` (`owd optimise --land-weight X`, or "Cost per m² of land" in the
app) charges every m2 of the channel's footprint that lies outside the river polygon, cut or no
cut. The footprint is the channel's plan width at the water surface: the bottom width plus the side
slopes up through the design depth. Its unit is m3 of cut per m2.

To choose it, compare it with the channel depth. A weight equal to the design depth prices a m2 of
land like a cut of the full depth over that m2: a good first value. A tenth of the depth still keeps
the line in the river where it can turn inside it. A weight much smaller than `length_weight`
divided by the footprint's width lets the line cut across bends to save length. The line still
leaves the river where the river bends tighter than R_min and is too narrow to turn in: no weight
can change that. The lattice's details give each pool's land area (`land_area_m2`) and land cost
(`land_cost`), and the baseline's land area beside them. The default is 0 (off). The cost steers
the search only: the volumes stage reports the m3 it measures and never the land cost. The weir
cascade does not read it.

**The corridor as a limit (`corridor_limit`).** The corridor is `corridor_half_m` (350 m by
default) either side of the baseline: the alignment's design line, or the river centreline when
there is none. By default it only tells the search where to sample its estimate of the cost to go;
the search itself may go anywhere in the rectangle round the corridor. With `corridor_limit` on
(`owd optimise --corridor-limit`, or "Keep the line inside the corridor" in the app) no point of the
lattice's line may lie outside the corridor, to within one grid cell (`cell_m`). A pool that cannot
end on its end pose inside the corridor is searched again with its end moved, as any pool that
cannot close. A pool with no line inside the corridor keeps the baseline; the details and a warning
say so. The limit never fails silently. Turn it off again with `--corridor-limit false`.

**With no DEM: optimise on plan area.** A design that has no DEM can still run the optimise stage.
There is then no ground to cut, so the stage chooses no weirs and no pool levels, and it needs no
water level. It searches the whole line for the least channel footprint outside the river polygon,
in m2, at a land weight of 1 unless `land_weight` is set, plus the length cost. The results
(`optimised_on: "plan_area"`), the lattice's details, a warning on the design line, the command
line and the app all say that the line was optimised on plan area, not on volume. The volumes stage
still needs a DEM. A design that names a DEM whose file is missing is refused as before: plan area
is never a quiet fallback.

**Paying for flooded land (`flood_cost_per_ha`, `max_above_bank_m`).** The weir search prices cut and
structures. A higher pool always cuts less, so with nothing else in the cost it takes the largest
lift you offer at every weir, and the pools can stand far above the banks. Two optimise settings
give it the other side of the trade. Both need a DEM, and both are off by default: a design that
sets neither gives the same weirs and numbers as before.

- `flood_cost_per_ha` (`owd optimise --flood-cost-per-ha X`, or the Optimise row in the app) is the
  cost of one hectare of land a pool floods outside the river polygon. It is in the units of the cut
  cost: with `wet_weight` and `dry_weight` as prices per m3, it is money per hectare. A starting
  value is 3,000. The right value is the engineer's to set: it stands for the land, crops, houses
  and roads the water takes.
- `max_above_bank_m` (`--max-above-bank X`) is a hard limit: no pool may stand more than X metres
  above bank-full anywhere in its reach. When no cascade can reach the top level inside the limit,
  the stage fails and names the reach that blocks it and what would unblock it (more candidate
  sites, smaller lifts, or a higher limit). It never falls back to a cascade outside the limit.

To compare the flood cost with the cut: 3,000 per hectare is 0.30 per m2, so at a cut price of 1 per
m3 flooding one hectare costs the same as cutting 3,000 m3. A 60 m channel cuts about 60,000 m3 more
per kilometre for each metre the pool is lowered, which is the price of 20 ha. So at 3,000 a metre of
pool is worth lowering only where it floods more than about 200 m of width for each kilometre of
river. With structures at their own prices the search may still prefer few, high weirs: read the
per-pool table and raise the cost, or set the limit, until the pools are ones you would build.

*What counts as flooded.* Ground on a grid of `flood_cell_m` cells (default 25 m), outside the river
polygon, below the pool level, joined to the river through other ground below the level (a hollow
away from the river stays dry), nearest to the pool's own length of the design line, and within
`flood_distance_m` of the line (`--flood-distance`, default 1,500 m). Ground past that distance, or
past the edge of the DEM, is not counted; a pool whose water reaches that edge is flagged, and its
hectares are then a lower bound. No bund is assumed beside a weir: low ground near a weir that joins
the river on the far side of it counts as flooded.

*Bank-full is an estimate.* In each 500 m of line it is the 25th percentile of the DEM in the cells
that touch the river polygon, on each side of the line, and the lower of the two sides. A coarse DEM
or a river polygon drawn wide of the water moves it. Check it against what you know of the banks.

*The report.* With either setting on, or with `--flood-report` (which leaves the search as it is),
`results.cascade.flood` lists each pool: its level and lift, the hectares it floods and their cost,
how far it stands above bank-full at its lower end, its upper end and at its worst place, and its
depth over the DEM's river level at each end. `owd optimise` prints the table; the app shows it under
the Optimise row.

**Paying for flooded buildings (`building_cost`).** With a buildings file loaded (`owd import
--buildings FILE`, `inputs.buildings` in a project, or Buildings in the app's Data), `building_cost`
(`owd optimise --building-cost X`, or the Optimise row in the app) is the cost of one building a pool
floods, in the units of the cut cost. It is the engineer's price, for example a resettlement cost, not
a fact. A building counts when its point is in land the pool floods by the rule above and the water
there is at least `building_min_depth_m` deep (`--building-min-depth`, default 0.3 m). Buildings below
the confidence threshold (default 0.7) or the smallest area are counted apart and never costed. Off
(0) by default: the search is then unchanged. With buildings loaded the flood report also lists the
buildings and the settlement land each pool floods, and the first 20 flooded buildings by place.
`owd volumes` and the screening report say which buildings the cut takes, comes within 50 m of, or
cuts off on a meander island. See [Buildings, places and settlements](docs/buildings-and-places.md).

*A wider lift menu.* The default lifts are 0.5 and 1 m. Set your own with `--lifts`, for example
`--lifts 1,2,3,4,5,6,7,8,9,10,11,12`, and candidate sites with `--sites` or the setting
`site_step_m`. A menu of 1 to 12 m with a site every 500 m on a 240 km river is searched in under a
second once the tables are built; building the cut table (one pass of the sections for each pool
level) is the long part.

**The joins: seams, weir sites and S-bends (`seams_on_straights`, `free_seams`, `min_straight_m`).**
The lattice searches each pool on its own, and a pool longer than 6 km in pieces. By default every
search starts and ends on the baseline's exact point and heading (the baseline is the alignment's
design line, else the river centreline). A pinned point on a bend forces a curve of R_min beside
it, and a pinned point off the river forces the line off the river there. Three settings change
that. All are off by default, and a design that does not set them gives the same line as before.

- `seams_on_straights` (`owd optimise --seams-on-straights`): the seam between two pieces of a long
  pool goes where the baseline is straightest and inside the river polygon, within `seam_window_m`
  (`--seam-window`, 1,500 m by default) of the even split. Straightness is the baseline's mean
  radius over the weir straight either side of the point (two vessel lengths, and at least half of
  R_min). The details list each seam: where the even split was, where the seam is, and why.
- `free_seams` (`--free-seams`): at a seam the line passes a gate instead of a point. The gate is
  the cross-line through the baseline's point, as wide as the river is there (set
  `seam_offset_m`, `--seam-offset`, for a fixed room either side), and the heading may differ from
  the baseline's by `seam_heading_tol_deg` (`--seam-heading-tol`, 30 degrees by default). The next
  piece starts from the pose the line ended on, so there is no gap and no closing curve. At a weir
  site between two pools the line passes a lock, so it must be straight there: the line runs
  straight for `weir_straight_m` (`--weir-straight`; two lengths of the sized vessel by default)
  through the gate, half upstream of the weir's cross-line and half downstream. Where the search
  finds no such line (a weir on a tight bend in a narrow corridor, for one), the weir site is pinned
  to the baseline as before, and the details and a warning say why by name. The ends of the whole
  line stay pinned.
- `min_straight_m` (`--min-straight`, 0 is off): between a left curve and a right curve the line
  runs straight for at least this length. One ship length is a good first value. The search holds
  it in its state, so it is a rule and not a cost; it is counted in whole lattice straights (twice
  the pitch), so the length enforced is the next multiple at or above the setting.

The details give every seam (`seams`) and every weir site (`weirs`): its chainage, whether it is
free, the gate, where the line passed it, and the straight the line has there, measured on the
line.

The search cost is a search objective. Re-measure the winner with `volumes.sections` before
you quote a quantity. The lattice is coarse by design; a finer one (`pitch_m`, `n_heading`,
more `radius_multiples`) follows the river more closely and runs longer.

## Map imagery (the Imagery setting)

The app draws satellite imagery under the design. Open **Imagery** in the header to choose the source. The choice is yours alone: it is kept on your machine (in the browser's storage for the web app) and is never saved in a design file, an export, a report or a drawing. A design opened on another machine shows that machine's own choice. Drawings and reports use no imagery.

- **Sentinel-2 cloudless 2016 (EOX, open)** is the default. It needs no key.
- **Esri World Imagery (your Esri key)** uses your own Esri account. Sign up for an ArcGIS Location Platform account at <https://location.arcgis.com>, create API key credentials with the Basemaps privilege (Location services, Basemaps: the Basemap styles service; the Static Basemap Tiles privilege alone gives place names with no picture), and paste the key into the field. Esri tiles are asked for only when you zoom in to a river reach; further out the map shows the open imagery, and says so. Esri bills per tile, with a free monthly allowance. A key expires, after at most one year. OWD stores the key only on your machine, shows only its last four characters, and sends it only to Esri to ask for tiles. Esri's own terms apply to your use of its imagery; OWD does not cache or save Esri tiles. The map shows "Powered by Esri" and Esri's data sources. If the key is wrong or has expired, the map says so in one line and goes back to the open imagery.
- **Custom tiles** takes an XYZ tile address with `{z}`, `{x}` and `{y}`, and an attribution text that the map shows. Check that the provider's terms allow your use.

## Exit codes

Every `owd` command exits with one of these codes. A failed check on an option is part of the result,
not an error: it is printed (`the option fails: …` on stderr), kept in the design and listed as
`failed_checks` in `--json`.

| Code | When |
|------|------|
| 0 | The command ran and saved the design, whether or not an option fails a check. |
| 1 | With `--strict`: an option the command made or re-ran fails a check (the design is saved first). Also: a stage that cannot run because an earlier one is stale; a checking command (`check-banks`, `validate`, the bank checks of `reach`) that finds a fault. |
| 2 | A usage or input error: a bad flag, a missing or unreadable file, a value out of range. Nothing is saved. |

Scripts that stopped on code 1 for a failed option check must add `--strict`.

## Conventions

- **Units:** metres, m², m³. **CRS:** any projected metric CRS; reprojection is the caller's job.
- **Offsets:** signed, − left bank, + right bank, looking downstream→upstream along the line.
- **Checks are data:** rules return `Check(name, ok, message, severity)` lists, not buried `if`s.
- **User choices are named, not hidden:** `BANK_CUT_FRACTION`, the reduced-radius claim on a canal reach, the
  cut-model parameters and the loop-rejection thresholds are judgements, not standards.

## Known issues

- **Rolling-circle rejoin kinks.** `alignment.rolling_circle` joins each arc to the old line
  where the circle crosses it, not tangentially. On a vessel-length chord (about 50 m) the line
  holds R_min. On a short chord the rejoin point reads below R_min: on the synthetic river at
  R_min 300 m, 111 m on a 10 m chord and 288 m on a 25 m chord. Read curvature on a
  vessel-length chord, or smooth the exits first. Details:
  [docs/known-issues.md](docs/known-issues.md).
- **R_min is reported, not guaranteed.** `rolling_circle` and `optimise_route` can return a
  design line tighter than R_min (a jagged input line; a baseline tighter than R_min). Both
  results carry `radius_ok`, `measured_min_radius_m` and `radius_check_chord_m`. Check
  `radius_ok` before you use the line.

## Contributing

Issues and pull requests are welcome — a new national Rule Set is a good first contribution
([docs/rules/codification.md](docs/rules/codification.md)). See
[CONTRIBUTING.md](CONTRIBUTING.md) and [ROADMAP.md](ROADMAP.md).

## Licence

Apache-2.0. Copyright 2026 African River Corridors Limited. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
