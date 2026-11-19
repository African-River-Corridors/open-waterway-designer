# Spec 0017: Stage contracts

Status: **draft for the maintainer's review, 2026-11-19.** Decision: ADR 0015 (stages are pure
functions with typed, versioned contracts). Builds on ADR 0004 (one channel design document), ADR 0006
(water level belongs to the river), spec 0002 (the document and CLI), spec 0007 (rules), spec 0014
(reports) and spec 0016 (the largest vessel). Planned for 0.5.

Marks used below: **[exists]** OWD does this today, **[changes]** it exists but its contract or
behaviour changes, **[new]** it does not exist.

## Problem statement

OWD must give a board, an investor or a lender's engineer a design answer they can trust, read
without running OWD. The answer depends on how much data goes in. Today the code cannot grow to that
answer cleanly:

- **Stages share one JSON value.** Each stage is a method on the design session. It reads and writes
  the whole document (`owd-format/src/design/session.rs`, `alignment.rs`, `optimise.rs`,
  `volumes.rs`). No stage can be tested without a full session.
- **Stages find each other by name.** Optimise finds its baseline as "the option whose method is
  `rolling_circle`"; the CLI finds "the `lattice` option". Six stages rebuild the channel shape from
  the sizing settings (`sized_shape()`) instead of reading the stored sizing result.
- **Types are duplicated.** The terrain sampler has six aliases, the elevation model three types, the
  refusal three. A water level profile has no type; it is a bare list of pairs.
