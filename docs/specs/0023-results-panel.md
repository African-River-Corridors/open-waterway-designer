# Spec 0023: The results panel and the design summary

Status: **agreed with the maintainer, 2026-11-27** (design round 1, questions 5 and 11; round 2,
questions 4, 5 and 13). Web app. Next to build after the menu bar (spec 0019) and the steps panel
(spec 0021).

## Problem

The results of a study are spread out: the design line card and the checks in the left panel; the
pools and flooded land, the windows of the window optimiser, the cut-throughs and the bends as long
tables inside the Stages list; the constraint stretches in a drawer. There is no one place that
gives the key numbers of the study, and a table row does not take the engineer to its place on the
map.

## Agreed behaviour

A **results panel** floats over the map. It starts along the bottom of the map, about a third of
its height, and can be dragged, resized, minimised to its tab row, and hidden from the Panels menu
(spec 0019). Its place and size are kept for next time.

### Tabs, in this order

1. **Summary** (opens first): the design summary, below.
2. **Options**: the design options side by side (spec 0028, design options in one file). Greyed until that
   spec is built.
3. **Weirs and pools**: one row per pool: pool level, lift, flooded land (ha), above bank-full (m),
   buildings flooded, settlement flooded; the totals above the table.
4. **Stretches**: the constraint stretches (today's drawer).
5. **Windows**: the window optimiser's windows (start, end, cost before, cost after, kept or why
   not).
6. **Cut-throughs**: the river-following cut-throughs (saves, stay cost, cut cost, face, decision,
   and the force choice: by the rule, on, off), and the forced stretches and bends.
7. **Checks**: every check of the design line and the verdict.

A tab with nothing in it is greyed, with a line saying why ("Run step 4 with the window optimiser").
**Click a row** and the map flies to that place and marks it; the profile and cross-section panels,
when open, go to its chainage.

### The design summary

Grouped, each number with its unit, and "–" with the reason when its step has not run:

- **Verdict** and its reason; vessel and rule set.
- **Line**: length, least radius, bends held at the least radius.
- **Cut**: dredge volume, dry cut volume, tallest cut face.
- **Weirs**: how many, total lift, highest lift; pool count.
- **Land**: land flooded (ha), buildings flooded, buildings in the cut.
- **Stretches**: how many widenings, deepenings and bend cut-throughs.
- **Data and limits**: the standing caveats of the data (for example low-confidence banks, a DEM
  that does not cover the river). They move here from the "Data notices" list in the map's corner,
  which goes (design round 4, question 3).

A number from a step that is *out of date* is shown greyed with "out of date" beside its group.

## Acceptance checks

1. Open the example and run all steps: the results panel opens on Summary with every group filled.
2. Each summary number equals the same number in today's app (design line card, flood totals,
   checks, stretches).
3. Change the vessel: the Line, Cut, Weirs, Land and Stretches groups show greyed "out of date".
4. Before step 4 has run, Weirs and pools is greyed and says why.
5. Click a pool row: the map flies to that pool; with the profile open, its station moves there.
6. Change a cut-through's force choice in the Cut-throughs tab: step 3 goes out of date, as today.
7. Minimise: only the tab row shows. Hide and show from the Panels menu: same place and size.
8. No long table remains in the steps panel.
9. A design with data notices lists them under Data and limits; the map's corner has no notices list.
