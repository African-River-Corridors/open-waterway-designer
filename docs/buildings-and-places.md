# Buildings, places and settlements

A design can name three extra files: building footprints, points of interest (places, such as
communities, bridges and quays) and settlement polygons. OWD draws them on the map, counts the
buildings and the settlement land each pool floods, prices the buildings in the weir search when you
give a cost per building, and reports the buildings the channel's cut takes, comes near, or cuts off.

## How the design holds them

The design never copies the features. It records each file as it records the DEM: a path from the
design's own folder, the SHA-256 of the file, the number of features and the file's CRS.

```json
"buildings": {"path": "../inputs/buildings.geojson", "sha256": "…", "count": 36012,
              "crs": "WGS 84", "min_confidence": 0.7, "min_area_m2": 0, "near_cut_m": 50}
```

Why: 36,000 footprints are about 8 MB of GeoJSON, and a design file is already large. A copy would
also go stale the day the source is updated. In memory OWD keeps one point per feature (a point
inside it, in the project CRS), its area, its confidence, its name and kind, and, for places and
settlements, its other properties. Settlements keep their outline too. Building footprints are not
kept: the map draws them from the file.

The checks are the DEM's. A file that is not where the design says is **missing**; a file whose
SHA-256 is not the one recorded has **changed**. Either is said when the design opens (on the command
line on stderr, in the app as a warning and in the Project panel), and what reads the file goes
without it, or refuses by name (a building cost does). Load the file again to record the new one.

Loading a file leaves every stage fresh. The optimise stage reads the buildings only when its
settings ask for them (a building cost, or the flood terms with buildings loaded); then the file's
hash and the building filters are part of its hash, and a change makes it stale.

## The files

Each is GeoJSON or a GeoPackage, in WGS84 or WGS84 / UTM. The project CRS must be set first (import
the river first).

- **Buildings** (`inputs.buildings`): Polygons or Points. `area_m2` and `confidence` (0 to 1) are read
  when given; with no `area_m2` the polygon's own area is used.
- **Points of interest** (`inputs.points_of_interest`): Points (a Polygon or a LineString is labelled
  at a point inside it). `name` and `kind` are read when given and must be text; every other property
  is kept and shown on a click. Kinds: `community`, `bridge`, `quay`, `loading_site`, `ferry`,
  `intake`, `structure`; any other kind is shown as "other", with its own word in the card.
- **Settlements** (`inputs.settlements`): Polygons, with any properties.

## Building filters

- `min_confidence` (default 0.7): a building below it is **counted apart**, never dropped and never
  costed. Every count says how many were counted apart.
- `min_area_m2` (default 0): the same for a smaller building.
- `near_cut_m` (default 50): the distance from the cut within which a building is reported as near it.

Set them with `owd import --building-min-confidence X --building-min-area X --building-near-cut X`, or
under Buildings in the app's Data panel.

## On the map

The Layers list has **Places** (on by default), **Buildings** and **Settlements** (both off). Each is
greyed with "none loaded" until a file is loaded, and "file missing" when the design names one that
is not there. The ticks are remembered on this machine and never change the design.

- Buildings: a density further out than zoom 12, dots from 12, footprints from 14. With Pool water on,
  the buildings the pool water covers at least `building_min_depth_m` deep (default 0.3 m) turn
  orange, and the Land table gains a column of buildings flooded per pool. Their edge shows the cut:
  red in the cut, light orange near it, purple on land a cut-through severs.
- Places: a mark for the kind and the name beside it. Where two names would collide the larger place
  (by `area_ha`) is placed first.
- Settlements: a thin outline with a light fill.

A click on any of them opens a card: for a building its area, confidence, the ground level, the pool's
water level there and the depth (or "dry"), its distance to the design line and to the cut's daylight
line; for a place its name, kind and properties, the ground, the water, the distance to the line and
the chainage of the nearest point; for a settlement the share of it the pool water covers.

**Names without a font server.** The map has no glyph server, so the names are small pictures the page
draws with the browser's own text, in the app's Public Sans font with a dark halo, one picture per
distinct kind and name. No request leaves the page. Limits: a script the machine has no font for (for
example some African scripts) shows the browser's fallback boxes; a name over 28 characters is cut
with "…" (the card has it whole); names do not curve along lines.