- **The river polygon is parsed three more times inline** (alignment's re-route, optimise, volumes),
  and those copies drop the holes, so islands count as water.
- **Results have no schema.** Only the whole document has one. Every result but sizing is
  "any object".
- **Drawings and reports re-run the section walk,** because volumes stores only sums.
- **The chain is fixed** (sizing, alignment, optimise, volumes). Fits, the envelope, the plan cut and
  the reference line hang off it with their own hashes. There is no place for cost, for given weirs, for
  locks, for constraints, or for a structures tool.
- **One document holds several design-line options,** and later stages pick one by id. The maintainer
  wants one design per run.
- **Chainage starts at whichever end the centreline starts.** The maintainer wants it from the river
  mouth.
- **Bank checks are written twice** (`owd-cli/src/check_banks.rs` and `Session::bank_check_warnings`),
  and the two differ.

## Solution

1. **Stages are pure functions with typed inputs and outputs** (ADR 0015). Each has a schema and a
   version. One runner in `owd-shell` serves the CLI and the web app.
2. **One run makes one design.** A run has an **objective** (one design measure to minimise or
   maximise) and **limits** (design measures with a bound each). Options are separate runs.
3. **A run has a target output level and reports the level it reached,** per reach and for the whole
   run. Every figure carries the level of the reach it came from.
4. **New inputs:** constraints (no-go areas and a cut-rate multiplier model), a rates table, and given
   or candidate structures.
5. **New stages:** weirs and locks (optional), cost, and design measures. **Seams** for structures,
   hydraulics, sediment and bank extraction.
6. **Chainage from the river mouth.**
7. **Outputs read stored results:** the plan drawing, the long and cross sections, volumes by reach,
   cost, and the report.

## User stories

1. As a board member, I want one plan drawing with the chosen line, so that I see the answer first.
2. As a lender's engineer, I want each figure marked with the output level it reached, so that I know
   how far to trust it.
3. As a river engineer, I want to name what the run optimises and what it must stay within, so that the
   design answers my client's question.
4. As a river engineer, I want to give weirs and locks I already have, or let OWD choose among sites
   and designs I give, so that the design fits what I know about the river.
5. As a river engineer, I want constraints to make cutting dearer where they apply, so that the line
   avoids them when that is worth it.
6. As a river engineer, I want cost as my rates times OWD's volumes, by reach, so that I can check it.
7. As a river engineer, I want chainage to run from the river mouth, so that it matches how the river is
   known.
8. As a developer, I want to test each stage alone from a small fixture, so that a fault is found where
   it is made.
9. As a developer, I want a typed seam for each future tool, so that a structures tool plugs in without
   a change to the stages around it.
10. As a scripter, I want to run OWD many times with different settings and compare the results, so
   that I can build an option study on top of one-design runs.

## Implementation decisions

### Vocabulary

- **Run**: one call of the runner over one channel design document. It makes one design.
  The document stays the unit a user saves (`.owd`). [new term; "run" was in the glossary's avoid list
  for "channel design"; it now means one pass of the stages, not the document]
- **Design measure**: one quantity of the whole design (cost, total cut, land take, channel capacity,
  design line length, structure count). It is distinct from the rules engine's **Measure**, which is
  per object and tested by a Rule. [new]
- **Objective**: one design measure and a sense (minimise or maximise). [new]
- **Limit**: one design measure and a bound (at most or at least a value). [new]
- **Output level**: concept, pre-feasibility or feasibility. [new]
- **Stage output**: a typed value one stage returns and the document stores. [changes]

### Rules all stages follow

From ADR 0015, made concrete:

- **Signature.** `fn stage(input: &StageIn, ctx: &Ctx) -> Result<StageOut, StageError>`.
  `StageIn` is a struct of typed references to inputs and earlier outputs. `Ctx` holds only the host
  callbacks a stage needs (the terrain sampler, a progress hook). No `Session`, no `Value`.
- **Types.** Each `StageIn` and `StageOut` derives `Serialize`, `Deserialize` and `JsonSchema`.
  `schema/stages/<stage>.<version>.schema.json` is generated and checked in CI (the build fails if the
  file and the type differ).
- **Errors.** One `StageError { stage, code, message, at: Option<Chainage> }` with a stable `code`
  per failure. It replaces the three `Refusal` aliases and the free-text errors.
- **Storage.** `stages.<name> = { version, depends_on, settings, output }`. `depends_on` is the hash of
  the `StageIn` value (so it is exactly what the stage read). Status is computed on load, as today.
- **The graph.** Each stage declares the outputs it reads. The runner builds the order from that.
  An optional stage with no settings is skipped, and a stage that reads it gets `None`.
- **Shared value types,** defined once in `owd-format::types`: `Chainage` (metres from the mouth),
  `Polyline`, `RiverPolygon` (shell and holes), `ElevationModel` (raster plus CRS and hash),
  `TerrainSampler`, `WaterProfile` (points of river chainage and level), `Reach` (from, to), `Money`
  (amount and currency code). [new; replaces the aliases listed in the problem statement]
- **Testing a stage alone.** Each stage has `tests/fixtures/<stage>/*.json`: a `StageIn`, the expected
  `StageOut` or error code, and a label (`build-process.md`). A test reads the fixture, calls the
  function, and compares. Stages that read terrain get a small synthetic raster (the repo's
  `make_dem_fixtures.py`).

### Run inputs

The document's `inputs` block. Each is a typed value with its own schema.

| Input | Contents | Status |
|---|---|---|
| `river` | Bank lines and/or centreline and/or river polygon (with holes); the **mouth** (a point, or "the downstream end") | [changes] one parse; mouth is new |
| `elevation` | Reference to a raster (path, hash, CRS), whether it holds bathymetry | [exists] as `inputs.dem` |
| `bed` | A depth table or bed model | [exists] as `inputs.bed_model` |
| `survey` | Spot levels and surveyed cross sections, with their extents | [new] |
| `water` | Water level profile, or pool levels, or a fall | [exists] as `inputs.water_level` |
| `vessel_and_rules` | Rule Sets, vessel class or custom vessel, waterway reaches | [exists] |
| `run` | `objective`, `limits[]`, `target_level` | [new] |
| `constraints` | `no_go[]` (polygons, each with a name) and `cut_rate_multiplier` (a raster reference; cell value multiplies the cut rate; no-data means 1.0) | [new] |
| `rates` | Table: dredge, dry cut, disposal, haul (per m³, haul per m³·km), currency; optional per reach | [new] |
| `structures` | `Given { items[] }` or `Search { sites[], designs[] }`; absent means no weirs or locks | [changes] today the cascade search inside optimise takes sites and lifts only |

A hard exclusion can be a no-go polygon or a multiplier cell at or above `1e6`; the alignment stage
treats both as impassable.

### Chainage from the river mouth [changes]

- River chainage 0 is at the mouth: the downstream end of the river centreline. Design-line chainage
  0 is at the design line's downstream end.
- The `river` input states the mouth: a point, or the word `downstream` with a water profile or an
  elevation model to decide which end is lower. With neither, import refuses with
  `RIVER_MOUTH_UNKNOWN`. For a new cut canal or a basin, the engineer names the start point.
- The river stage reverses the centreline, the banks and the water profile so that all run from the
  mouth. Every chainage OWD writes is from the mouth.
- Opening a 0.4 document: if its line runs from upstream, the river stage reverses it, and every later
  stage is stale.

### Output levels [new]

The engineer sets `run.target_level`. OWD works out the level each reach can reach from its inputs:

| Level | Inputs needed in the reach | Stated accuracy of volumes | Outputs the level gives |
|---|---|---|---|
| Concept | Banks (or a river polygon) and an open elevation model | about ±50 % | one line, channel size, total volumes |
| Pre-feasibility | Plus spot levels or a partial survey, and a rates table | about ±30 % | volumes by reach, cost |
| Feasibility | Plus bathymetry and a ground survey covering the reach | about ±15 % | drawings and the board-ready report |

- **Reach level** = the highest level whose inputs cover that reach. "Cover" means the survey or
  bathymetry extent contains the design line's corridor in that reach.
- **Run level** = the lowest reach level. The report states both.
- **Each figure is marked** with its reach level. A figure that spans reaches (a total) takes the
  lowest.
- If the reached level is below the target, the run still saves; the report and the CLI say which
  inputs are missing, reach by reach. `--strict` makes that exit 1, as for failed checks today.
- The accuracy bands are the maintainer's statement, not a computed error. The report quotes them.

### The stages

Order of a full run: river → sizing → reference line → alignment → structures → water → refine →
volumes → cost → measures. Fits and the envelope are screening stages that read the river, sizing and
reference line; they are not on the design path.

#### 1. River [changes]

- **Purpose:** turn the river inputs into one checked geometry, oriented from the mouth.
- **Reads:** `inputs.river`, `inputs.water` (for the mouth only), project CRS.
- **Output `RiverGeometry`:** `centreline: Polyline`, `banks: Option<(Polyline, Polyline)>`,
  `polygon: Option<RiverPolygon>` (with holes), `length_m`, `bank_checks[]`.
- **Errors:** `RIVER_MOUTH_UNKNOWN`, `RIVER_NO_GEOMETRY`, `RIVER_POLYGON_INVALID`.
- **Today:** `base_line()`, `river_polygons()` and the import's bank turning do this in parts; three
  stages re-parse the polygon and drop holes; the bank check is in two places.
- **Change:** one stage; every later stage reads `RiverGeometry`; the bank check lives here only.
- **Test alone:** fixtures of a centreline given upstream-first (reversed), a polygon with an island
  (hole kept), banks in opposite directions.

#### 2. Sizing [changes]

- **Purpose:** the governing vessel's channel size with the rule trace.
- **Reads:** `inputs.vessel_and_rules`, `RiverGeometry` (for reaches).
- **Output `ChannelSize`:** `shape` (bottom width, depth, side slopes), `r_min_m`, bend widening rule,
  per-reach shape where reaches differ, `trace`.
- **Errors:** as today's sizing refusals, with codes.
- **Today:** typed in the schema; six readers rebuild the shape with `sized_shape()`.
- **Change:** readers take `&ChannelSize`; `sized_shape()` is removed.
- **Test alone:** a vessel and a Rule Set in, a shape and trace out (many exist today; move them to
  fixtures).

#### 3. Reference line [changes]

- **Purpose:** the smooth water-aware line that belongs to no vessel (spec 0016).
- **Reads:** `RiverGeometry`.
- **Output `ReferenceLine`:** line, settings, low-confidence stretches.
- **Today:** stored in `inputs.reference_line` with its own hash. **Change:** a stage like the others.
- **Test alone:** synthetic bends, as today.

#### 4. Alignment [changes]

- **Purpose:** one design line that meets R_min, stays in the corridor, avoids no-go areas and scores
  best on the run's objective.
- **Reads:** `RiverGeometry`, `ChannelSize`, `Option<ReferenceLine>`, `inputs.constraints`,
  `inputs.rates` (for a cost objective), `inputs.run`, the terrain sampler.
- **Output `DesignLine`:** `line: Polyline` from the mouth, `method` and its settings, `bends[]`
  (chainage from, to, radius, R_min, pass), `cut_offs[]` (river reaches the line leaves),
  `constraints_touched[]`.
- **Errors:** `ALIGN_NO_ROUTE`, `ALIGN_RADIUS_UNMET` (with chainage), `ALIGN_NO_GO_CROSSED`.
- **Today:** rolling circle, re-route and given line exist; each adds an entry to `options[]`;
  constraints do not exist; the re-route's least-cost grid uses unitless weights.
- **Change:** one line, no options list. The method is a setting. The re-route's cost grid uses the
  rates and the multiplier raster when given, and no-go cells are impassable.
- **Test alone:** a straight river (line equals centreline), a bend tighter than R_min (cut-off), a
  no-go polygon across a bend (line goes round), a multiplier patch (line moves off it when cheaper).

#### 5. Weirs and locks [changes; optional]

- **Purpose:** the structures along the design line and the pools they hold.
- **Reads:** `DesignLine`, `ChannelSize`, `RiverGeometry`, `inputs.water`, `inputs.structures`,
  `inputs.rates`, `inputs.run`, the terrain sampler; and, for search, the structures seam.
- **Two modes:**
  - `Given`: each item has location (chainage or point), upstream mean water level, structure height,
    kind (weir or lock or both) and cost. OWD places them and checks the pool levels step down to the
    mouth.
  - `Search`: candidate sites (points the engineer gives) and a short list of designs (each with a
    lift, height, footprint and cost, or a reference for the structures seam to price). OWD picks the
    set that scores best on the objective within the limits. It never invents a site.
- **Output `StructurePlan`:** `structures[]` (chainage, kind, upstream and downstream level, height,
  footprint, cost, source: given or seam), `pools[]` (from, to, level).
- **Errors:** `WEIRS_LEVELS_NOT_STEPPED`, `WEIRS_SITE_OFF_LINE`, `WEIRS_NO_FEASIBLE_SET`.
- **Today:** the cascade search inside optimise takes candidate chainages (or a site step) and a lift
  menu, and prices structures with unitless weights (`structure_fixed`, `per_lift`, `power`). Given
  structures and locks do not exist.
- **Change:** its own stage; `Given` is new; `Search` takes sites as points and a design list; cost in
  money from the rates and designs; locks are a kind.
- **Test alone:** given structures on a synthetic river (pools as given); a search with three sites and
  two designs whose best set is known by hand; a given level that steps up (error).

#### 6. Water [changes]

- **Purpose:** the water level along the design line.
- **Reads:** `RiverGeometry`, `inputs.water`, `Option<StructurePlan>`, `DesignLine`.
- **Output `WaterProfile`** on design-line chainage, with the source of each stretch (given, pool,
  open river).
- **Today:** `water.rs` holds the profile as bare pairs and samples it ad hoc. **Change:** a typed
  output read by volumes, drawings and the report. Pools from the structures stage set the level above
  each structure.
- **Test alone:** a given profile and two pools; the level at chainages either side of a weir.

#### 7. Line refinement [changes; optional]

- **Purpose:** move the design line within each pool to cut less, keeping R_min.
- **Reads:** `DesignLine`, `StructurePlan`, `WaterProfile`, `ChannelSize`, `RiverGeometry`,
  `inputs.constraints`, the terrain sampler.
- **Output:** a `DesignLine` that replaces the alignment's line for the stages after it; the alignment's
  line stays stored as the stage's input.
- **Today:** the pool lattice in optimise adds a `lattice` option. **Change:** it refines the one line.
- **Test alone:** one pool on a synthetic valley; the refined line cuts less and meets R_min.

#### 8. Volumes [changes]

- **Purpose:** cut volumes along the final design line.
- **Reads:** final `DesignLine`, `ChannelSize`, `WaterProfile`, `RiverGeometry` (polygon with holes),
  `inputs.bed`, `inputs.constraints.cut_rate_multiplier`, the terrain sampler.
- **Output `Volumes`:** `stations[]` (chainage, dredge m², dry m², cut-rate-weighted dredge and dry m²,
  section outline), `by_reach[]` (reach, dredge m³, dry m³, weighted m³), totals, `areas` (polygons of
  cut-off and dredge areas for the plan drawing).
- **Errors:** `VOLUMES_NO_ELEVATION`, `VOLUMES_OUTSIDE_RASTER` (with chainage).
- **Today:** stores sums and a station count; drawings re-walk; holes dropped.
- **Change:** stores the stations, so drawings and reports read them; holes kept; weighted volumes
  when a multiplier is given; reaches are the run's reaches (waterway reaches, pools, and level
  reaches).
