# Spec 0021: The steps panel

Status: **agreed with the maintainer, 2026-11-27** (design round 1, questions 2, 3 and 9; round 2,
questions 1, 2 and 6). Web app. Replaces the left panel. Needs spec 0019 (the menu bar) for its
Panels menu entry; its settings windows are spec 0022, its tables move to spec 0023 (the results
panel).

## Problem

The left panel is one long scroll in the reverse of the engineer's order of work: Stages first
(with about ten optimise settings and three long tables open), then the design line card, Checks,
Project, Data, and the vessel last. The engineer must scroll past settings and tables to find the
next thing to do, and the panel takes a fixed strip of the screen from the map.

## Agreed behaviour

### Seven steps, in the order of work

1. **Project and data**: the project folder, the river (polygon, centreline, banks), the DEM,
   buildings, places and settlements, the water level and its fall, the river mouth, the downstream
   point. "Start from a river" (spec 0012) sits here as one way to make the data.
2. **Vessel and channel**: the Rule Sets, the vessel (catalogue or custom), what fits, and the
   channel it sizes (bottom width, depth, R_min).
3. **Design line**: the alignment method (rolling circle, re-route, river-following) and its line.
4. **Weirs and pools**: the optimise stage (weirs and pool lattice, or the window optimiser).
5. **Quantities**: the volumes stage: dredge, dry cut, cut faces.
6. **Checks**: the checks of the design line and the verdict.
7. **Drawings and report**: the Sheets view, the report and the exports.

Steps never lock. The engineer can open any step at any time and go back (for example from step 4
to step 3). A step that needs an earlier one says so in one line ("Needs a design line: run step
3") and its Run button is greyed.

### Each step

- **Closed**: one row: the step number, its name, its status, and a one-line summary of its result
  (for example "Rolling circle, 18.4 km, least radius 420 m").
- **Status**: one of *not run*, *up to date*, *out of date*, *running*. Each is a word and a shape
  and a colour, as the stage chips today.
- **Open**: one step open at a time. It shows its main choice (for example the alignment method),
  its Run button, its short result, and a gear button that opens its settings window (spec 0022).
  Long tables are not here: a "Show in results" link opens the right tab of the results panel
  (spec 0023).

### Which steps run by themselves

A fixed rule:

- **Vessel and channel** and **Checks** run by themselves after a change (they take under a
  second).
- **Design line**, **Weirs and pools** and **Quantities** wait for Run, because on a long river they
  take seconds to minutes. When one is out of date, its row says "out of date" and the menu bar's
  Run button (spec 0019) runs it and the later ones that are due.

### The panel floats

- It starts on the left edge of the map, full height under the menu bar, about 340 px wide.
- It can be dragged anywhere over the map and resized by its edges. Its place and size are kept in
  the browser for next time.
- **Minimise** turns it into a slim strip of step numbers 1 to 7, each in its status colour. Click a
  number to open the panel at that step.
- **Hide** removes it; the Panels menu (spec 0019) brings it back.

## Acceptance checks

1. Open the example: the panel shows seven steps in the order above, step 1 open.
2. Every control of today's left panel is reachable from a step or from its settings window
   (spec 0022); none is lost.
3. Open step 4, then step 3: step 4 closes, step 3 opens; nothing is locked.
4. Change the vessel: step 2 runs by itself and shows *up to date*; steps 3 to 5 show *out of
   date*; step 6 updates by itself.
5. Press Run in the menu bar: steps 3, 4 and 5 run in order; each shows *running* then *up to date*.
6. Drag the panel to the right of the map and reload: it is still on the right.
7. Minimise: a strip of seven numbers in status colours; click 4: the panel opens at step 4.
8. Hide it from the Panels menu: the map fills the window; tick it again: it comes back.
9. With no design line, step 4 says "Needs a design line: run step 3" and its Run is greyed.