**Two rules for "flooded".** The map's pool water is worked out on the DEM's own cells, and its water
does not pass a weir. The weir search counts on its own grid (`flood_cell_m`), and its water may pass
the flank of a weir (no bund is assumed). Near a weir the map's count of buildings can differ from the
search's; the cost and the stage's report use the search's rule.

## In the weir search

`building_cost` (`owd optimise --building-cost X`, or the Optimise row in the app) is the cost of one
building a pool floods, in the units of the cut cost. It is the engineer's price, for example a
resettlement cost, not a fact. Off (0) by default: then the search is the one with no buildings, byte
for byte. A building counts when its point is in land the pool floods by the weir search's own rule
and the water there is at least `building_min_depth_m` deep (`--building-min-depth`, default 0.3 m),
so a puddle does not count. Only buildings that pass the filters are costed.

The count is a table built once, like the flooded hectares: each building's lowest flooding level is
one number, so the count over a reach at a level is two reads of a prefix sum. Building it is
`O(B log S + (S + 1) L)` for B buildings, S candidate sites and L levels, after one pass over the flood
grid's cells to place them. Measured (release build, `cargo test --release -p owd-core --test
flood_buildings -- --ignored`): on 240 km of line with a 1,500 m flood distance and 25 m cells (1.17
million cells), 40,000 buildings, 479 sites and 73 levels, placing the buildings takes 119 ms and the
count table 0.5 ms; the flood field itself takes 10.7 s. The search takes 0.70 s without the building
term and 1.19 s with it. A non-zero building cost with no buildings loaded, or with no DEM, is
refused in plain words.

With `--flood-report` (or any flood setting) and buildings loaded, `results.cascade.flood.buildings`
says per pool and in total: the buildings flooded, those counted apart, the flooded buildings by
confidence band, the building cost, and the first 20 flooded buildings with their id, longitude and
latitude, pool, chainage and depth, so you can go and look. With settlements loaded it also gives the
settlement land each pool floods (`settlement_flooded_ha`), by the same rule. That is a report, not a
cost: to price it the search would need a third table of the same kind (settlement cells per break
and level) and a cost per hectare of settlement land.

## Buildings and the channel

`owd volumes`, `owd report` (the screening report's Buildings section) and the map read, from the
volumes stage's own stations:

- **in the cut**: the building, as a disc of its own area about its point, reaches inside the cut's
  daylight lines (the water's edge where a side has no cut);
- **near the cut**: within `near_cut_m` of them;
- **severed**: on land that a cut-through (the design line out of the river) and the bypassed loop of
  the river close round, found on a 10 m grid around each cut-through.

These are counts and lists, not costs. To use them in the line search a later ticket would need the
same tests inside the lattice's section cost (a building per station, priced when the footprint at that
station takes it), and an island test per candidate line, which is costly; a cheaper start is a cost
per building taken, added where the lattice already prices land outside the river.

## The data and its limits

Machine-detected footprints (for example Google Open Buildings, CC BY 4.0 or ODbL by version) are
**not a survey**. Each has a confidence; low-confidence shapes are often not buildings, and some
buildings are missed. The imagery behind them has a date; buildings built since are not there. Keep the
source's licence and attribution with the file, and say in any report what the counts are: a screening
figure from detected footprints, with the threshold used.

## Places, later: 3D models

A place may carry `model` (today only `"block"`), `bearing_deg` (clockwise from north), `length_m`,
`width_m`, `height_m` and `deck_level_m`. A `bridge` with a length and a width is drawn as a deck
across the river at `deck_level_m`; a `quay` as a slab along the bank, `height_m` tall. Both are
labelled "placeholder model, not a design", like the weirs and locks, and stand up in the 3D view the
same way (the map measures a block's height from the terrain at its middle).

A real model file could later be attached the same way as the buildings: a glTF file named by path and
SHA-256 from the place's `model` property, placed at its point, turned to `bearing_deg` and set at
`deck_level_m`. That needs a 3D layer that draws glTF on the terrain (MapLibre custom layers can), a
size limit, a check that the model's units are metres, and the licence of each model kept with it.