- **Test alone:** a prism channel in a flat raster (volume known exactly); an island hole (its area is
  dry, not dredge); a multiplier of 2 over half the stations.

#### 9. Cost [new]

- **Purpose:** cost as rates times volumes, plus structures.
- **Reads:** `Volumes`, `inputs.rates`, `Option<StructurePlan>`.
- **Output `Cost`:** items (dredge, dry cut, disposal, haul, structures) by reach and in total, in the
  table's currency. Dredge and dry cut use the weighted volumes, so a constraint's multiplier raises
  the cost where it applies.
- **Errors:** `COST_NO_RATES` (run continues without cost; level capped at concept).
- **Test alone:** a fixed volume table and a rates table, by hand.

#### 10. Design measures [new]

- **Purpose:** the run's design measures, the objective's value, each limit's pass or fail, and the
  reached level.
- **Reads:** every output above that exists, and `inputs.run`.
- **Output `Measures`:** `values{}` (cost, total cut, dredge, dry cut, land take, line length,
  structure count, capacity), `objective`, `limits[]` (bound, value, pass), `level` (per reach, run,
  target, missing inputs per reach).
- **Land take** = area of the channel's top width outside the river polygon. **Capacity** = the
  governing vessel's size (see open assumptions).
- **Test alone:** fixtures of the outputs above; a limit that fails; a reach with no survey.

