# Spec 0050: Lock and weir lined up with the channel

Status: **approved by the maintainer, 27 November 2026; build after spec 0029.** From the maintainer's answers in grill rounds on 27 November
2026 (weirs, rounds 1 and 3). Builds on spec 0029 (weir sites from the engineer) and spec 0017
(stage contracts: the structures seam).

Marks used below: **[exists]** OWD does this today, **[changes]** it exists but changes,
**[new]** it does not exist.

## Problem

Today the map draws each lock and weir as one block, centred on the design line where the water
level steps, turned to the line's heading there (`owd-format/src/design/structures.rs`). Where the
line bends, the lock sits across the bend. Where the line cuts through a meander neck, the weir is
drawn across the cut, on dry land, and the river loop beside it has no weir at all. The model does
not look like a scheme an engineer would draw, and quantities taken from it later would be wrong.

This is a separate step from the weir search. The search (spec 0029) sets which sites are used and
their levels. This step places the lock and the weir of each chosen site on the channel, so the
model is realistic, and shows the engineer where a site does not fit.

## The agreed rules

1. **The lock stands on a straight.** [new] The lock is placed on the design line, centred on the
   middle of the straightest stretch of `lock_straight_m` within `lock_search_m` (default 500 m) of
   the site. A stretch counts as straight when no point of the line is more than `straight_tol_m`
   (default 1 m) from the chord between its ends. `lock_straight_m` defaults to today's
   `weir_straight_m` (two vessel lengths); the lock model will set it per lock later.
2. **The weir spans the river.** [new] The weir stands on the river centreline, square to the
   river (not to the design line), and runs bank to bank across the river polygon.
3. **At a cut-through, the lock goes in the cut and the weir on the old loop.** [new]
   - A cut-through is a run of the design line outside the river polygon that joins the river at
     both ends, with a loop of river between the two joins.
   - When a site lies on such a loop, or within `cut_search_m` (default 1,500 m) of a
     cut-through, the lock stands in the cut (by rule 1, searched along the cut) and the weir
     stands on the bypassed loop, at the loop point nearest the lock.
   - The cut is a lock canal. The loop carries the flood and holds the upper pool behind its weir.
4. **Away from a cut-through, the lock and the weir stand side by side** [changes]: the weir across
   the river from the outer lock wall to the far bank, as the placeholder model does today, but at
   the lock's place from rule 1.
5. **Where the levels step.** On the design line the level steps at the lock. On the river (for
   flooded land and the river's own level) it steps at the weir.
6. **Report, never force.** When no straight is found, the lock is placed at the site, and the site
   is flagged with the best stretch found (its offset from the chord). When the weir's cross-line
   does not meet both banks, the site is flagged. OWD does not move the site; the engineer does.

## Out of scope

- The size and form of the lock and weir: the lock and weir model, through the structures seam of
  spec 0017. Until it lands, the blocks keep today's placeholder sizes.
- Construction quantities. They come once the lock model gives a form; this spec gives them the
  right place and heading.
- Choosing sites or levels: spec 0029.

## What the engineer sees

1. On the map, each lock on the design line and each weir across the river, each at its own place
   and heading. At a cut-through, the lock in the cut and the weir on the loop, with a thin line
   joining them and one label (spec 0029: "Weir 3 · W09 · km 9.9 · lift 1.5 m").
2. A site that does not fit is drawn in the warning colour. Its card says why: "no straight of
   180 m within 500 m: best stretch leaves its chord by 6.7 m", or "weir does not reach both banks".
3. A table, printed by `owd optimise` after the weir list and stored in the design file:

   ```text
   locks and weirs on the channel
     weir  site  lock at design km  straight      weir at river km  weir in river  cut-through
     1     W03   4.180              yes (0.4 m)   4.200             yes            no
     2     W07   6.950              yes (0.2 m)   7.420             yes            lock in cut, weir on loop
     3     W09   9.550              NO (6.7 m)    9.900             yes            no
   ```

4. Checks in the design file: `checks.lock_on_straight` and `checks.weir_in_river`, each with `ok`
   and, per failing weir, its number, site id and the measured value against the limit. A fail is
   a warning in the check list, as `no_self_crossing` is.

## Acceptance checks

All on synthetic rivers made in the test. No real place or result.

1. **Lock moves to the straight.** A river of two long straights joined by an arc of radius 600 m.
   A site at the middle of the arc, 300 m from the straight, `lock_straight_m` 180 m: the lock is
   placed on the straight, and its offset from the chord is under 0.01 m.
2. **No straight near.** The same site with `lock_search_m` 50 m: the lock stays at the site, the
   check fails and names it, and the offset matches the sagitta of a 180 m arc of radius 600 m
   (6.74 m) to 0.1 m.
3. **Weir square to the river.** A straight river 80 m wide, a design line at 10° to it: the weir
   is square to the river centreline to 0.5°, and its length is 80 m to 1 m.
4. **Weir on land.** A site whose cross-line does not meet both banks: `weir_in_river` fails and
   names it.
5. **Lock in the cut, weir on the loop.** A river with one meander loop of 3 km round a neck 600 m
   wide, a design line that crosses the neck, and a site on the loop. The lock is on the design line
   inside the cut. The weir is on the loop's centreline, inside the river polygon. The design line's
   level steps at the lock. The river's level steps at the weir. Land beside the loop above the weir
   counts as flooded at the upper pool level.
6. **No cut-through near.** The same site moved 5 km from any cut-through: the lock and the weir
   stand side by side.
7. **Levels unchanged.** On every case above, the pool levels and the chosen sites equal those of
   spec 0029's search: this step moves structures, never levels.
