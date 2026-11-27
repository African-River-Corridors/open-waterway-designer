# Spec 0032: The map legend

Status: **agreed with the maintainer, 2026-11-27** (design round 4, question 5). Web app, 2D and
3D map.

## Problem

The map now draws many things in colour: the river polygon, bank lines, the design line of each
option, the channel, the cut extent, buildings in the cut (red), near the cut (orange) and on severed
land (purple), pool water, weirs and their labels, constraint stretches, contours, the swept path,
structures. A reader who did not build the study cannot tell what each colour and line means.

## Agreed behaviour

- A **Legend** button on the map, under the Layers button. It opens a small floating legend that
  can be moved and closed.
- The legend lists **only the layers that are ticked and have something drawn** in the current
  view, in the order the Layers list uses.
- Each entry is the symbol as drawn (a line sample, a fill swatch, a point symbol) and a short plain
  name ("Building in the cut", "Pool water", "Weir", "Design line, option B").
- A layer drawn in graded colour (pool depth, sea-bed depth, contours) shows its scale with units.
- The legend updates when a layer is ticked or unticked, and when the view changes what is drawn.
- The legend is drawn on the plan sheet too, when the Sheets view draws the map (spec 0004).

## Acceptance checks

1. Open the example with the default layers: the legend lists exactly the ticked layers with
   features in view.
2. Untick Buildings: its three entries go. Tick it: they come back.
3. Each entry's colour and line style matches the map's, in light and dark themes.
4. Two options: one entry per option line, named.
5. Pool depth shows a colour scale in metres.
6. Pan to where no buildings are: the building entries go.