#### Screening stages: fits and envelope [changes]

Unchanged in purpose (spec 0016). They read `RiverGeometry`, `ChannelSize` and `ReferenceLine` by
type and store typed outputs. They are not on the design path and no design stage reads them.

### Seams [new]

Each seam is a pair of typed request and response, with a schema. Today each is filled by a given
input; later an Open Tool fills it through the same types.

| Seam | OWD sends | OWD gets back | Filled today by |
|---|---|---|---|
| **Structures** (first) | per structure: site (point and chainage), upstream and downstream water level, channel width | height, footprint (polygon), cost | the `designs[]` list or the `Given` items |
| Hydraulics | design line, channel shape, structures, flows | water profile | `inputs.water` |
| Sediment | design line, volumes, water profile | maintenance dredge per reach per year | nothing (empty) |
| Bank extraction | imagery reference and a reach | `river` input (banks and polygon with holes) | the given banks |

A seam has one Rust trait (`trait StructuresTool { fn price(&self, req: &[StructureRequest]) ->
Result<Vec<StructureResponse>, StageError>; }`) and a given-input implementation. The web app and the
CLI pass the same implementation.

### Outputs [changes]

All read stored stage outputs. None re-runs a stage.

- **Plan drawing** (leads the board output): imagery background; banks; the design line with chainage
  ticks from the mouth; each bend's radius against R_min (from `DesignLine.bends`); weirs and locks
  with pool levels (`StructurePlan`); cut-off and dredge areas shaded (`Volumes.areas`); constraints the
  line touches (`DesignLine.constraints_touched`). [changes: imagery, structures, shading and
  constraints are new; the plan sheet exists]
