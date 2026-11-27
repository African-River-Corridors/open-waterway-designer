# Spec 0028: Design options in one file

Status: **agreed with the maintainer, 2026-11-27** (design round 1, question 10: options are core;
round 2, questions 7 and 8; round 3, questions 1 to 6). Engine, document format and web app.
**Changes spec 0017 (stage contracts):** it said "one run, one design; an option study is a
script", and its part 06 removed `options[]` (format 0.5). This spec keeps the stage contracts and
adds options as a layer around them.

## Problem

An engineer compares options: river-following against rolling circle, two vessels, a lattice
against the window optimiser, least cut against no building flooded. Today a design file holds one
design line, so each option is a separate file, with its project data copied, and nothing in the
app compares them.

## Agreed behaviour

### What an option is

- The file holds the **shared project data once** (step 1, Project and data: the river, the DEM,
  buildings, places, settlements, the water level, the mouth).
- Each **option** is one full chain of steps 2 to 6: vessel and channel, design line, weirs and
  pools, quantities, checks, with its own settings and its own **objective and limits** (the run
  inputs of spec 0017). Each chain is one "run" as spec 0017 defines it: the stages, their contracts
  and their tests do not change.
- Each option has a **name** (for example "River-following, class IV") and a colour.
- A change to the shared data makes every option out of date; a change inside an option makes only
  that option out of date.

### Making an option

**New option** (in the option picker and the File menu) asks first **what differs**, ticks of:

- vessel and channel;
- design line method and its settings;
- weirs and pools method and settings;
- the objective and limits of the run.

It copies the active option, opens the settings of the ticked parts for the engineer to change,
and names the option from what differs (editable). **Delete option** and **Rename option** sit in
the picker. The last option cannot be deleted.

### Working on options

- **One active option.** A picker at the top of the steps panel (spec 0021) names it. The steps,
  their settings windows (spec 0022) and the results panel's other tabs (spec 0023) show the
  active option.
- **The map** draws every option's line in its own colour, from a set kept apart from the layer
  colours; the active one bold, the others thin. **Only the active option** draws its channel, cut,
  buildings in the cut and pools; the others draw their line only (design round 4, question 4). The
  Options tab can hide an option from the map.
- **Run** (spec 0019) runs the active option. Beside it, **Run all options** runs every option that
  is out of date, one after another.

### Comparing

The **Options** tab of the results panel:

- one column per option; rows are the design summary numbers (spec 0023);
- the best value in each row is marked (least cut, least land flooded, fewest buildings, and so on;
  a row with no "best", such as the vessel, is not marked);
- a first row says **what differs** (vessel, line method, weirs method, settings changed, objective);
- each column has **Make active** and **Mark as preferred**. One option at most is preferred.

### Drawings and report

The drawings and the report show the **preferred** option in full, plus **one table comparing all
options** (the Options tab's rows). With none preferred, the active option.

### Files and the command line

- The document format gains a list of options around the run; format 0.6. A 0.5 file opens as one
  option, named after its line method, and nothing in it moves.
- FMT-2 (the split design file, grilled with the maintainer on 26 November) also moves the format to
  0.6. The two need **one 0.6 plan**: the architecture check before the build (coordinator task
  T-256081) settles it, with ADR 0015 and spec 0017.
- `owd run <design> --option NAME` runs one option; with no `--option`, every option that is due.
  `owd measures --json` gives every option's measures.

## Acceptance checks

1. Open a 0.5 design: it shows one option, named after its method; every number is as before.
2. New option, tick "design line method": a copy is made; set it to river-following; Run: two lines
   on the map, the active one bold.
3. The Options tab has two columns; each number equals that option's Summary tab; the lesser cut is
   marked.
4. Change the vessel in option B: only option B goes out of date. Change the water level: both do.
5. Run all options: both run, one after the other.
6. Mark option B as preferred: the report and drawings show B in full, and the comparison table
   lists A and B.
7. Delete option A: one option left; Delete is then greyed.
8. Save and reopen: the options, their names, colours, the active and the preferred one are back.
9. Each option's stages give the same numbers as the same run in its own one-option file (the
   stage contracts hold).
10. `owd run --option B` runs only B.
