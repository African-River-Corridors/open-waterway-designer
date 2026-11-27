# Spec 0029: Weir sites from the engineer

Status: **approved by the maintainer, 27 November 2026.** Agreed in grill rounds
on 27 November 2026 (weirs, rounds 1, 3 and 4). Builds on spec 0017 (stage contracts: the weirs and
locks stage) and ADR 0006 (the water level belongs to the river). Where each lock and weir stands
on the channel is spec 0050 (lock and weir lined up with the channel).

Marks used below: **[exists]** OWD does this today, **[changes]** it exists but changes,
**[new]** it does not exist.

## Problem

Today the weir search (the cascade: an exact least-cost search, `owd-core/src/optimise/cascade.rs`)
makes its own candidate sites when it is given none: one every `site_step_m` (500 m) of river. It
then picks weirs among them by cut, structure cost and flooded land. Those sites come from
nowhere an engineer would choose. On a winding river most of them stand in bends.

The engineer already has weir sites, chosen by judgement in earlier work. OWD should search only
those sites, choosing which to use and the lift at each. It must never make up a site.

## The agreed rules

1. **No sites, no weir search.** [changes] With no weir-site list, the optimise stage chooses no
   weirs. The water level is the one the inputs give (a level, a fall, a table or a pool table)
   [exists]. The result says so: "no weir sites given: no weirs; the water level is the inputs'".
   `site_step_m` is removed.
2. **The engineer gives the sites.** [new] The input is a list of weir sites. Each site has an id,
   a place (a point, or a river chainage from the mouth) and a use:
   - `fixed`: a weir must stand here;
   - `candidate`: the search may put a weir here.
3. **The search goes through those sites and the lifts.** [changes] It uses every `fixed` site,
   chooses among the `candidate` sites, and sets the lift at each from the lift menu, at least
   cost. The cost is today's (cut, structures, flooded land and buildings). This is the `Search`
   mode of the weirs and locks stage of spec 0017, with `fixed` sites added. Its `Given` mode
   (sites and levels both given) is unchanged.
4. **The largest lift is a limit the engineer sets per project: `max_lift_m`, default 10 m.**
   [new] A lift on the menu above it is refused before the search starts. The message names the
   lift and the limit.
5. **The top of the waterway stays as today.** [exists] The top pool runs to the end of the line.
   The engineer gives a floor (`terminal_min_level_m`), normally the river level at the end. The
   search chooses the top pool level, and the floor is a hard limit.
6. **Spacing.** [exists] The search keeps two weirs at least `min_spacing_m` apart. Two `fixed`
   sites closer than that are refused before the search starts, with both ids.

## Out of scope

- **Where the lock and weir stand on the channel** (a straight for the lock, the weir across the
  river, the lock in a cut-through and the weir on the old loop): spec 0050.
- **The lock and weir model** (chamber, footprint, cost): another model will plug into the
  structures seam of spec 0017. Until then the map keeps its placeholder blocks, marked
  "placeholder model, not a design".
- **A water level with no weirs that follows the river** (for example the river bed plus an
  assumed depth, or levels from a hydraulic model). Agreed for now: the engineer gives a sloped
  table or a fall as the water input, as today. Nothing new is built for it.

## What the engineer sees

1. `owd import DOC --weir-sites FILE` takes a GeoJSON of points (properties `id`, `use`) or a CSV
   of `id,river_km,use`. Points are put on the river centreline. A point more than `site_snap_m`
   (default 200 m) from it is refused, with its id. A missing `use` is `candidate`.
2. `owd optimise` prints the sites it used:

   ```text
   weir sites: 12 from the engineer (3 fixed, 9 candidate); 5 used
     weir  site  use        river km  lift m  pool m
     1     W03   fixed         4.200     2.0     2.0
     2     W07   candidate     7.300     2.0     4.0
     3     W09   fixed         9.900     1.5     5.5
     ...
   ```

3. The design file keeps each chosen weir's site id and use beside its chainage and lift.
4. On the map, unused candidate sites are small hollow marks with their id. A weir's label is its
   number, site id, river km and lift ("Weir 3 · W09 · km 9.9 · lift 1.5 m"); its card adds the
   levels either side. Numbers run from the mouth: pool 0 is the water below weir 1, weir k stands
   between pool k−1 and pool k, and the lock at weir k is lock k.

## Acceptance checks

All on synthetic rivers made in the test. No real place or result.

1. **Fixed sites are used.** A straight river 20 km long, rising 1 m per km. Sites: `A` fixed at
   km 5, `B` candidate at km 10, `C` fixed at km 15. Lifts 1 to 10 m. The chosen weirs include `A`
   and `C`, with a flood cost of 0 and with a high one.
2. **No site is invented.** Same river, three candidate sites. Every chosen weir has one of the
   three ids; no weir stands at another chainage.
3. **No sites, no weirs.** Same river, no site list, a water fall given. The stage chooses no
   weirs, the water level equals the input's at every station, and the result holds "no weir sites
   given". `site_step_m` in a settings file is refused, naming it.
4. **Largest lift.** A lift menu of 2, 5, 12 m with `max_lift_m` 10 is refused before the search;
   the message holds "12" and "10". With the default (10 m) and a menu up to 10 m it runs.
5. **Top floor holds.** With `terminal_min_level_m` set, the top pool is never below it, and the
   search is free to go above it.
6. **Spacing.** Two fixed sites 100 m apart with `min_spacing_m` 500 m are refused before the
   search, naming both ids. Two candidate sites 100 m apart are never both used.
7. **Import.** A GeoJSON of three points (one 300 m off the centreline) is refused naming that
   point's id. A CSV of three rows imports, and a row with no `use` is a candidate.
8. **Round trip.** A site list survives save and load of the design file, with ids and uses; the
   chosen weirs keep their site ids.
9. **Numbering.** Three chosen weirs: pools are numbered 0 to 3 from the mouth, weir 2 stands
   between pool 1 and pool 2, and its label holds its site id and river km.