- **Long section:** bed, water profile, pools, structures, design depth (`Volumes.stations`,
  `WaterProfile`, `StructurePlan`). [changes: reads stored stations]
- **Cross sections:** at chosen chainages from `Volumes.stations`. [changes]
- **Volumes by reach** and **cost**: tables from `Volumes.by_reach` and `Cost`. [changes; cost new]
- **Board-ready report** (spec 0014): the plan drawing first; the objective, limits and their results;
  the reached level by reach and the missing inputs; every figure marked with its level. [changes]
- **Design measures** as JSON for option scripts: `owd measures --json`. [new]

### Options are a layer on top

One run, one design. An option study is a script or several app runs. The CLI gives what a script
needs: `owd run <design> --set <key>=<value>` writes a new document, and `owd measures --json` reads
its design measures. A comparison table across documents is a later tool, outside the core.

### CLI and web app

- One request type per stage in `owd-shell`. The CLI's subcommands and the web app's buttons build
  those requests. [changes]
- `owd run` runs every due stage. `owd stage <name>` runs one. [changes; `owd run` exists for the
  chain]
- New commands: `owd structures`, `owd cost`, `owd measures`. New input flags: `--mouth`,
  `--objective`, `--limit`, `--target-level`, `--constraints`, `--cost-weights`, `--rates`,
  `--structures`. [new]

