# The optimisation: appendices

These are the appendices of [The optimisation: mathematics and approach](README.md).

- [A Commands for every number](#a-commands-for-every-number)
- [B Settings](#b-settings)
- [C Derivations](#c-derivations)
- [D Where each section is in the source](#d-where-each-section-is-in-the-source)

## A Commands for every number

### The files

Everything of the worked example is in `docs/optimisation/example/`.

| File | What it is |
|---|---|
| `make_inputs.py` | Writes the synthetic river from formulas: `inputs/centreline.geojson`, `inputs/river-polygon.geojson`, `inputs/dem.tif`. The three files are kept in the repository, so you need not run it. |
| `runs.json` | The commands of every run. The script and the tests both read it. |
| `set_setting.py` | Sets an optimise setting that has no command-line flag, in a design file. |
| `run.py` | Runs every run, stores what the tool printed in `runs/`, writes `numbers.json` and `station.csv`, then draws the figures. |
| `figures.py` | Draws the figures in `docs/optimisation/figures/` from the run files. |
| `derivations.py` | The numerical check of appendix C. |
| `numbers.json` | Every number the document quotes, by key, with the run and the place in the design file it was read from, or how it was derived. |
| `station.csv` | The section of the station of section 3, sample by sample: offset, ground, in the river or not, cut face, cut. Kept in the repository. |
| `runs/*.txt` | What the tool printed in each run, untouched. |

### Run it all

Build the tool from this repository, then run the script. It takes about two minutes.

```bash
cargo build --release -p owd-cli
python3 docs/optimisation/example/run.py
```

The script needs Python 3 and nothing else. It writes the design files to
`docs/optimisation/example/out/`, which is not kept in the repository.

### Run it by hand

Copy the folder `docs/optimisation/example/` to a place of your own and work there, with `owd` on
your path. The design file names its DEM by a path beside it, so the DEM is copied first. `--date`
fixes the date written into the file, so two runs give the same file.

```bash
cp inputs/dem.tif dem.tif
owd new example.owd --title "Example River" --date 2026-11-24
owd import example.owd --centreline inputs/centreline.geojson --polygon inputs/river-polygon.geojson \
    --dem dem.tif --water-level 0.0 --water-slope -0.001
owd size example.owd --rule-set pianc-wg141 --set-input se_quality=B --vessel CEMT_IV --draught 2.5
owd align example.owd --method rolling-circle
python3 set_setting.py example.owd terminal_min_level_m=6.5
```

`--water-slope -0.001` says the water falls 1 mm per metre towards chainage 0, the downstream end.
The top level of the cascade, `terminal_min_level_m`, has no flag: it is set in the design file.

**The base run** (sections 2 to 7):

```bash
cp example.owd base.owd
owd optimise base.owd --lifts 0.5,1,1.5,2 --sites 1000,2000,3000,4000,5000,6000 --flood-report
owd volumes base.owd
```

**The runs of section 9.** Each starts from a fresh copy of `example.owd` and adds one flag, or two,
to the `owd optimise` command of the base run, then runs `owd volumes`:

| Run | Added flags |
|---|---|
| `length-300` | `--length-weight 300` |
| `length-1000` | `--length-weight 1000` |
| `land` | `--land-weight 3.25` |
| `flood-3000` | `--flood-cost-per-ha 3000` |
| `flood-20000` | `--flood-cost-per-ha 20000` |
| `bank-0` | `--max-above-bank 0` |
| `free-seams` | `--free-seams` |
| `free-seams-0` | `--free-seams --weir-straight 0` |

For example:

```bash
cp example.owd land.owd
owd optimise land.owd --lifts 0.5,1,1.5,2 --sites 1000,2000,3000,4000,5000,6000 --flood-report --land-weight 3.25
owd volumes land.owd
```

### Where a number is in a design file

A design file (`.owd`) is JSON. The numbers of this document are read from these places:

| What | Place in the file |
|---|---|
| Channel size | `stages.sizing.results` |
| Baseline and its checks | `stages.alignment.results` |
| Weirs, lifts, levels and costs | `stages.optimise.results.cascade`. Its `reach_cost` is the cut plus the flood cost; with a flood cost on, the cut alone is `cascade.flood.cut_cost`. |
| Flooded land and bank-full, per pool | `stages.optimise.results.cascade.flood` |
| Design line and its checks | `stages.optimise.results.design_line.line`, `.checks` |
| Each pool's search: cost, baseline cost, used or not, why not | `stages.optimise.results.design_line.details.pools` |
| Joins, seams and weir sites | `stages.optimise.results.design_line.details.joins`, `.seams`, `.weirs` |
| Volumes | `stages.volumes.results` |
| Stations and their areas | `stages.volumes.results.sections.stations`: one record a station, with `chainage_m`, `half_width_m`, `water_level_m`, `invert_m` (the bed level), `dredge_area_m2`, `excav_area_m2` (dry cut) |
| The section of a station | `profile` in the station's record: `offsets_m`, `terrain_m`, `wet` (in the river polygon) and `cut_face_m` (rounded to 1 cm). Kept only for the station nearest each 500 m of line and at a few tight bends: 21 of the 268 stations of the base run. |

`numbers.json` gives the exact place of each number as a JSON pointer.

### The station by hand

`station.csv` holds the 401 samples of the station of section 3: the station's `profile` from
`base.owd`, with the cut face and the cut worked from the formulas. To check it yourself, sum the last
column over the rows with `in_river` = 1 for the dredge area, and over the rows with `in_river` = 0
for the dry cut area; each sample stands for 1 m. The script works the `face_m` and `cut_m` columns
from the formulas of section 3, not from the tool's areas, and stops with an error if the sums
differ from the tool's by more than one part in a million. It does the same for every station that
keeps a section, and checks the half-width formula of section 3 at all 268 stations. The areas of
the other stations cannot be checked from the file alone: for those you need the DEM and the line.

## B Settings

### Settings of the optimise stage

Give a setting with its flag on `owd optimise`, where it has one. A setting with no flag is set in
the design file, under `stages.optimise.settings` (`set_setting.py` does this), or in OWD's web app.
A name the stage does not know is refused.

A test holds the names and defaults of this table to the engine's.

<!-- settings:optimise -->
| Setting | Flag | Unit | Default | What it does |
|---|---|---|---|---|
| `site_step_m` | none | m | `500` | With no list of sites: a candidate weir site every this many metres of baseline. |
| `candidate_sites_m` | `--sites` | m | `null` | The candidate weir sites, as chainages along the baseline. `null`: one every `site_step_m`. |
| `lifts_m` | `--lifts` | m | `[0.5, 1]` | The lifts a weir may have. |
| `terminal_min_level_m` | none | m | `0.5` | The top pool must be at or above this level. The level cap is this plus the largest lift. |
| `min_spacing_m` | none | m | `0` | The least distance between two weirs. |
| `base_level_m` | `--base-level` | m | `null` | The water level below the first weir. `null`: the design's water level at the start of the line. |
| `k_best` | none | count | `5` | The weir search keeps this many of its best cascades, each with a different last weir or top level. The stage uses the best one. |
| `wet_weight` | `--wet-weight` | cost per m³ | `1` | The weight of dredge (cut inside the river polygon), in both searches. |
| `dry_weight` | `--dry-weight` | cost per m³ | `1` | The weight of dry cut (cut outside the river polygon), in both searches. |
| `length_weight` | `--length-weight` | cost per m | `0` | The cost of one metre of line, in the pool search. 0 is off. |
| `land_weight` | `--land-weight` | cost per m² | `0` | The cost of one m² of channel, at the water surface, outside the river polygon, in the pool search. 0 is off. |
| `flood_cost_per_ha` | `--flood-cost-per-ha` | cost per ha | `0` | The cost of one hectare of land a pool floods, in the weir search. 0 is off. Needs a DEM. |
| `max_above_bank_m` | `--max-above-bank` | m | `null` | No pool may stand more than this above bank-full. `null`: no limit. Needs a DEM. |
| `flood_distance_m` | `--flood-distance` | m | `1500` | Flooded land is counted out to this distance from the baseline. |
| `flood_cell_m` | none | m | `25` | The side of a cell of the flood grid. |
| `flood_report` | `--flood-report` | yes or no | `false` | Report each pool's flooded land and height above bank-full with no flood cost and no limit. |
| `structure_fixed` | none | cost | `100000` | $F$: the part of a weir's cost that does not depend on its lift. |
| `structure_per_lift` | none | cost per m | `200000` | $P$: the part of a weir's cost per metre of lift (to the power below). |
| `structure_power` | none | none | `1` | $p$: the power of the lift in the structure cost. |
| `cut_model` | `--model` | name | `"cut_model_a"` | The section of the weir search's cut table: `a` or `b`. |
| `station_m` | none | m | `25` | The station spacing of the weir search's cut table. |
| `maxw_m` | none | m | `200` | The half-width of each section of the weir search's cut table. |
| `half_width_m` | none | m | `null` | The bottom half-width $b$ in both searches. `null`: half the sized bottom width. |
| `side_slope` | none | horizontal per vertical | `3` | The side slope $m$ of the pool search's section. |
| `rake_half_m` | none | m | `250` | How far each rake of the pool search reaches either side of the line. |
| `rake_step_m` | none | m | `4` | The spacing of the samples of a rake. |
| `ds_step_m` | none | m | `8` | The spacing of the sections of the search cost. On the baseline it is the spacing. On a move it has no effect unless it is set above 10 m, the spacing a move's sections already have. |
| `pitch_m` | none | m | `12` | The grid the states are snapped to. The straight is twice this. |
| `n_heading` | none | count | `16` | The number of headings. Each arc turns by one step of 360° over this. |
| `radius_multiples` | none | none | `[1, 1.5, 3]` | The radii of the arcs, as multiples of $R_{\min}$. None may be below 1. |
| `corridor_half_m` | none | m | `350` | The half-width of the corridor round the baseline. |
| `corridor_limit` | `--corridor-limit` | yes or no | `false` | Hold the line inside the corridor. |
| `seams_on_straights` | `--seams-on-straights` | yes or no | `false` | Put the seams of a long pool where the baseline is straightest and in the river. |
| `seam_window_m` | `--seam-window` | m | `1500` | How far a seam may move from the even split (with `seams_on_straights`). |
| `free_seams` | `--free-seams` | yes or no | `false` | Let the line pass a gate at a seam or weir site, in place of the baseline's point and heading. |
| `seam_offset_m` | `--seam-offset` | m | `null` | The room of a gate either side of the baseline. `null`: the river's width there, no more than half of `corridor_half_m`. |
| `seam_heading_tol_deg` | `--seam-heading-tol` | degrees | `30` | How far the line's heading at a gate may be from the baseline's. |
| `weir_straight_m` | `--weir-straight` | m | `null` | The straight through a weir site with `free_seams`. `null`: two lengths of the sized vessel. |
| `min_straight_m` | `--min-straight` | m | `0` | The least straight between a left curve and a right curve. 0: one straight of twice the pitch. |
| `cell_m` | none | m | `2` | The cell of the grid the pool search reads the ground on. |
| `check_chord_m` | none | m | `null` | The chord of the $R_{\min}$ check. `null`: a third of $R_{\min}$. |
| `method` | `--method` | name | `"lattice"` | `lattice`: the weir cascade, then the pool lattice (this document). `windows`: the window optimiser, with its own settings under `windows` (see `docs/window-optimiser.md`). |
<!-- /settings -->

### Settings of the volumes stage

<!-- settings:volumes -->
| Setting | Flag | Unit | Default | What it does |
|---|---|---|---|---|
| `cut_model` | `--model` | name | `"cut_model_a"` | The section: `a` or `b`. Model B is model A with the works stopped, on each side, where the ground has been at or below the cut face for 15 m. |
| `station_m` | `--station` | m | `25` | The station spacing $\Delta s$. |
| `maxw_m` | `--maxw` | m | `200` | How far each section reaches either side of the line. |
| `dx_m` | none | m | `1` | The spacing $\Delta x$ of the samples across a section. |
| `wet_slope` | none | horizontal per vertical | `3` | The under-water slope $m$. |
| `half_width_m` | none | m | `null` | The bottom half-width $b$. `null`: half the sized bottom width, widened in bends. |
<!-- /settings -->

### Values fixed in the engine

These are not settings. They are listed because a result depends on them. The same test holds them
to the engine's.

<!-- settings:fixed -->
| Value | In the code | Unit | Value | What it is |
|---|---|---|---|---|
| Straight move | `straight_len_m` | pitches | `2` | The length of the straight, as a multiple of `pitch_m`. |
| Sampling along a move | `arc_step_m` | m | `10` | The spacing of the points of a move that sections are taken at. |
| Closing reach | `close_reach_m` | $R_{\min}$ | `3` | A closing path is tried from states this close to the end. |
| Fallback tolerance | `goal_pos_tol_m` | pitches | `0.5` | With no closing path: the end is a state this close to the end point. |
| Fallback heading tolerance | `goal_head_tol_bins` | heading steps | `1` | And within this of the end heading. |
| Grace after the fallback | `CLOSE_GRACE_EXPANSIONS` | states | `20000` | The search goes on this long for a closing path before it takes the fallback. |
| Estimate | `heuristic` | name | `"c_min"` | The estimate of the cost to go: the cheapest metre times the straight distance. |
| Estimate weight | `heuristic_weight` | none | `1` | The estimate is used as it is, not scaled up. |
| Search limit | `max_expand` | states | `4000000` | A search that expands this many states stops with no line. |
| Longest piece | `max_pool_piece_m` | m | `6000` | A longer pool is searched in pieces. |
| Seam slack | `PIECE_BOUNDARY_SLACK` | of a piece | `0.125` | How far a piece boundary may move from the even split. |
| Grid margin | `bounds_margin_m` | m | `20` | The search grid is the corridor's rectangle plus this. |
| Known-ground share | `min_valid_fraction` | share | `0.6` | A move needs this share of its bottom samples on known ground. |
| Estimate: spacing | `DEFAULT_C_MIN_STRIDE_M` | m | `24` | $c_{\min}$ is sampled on a grid of this spacing. |
| Estimate: points | `DEFAULT_C_MIN_MAX_SAMPLES` | count | `4000` | At most this many points. |
| Gate bias | `GATE_MIDDLE_BIAS` | share | `0.01` | An end at the edge of a gate must be this much cheaper than one in the middle. |
| Moved end: step | `BOUNDARY_STEP_FRACTION` | $R_{\min}$ | `0.25` | The step by which a search end is moved. |
| Moved end: steps | `BOUNDARY_STEPS` | count | `4` | The most steps each way. |
| Line arc step | `LINE_ARC_STEP_M` | m | `2` | The longest step of the stored line along an arc. |
| Radius reading tolerance | `RADIUS_TOL` | share | `0.02` | The $R_{\min}$ check passes at $(1 - \text{this}) R_{\min}$. |
| Berm | `BERM_M` | m | `2` | The berm of Cut Model A at the pool level. |
| Upper slope | `ABOVE_SLOPE_H_PER_V` | horizontal per vertical | `1.5` | The slope of Cut Model A above the berm. |
| Bench gate | `PRISM_INTRUDE_M` | m | `2` | A side is benched when ground above the pool level reaches further than this into the prism. |
| Bank window | `BANK_WINDOW_M` | m | `500` | Bank-full is estimated in windows of this length. |
| Bank percentile | `BANK_PERCENTILE` | percent | `25` | The percentile of the bank cells' ground taken as the bank level of a side. |
| Bank cells | `BANK_MIN_CELLS` | count | `3` | A side needs this many bank cells in a window to give a level. |
<!-- /settings -->

## C Derivations

### C.1 The curvature of a line shifted sideways from a seed

This is the formula of section 8. It is for the planned method; nothing in the released tool uses it.

**Set-up.** The seed line is $\mathbf{r}_0(s)$, with $s$ its arc length. $\mathbf{T}(s)$ is its unit
tangent and $\mathbf{N}(s)$ its unit normal, a quarter turn to the left of $\mathbf{T}$. Its
curvature $k_0(s)$ is positive for a turn to the left. The Frenet formulas for a plane curve are

$$\mathbf{r}_0' = \mathbf{T}, \qquad \mathbf{T}' = k_0\, \mathbf{N}, \qquad \mathbf{N}' = -k_0\, \mathbf{T}$$

The shifted line is $\mathbf{r}(s) = \mathbf{r}_0(s) + d(s)\, \mathbf{N}(s)$. Note that $s$ is the
arc length of the seed, not of the shifted line.

**First derivative.**

$$\mathbf{r}' = \mathbf{T} + d'\, \mathbf{N} + d\, \mathbf{N}' = (1 - k_0 d)\, \mathbf{T} + d'\, \mathbf{N}$$

Write $a = 1 - k_0 d$. Then $\mathbf{r}' = a\, \mathbf{T} + d'\, \mathbf{N}$ and
$\lvert \mathbf{r}' \rvert^2 = a^2 + d'^2$.

**Second derivative.** With $a' = -k_0' d - k_0 d'$,

$$\mathbf{r}'' = a'\, \mathbf{T} + a\, \mathbf{T}' + d''\, \mathbf{N} + d'\, \mathbf{N}' = (a' - k_0 d')\, \mathbf{T} + (a k_0 + d'')\, \mathbf{N}$$

$$\mathbf{r}'' = -(k_0' d + 2 k_0 d')\, \mathbf{T} + (k_0 a + d'')\, \mathbf{N}$$

**Curvature.** For a plane curve with any parameter, the signed curvature is the cross product of
the first two derivatives over the cube of the speed. In the frame $(\mathbf{T}, \mathbf{N})$, the
cross product of $u_T \mathbf{T} + u_N \mathbf{N}$ and $v_T \mathbf{T} + v_N \mathbf{N}$ is
$u_T v_N - u_N v_T$. So

$$k = \frac{\mathbf{r}' \times \mathbf{r}''}{\lvert \mathbf{r}' \rvert^3} = \frac{a\,(k_0 a + d'') + d'\,(k_0' d + 2 k_0 d')}{(a^2 + d'^2)^{3/2}}$$

and with $a = 1 - k_0 d$ this is the formula of section 8:

$$k = \frac{(1 - k_0 d)\bigl(k_0 (1 - k_0 d) + d''\bigr) + d'\,\bigl(k_0'\, d + 2 k_0\, d'\bigr)}{\bigl((1 - k_0 d)^2 + d'^2\bigr)^{3/2}}$$

**Three checks by hand.**

- $d = 0$: $a = 1$, and $k = k_0$.
- $d$ constant, $k_0 = 1/R_0$ constant: $k = k_0 a^2 / a^3 = k_0 / (1 - k_0 d) = 1/(R_0 - d)$. This
  is the parallel curve of a circle: a circle of radius $R_0 - d$.
- $k_0 = 0$ (a straight seed): $k = d'' / (1 + d'^2)^{3/2}$, the curvature of the graph of $d$.

The formula needs $a > 0$, that is $d < 1/k_0$ on a left bend: the shift must not pass the centre of
the seed's arc.

**A check with numbers.** We ran this check; it is `derivations.py` in the example folder. It builds
a seed whose curvature varies, $k_0(s) = 1/400 + \sin(s/150)/2000$ per metre, over 1,200 m, and
shifts it by $d(s) = 20 \sin(s/120)$ metres. It then reads the curvature of the shifted points
themselves by finite differences and compares with the formula at 120 stations.

| | Value |
|---|---:|
| Tightest radius of the seed, m | 333.3<!--=curvature.seed_tightest_radius_m--> |
| Tightest radius of the shifted line, by the formula, over its whole length, m | 276.7<!--=curvature.tightest_radius_m--> |
| Largest difference between the formula and the finite differences, as a share of the largest curvature | 0.000012<!--=curvature.worst_relative--> |

The two agree to about one part in 100,000, which is the accuracy of the finite differences. The
shift tightens this seed from 333 m to 277 m: a limit on the seed's radius alone would not have
caught it.

This check uses a seed whose curvature changes smoothly. A seed of arcs and straights has jumps of
curvature at its joins, where $k_0'$ does not exist; section 8 says what that means for the shifted
line.

### C.2 The length of an arc that turns one heading step

An arc of radius $R$ that turns the heading by $\Delta\theta$ has length $R\, \Delta\theta$. With 16
headings $\Delta\theta = 2\pi/16$. For $R$ = 255 m: $255 \times 0.392699$ = 100.14 m. Starting at
the origin heading along $x$, a left arc ends at

$$x = R \sin \Delta\theta, \qquad y = R\,(1 - \cos \Delta\theta)$$

that is 97.58 m ahead and 19.41 m to the left for $R$ = 255 m. These end points are not on the 12 m
grid, which is why the states are snapped.

### C.3 The closing path

The closing path goes from a pose $A$ (a point and a heading) to a pose $B$ on an arc of radius
$R = R_{\min}$, a straight and a second arc of radius $R$. Each arc may turn left or right, so there
are four candidates.

For a candidate, put the centre of the first circle at distance $R$ from $A$, to the left of its
heading for a left turn and to the right for a right turn; the same for the second circle at $B$.
Let $q$ be the distance between the two centres and $\varphi_c$ the direction from the first to
the second.

- **Both arcs turn the same way.** The straight is the outer common tangent. Its length is $q$
  and its heading is $\varphi = \varphi_c$.
- **The arcs turn opposite ways.** The straight is the inner common tangent. It exists when
  $q \ge 2R$. Its length is $\sqrt{q^2 - 4R^2}$, and its heading is
  $\varphi = \varphi_c \pm \arctan\bigl(2R / \sqrt{q^2 - 4R^2}\bigr)$, plus for a left turn first.

The first arc turns from the heading of $A$ to $\varphi$, and the second from $\varphi$ to the
heading of $B$, each in its own direction. A candidate is dropped when the two turns add to more
than 180°, when its first arc turns against the arc the line arrived on, or, with `min_straight_m`,
when the straight between two opposite arcs is too short. Of the candidates left, the shortest is
taken. Its heading is continuous at both joins because the straight is tangent to both circles.

### C.4 Why the estimate of the pool search is a bound, and where it is not

Suppose every metre of any line costs at least $c_{\min}$. A line from a state to the end is at
least as long as the straight distance $\rho$ between them. So its cost is at least $c_{\min}\, \rho$,
and the estimate $e = c_{\min}\, \rho$ never exceeds the true cost to go. With such an estimate A\*
takes the end only when no cheaper way to it remains.

The cost per metre of line $\lambda$ fits this: every metre pays $\lambda$, so $c_{\min}$ includes
it.

The argument rests on $c_{\min}$ being a true least cost per metre. The search finds $c_{\min}$ by
trying a 12 m straight at up to 4,000 points, 24 m apart, in 8 headings. It is the least of those
trials. If the cheapest metre lies between the trial points, the estimate is a little high, and the
bound is not exact. When the cheapest ground costs nothing and $\lambda = 0$, $c_{\min} = 0$ and the
estimate is exact as a bound; that is the usual case in a pool that drowns its plain.

## D Where each section is in the source

File paths are given only here. Paths are from the root of the repository.

| Section | What | File | Function or type |
|---|---|---|---|
| 2 | Channel size | `crates/owd-format/src/design/sizing.rs` | `ChannelSize` |
| 2 | Rolling circle | `crates/owd-core/src/alignment/rolling_circle.rs` | `rolling_circle` |
| 2 | Rolling circle in the water | `crates/owd-core/src/alignment/rolling_circle/water.rs` | `rolling_circle_with_water` |
| 2, 6 | Stale stages | `crates/owd-format/src/design/session.rs` | `Session::dependency_hash` |
| 3 | Cut face and bench gate | `crates/owd-core/src/cut_model.rs` | `cut_model_a_face`, `cut_model_a_bench_gate` |
| 3 | Area of a section | `crates/owd-core/src/cut_model.rs` | `section_area_weighted` |
| 3 | Bend widening of the half-width | `crates/owd-core/src/cut_model.rs` | `widened_hb_from_shape` |
| 3 | Stations and volumes | `crates/owd-core/src/volumes/sections.rs` | `station_frame`, `section_volumes_weighted`, `SectionVolumes` |
| 3 | The volumes stage | `crates/owd-format/src/design/volumes_stage.rs`, `volumes.rs` | `volumes_stage`, `VolumesSettings` |
| 4 | Levels a cascade can reach | `crates/owd-core/src/optimise/cascade.rs` | `cascade_levels` |
| 4 | The dynamic programme | `crates/owd-core/src/optimise/cascade.rs` | `search_cascade` |
| 4 | The cut table by level | `crates/owd-core/src/optimise/cascade.rs` | `build_level_table`, `LevelTable`, `earthworks_reach_cost` |
| 4 | Flooded land | `crates/owd-core/src/optimise/flood.rs` | `FloodField::build`, `FloodTable` |
| 4 | Bank-full | `crates/owd-core/src/optimise/flood.rs` | `BankProfile` |
| 4 | Flood cost and bank limit | `crates/owd-core/src/optimise/flood.rs` | `FloodTerms`, `with_flood_cost`, `explain_blocked` |
| 4, 5 | The stage: settings, order of the two searches, structure cost | `crates/owd-format/src/design/optimise.rs` | `OptimiseSettings`, `Session::run_optimise` |
| 5 | The moves | `crates/owd-core/src/optimise/lattice.rs` | `lattice_primitives`, `LatticeConfig` |
| 5 | The cost of a move | `crates/owd-core/src/optimise/lattice.rs` | `FieldScorer::score`, `FieldScorer::land_area`, `SectionSpec` |
| 5 | The estimate | `crates/owd-core/src/optimise/lattice.rs` | `min_cost_per_metre` |
| 5 | The search, the gate | `crates/owd-core/src/optimise/lattice.rs` | `solve_lattice_ends`, `Gate` |
| 5 | The closing path | `crates/owd-core/src/optimise/lattice.rs` | `closing_path` |
| 5 | The corridor and the search grid | `crates/owd-core/src/optimise/lattice.rs` | `line_corridor`, `build_terrain_grid` |
| 5 | Pieces and seams | `crates/owd-core/src/optimise/lattice.rs` | `pool_pieces`, `seams_on_straights`, `gate_at` |
| 5 | One pool; all pools; moved ends; the loop guard | `crates/owd-core/src/optimise/lattice.rs` | `optimise_pool_ends`, `optimise_pools`, `pool_in_a_crossing` |
| 5 | The rule for keeping the baseline | `crates/owd-core/src/optimise/lattice.rs` | `adopt`, `segment_cost` |
| 6 | Radius on a chord | `crates/owd-core/src/alignment/radius_check.rs` | `min_radius_on_chord`, `radius_ok` |
| 6 | Joins | `crates/owd-core/src/optimise/lattice.rs` | `join_line`, `PoolJoin` |
| 6 | Self-crossing | `crates/owd-core/src/alignment/loops.rs` | `first_self_crossing_m`, `self_crossings_m` |
| 6 | Failed checks by name | `crates/owd-format/src/design/export.rs` | `failed_checks` |
| 6 | The straight at a weir site | `crates/owd-core/src/optimise/lattice.rs` | `straight_through_m` |
| 10 | The tests of this document | `crates/owd-cli/tests/optimisation_doc.rs`, `crates/owd-core/tests/optimisation_doc_claims.rs` | |
| 10 | The slow-test list | `.config/nextest.toml`, `docs/agents/slow-tests.md` | |

The design decisions behind the stages are in `docs/specs/0017-stage-contracts.md` and
`docs/adr/0015-stages-are-pure-functions-with-typed-contracts.md`.