### Document format

Format 0.5. `inputs` gains `run`, `constraints`, `rates`, `structures`, `survey`. `stages` holds
`river`, `sizing`, `reference_line`, `alignment`, `structures`, `water`, `refine`, `volumes`, `cost`,
`measures`, `fits`, `envelope`, `plan_cut`. `options[]` is removed. A 0.4 file opens: its inputs are
read, its alignment's rolling-circle (or lattice) line becomes the design line, its stages are marked
stale, and it is saved as 0.5.

## Testing decisions

- **Each stage alone,** from its fixtures, through its public function. No session in those tests.
- **Schemas** generated from the types; CI fails if a checked-in schema differs.
- **The runner** on the synthetic example end to end, CLI and web engine, same bytes (as today's web
  parity, moved to the shell's request types).
- **Chainage** tests: a reversed centreline gives the same design, with chainage from the mouth.
- **Private real-river cases** run after each logic merge (as now). A new private case covers a whole
  river from the mouth, open river below and weirs above, at the level its data allows.
- Fixed numbers carry labels (`build-process.md`).

## Out of scope

- Hydraulic, sediment, structural and bank-protection design, and bank extraction: seams only.
- Comparing options inside the core.
- A computed error band for volumes; the bands are stated.
- The desktop app (paused).

## Open assumptions (for the maintainer)

1. **Capacity** as a design measure means the governing vessel's size (its deadweight or LOA × beam),
   not traffic. Traffic needs a fleet model OWD does not have.
2. **Run level is the lowest reach level.** A run that is pre-feasibility on its lower half and concept
   above is reported as concept overall, with each reach's level shown.
3. **Rates per reach are optional;** one table for the run is the default.
4. **Haul is a rate per m³ per km,** with the haul distance a setting per reach (no disposal-site
   geometry yet).
5. **Line refinement stays** as an optional stage, since it is how the pool lattice fits "one design".
6. **The structures stage checks levels step down to the mouth** but does not check pool depth over
   the bed upstream of each structure; the hydraulics seam will.

## Tickets

Ordered. One PR each, test first. Each names its stage. Blocked by the ticket(s) in brackets.

1. **Shared value types** (all stages). `owd-format::types`: `Chainage`, `Polyline`, `RiverPolygon`,
   `ElevationModel`, `TerrainSampler`, `WaterProfile`, `Reach`, `Money`, `StageError`. Replace the six
   sampler aliases, the three refusals and the bare profile pairs. Tests: round-trip JSON; schema
   generated. No behaviour change.
2. **Stage storage and schema generation** (runner). `stages.<name> = {version, depends_on, settings,
   output}`; `schemars` output under `schema/stages/`; CI check. Tests: schema drift fails. [1]
3. **River stage** (river). One parse with holes; bank check in one place; CLI and shell use it.
   Tests: polygon with island; banks in opposite directions; the two bank-check copies' cases give one
   answer. [2]
4. **Chainage from the mouth** (river). The mouth input, `--mouth`, orientation, 0.4 files reversed and
   marked stale. Tests: reversed input gives the same design; `RIVER_MOUTH_UNKNOWN`. [3]
5. **Sizing reads and writes `ChannelSize`** (sizing). Remove `sized_shape()`; readers take the type.
   Tests: move sizing tests to fixtures. [2]
6. **One design line** (alignment). `DesignLine` output; method as setting; `options[]` removed;
   bends and cut-offs recorded; format 0.5 migration. Tests: each method from fixtures; a 0.4 file
   opens. [4, 5]
7. **Reference line, fits and envelope as typed stages** (screening). Typed outputs; no own hashes.
   Tests: spec 0016's tests unchanged in result. [3, 5]
8. **Water stage** (water). Typed `WaterProfile` on design-line chainage. Tests: given profile, pools,
   fall. [6]
9. **Volumes stores stations, by reach, with holes** (volumes). Tests: prism in a flat raster; island
   is dry; by-reach sums equal total. [8]
10. **Drawings and reports read stored stations** (outputs). Remove `option_section_walk` re-walks.
    Tests: sheets and report byte-equal before and after on the synthetic example. [9]
11. **Run inputs: objective, limits, target level** (inputs). Types, CLI flags, web fields. Tests:
    schema; refusal of an unknown measure. [2]
12. **Rates table and cost stage** (cost). Tests: hand-worked table. [9, 11]
13. **Constraints: no-go areas and cut-rate multiplier** (alignment, volumes). Impassable cells in the
    re-route; weighted volumes. Tests: line goes round a no-go; multiplier patch moves the line when
    cheaper; weighted volume of a ×2 patch. [6, 9, 12]
14. **Structures seam and given structures** (weirs and locks). The trait, the given-input
    implementation, `Given` mode, locks as a kind. Tests: given set on a synthetic river; levels that
    step up refused. [8, 12]
15. **Structure search over given sites and designs** (weirs and locks). Move the cascade search out of
    optimise; sites as points; designs list; cost in money; objective and limits. Tests: three sites,
    two designs, best set by hand. [14]
16. **Line refinement** (refine). The pool lattice refines the one line. Tests: one pool, less cut,
    R_min met. [15]
17. **Design measures and reached level** (measures). Values, objective, limits, level per reach and
    run, missing inputs, `--strict`, `owd measures --json`. Tests: a failing limit; a reach without
    survey. [12, 15]
18. **Plan drawing for the board** (outputs). Imagery background, chainage from the mouth, bend radii
    against R_min, structures and pool levels, shaded areas, constraints touched. Tests: sheet check
    reads each layer. [10, 13, 15, 17]
19. **Report: objective, limits, levels** (outputs). Spec 0014 report gains them; figures marked by
    level. Tests: report numbers equal document numbers. [17, 18]
20. **One runner for CLI and web** (runner). Request types per stage; `owd run`, `owd stage`; web
    parity on the new stages. Tests: CLI and web give the same bytes on the synthetic example. [17]
21. **Empty seams: hydraulics, sediment, bank extraction** (seams). Types and schemas only; given-input
    fill for hydraulics (the water input) and bank extraction (the river input). Tests: schema. [2]
