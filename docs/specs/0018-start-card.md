# Spec 0018: The start card

Status: **agreed with the maintainer, 2026-11-27** (design round 1, question 8). Web app.
Part of the UI workflow design (specs 0018 onward).

## Problem

When nothing is open, the web app shows an empty map of the world and the full left panel: Stages,
optimise settings and empty tables. A new user cannot see where to start. The ways to start are
spread across the header ("New", "Open the example", "Open", "Open project", "Start from a river")
and a "Reopen" notice bar.

## Agreed behaviour

When no design is open (the app has just loaded, or the user pressed New and has not loaded any
input), a **start card** shows in the middle of the map. It offers four choices, in this order:

1. **Open a project folder** — the same action as "Open project" today (spec PROJ-1, project
   folders). In a browser that cannot open a folder (Firefox, Safari), the button is greyed and says
   "Chrome or Edge only".
2. **Start from a river** — opens "Start from a river" (spec 0012) in the step panel.
3. **Open the example** — opens the synthetic example river and its DEM.
4. **Reopen <last project title>** — only when the browser remembers a last project. It replaces the
   "Reopen" notice bar, which goes. A small "Forget it" link sits beside it.

Below the four choices, one plain line: "Or open a single design file", which runs Open.

The card closes as soon as a design is open, and does not come back until the next empty state.
The user can close it with its close button or Escape; then the empty map shows, and the File menu
(spec 0019, the header) still has every action.

## What the screen shows

- A card, about 420 px wide, centred over the map, with the OWD mark and the title "Start a study".
- Each choice is one row: a button, and one short line under it saying what it does
  (for example "A folder with owd-project.json, your design and its DEM. Nothing is uploaded.").
- The map behind it stays visible and dimmed slightly.
- With the card showing, the step panel shows only its step names, all "not started".

## Acceptance checks

1. Load the app with nothing remembered: the card shows with three choices (no Reopen row).
2. Press each choice: it does the same thing as today's header button, and the card closes once a
   design is open.
3. With a remembered project: the Reopen row shows its title; the old "Reopen" notice bar does not
   show. "Forget it" removes the row and the browser does not offer it again.
4. In a browser with no folder picker, "Open a project folder" is greyed and says "Chrome or Edge
   only".
5. Escape closes the card; New brings it back.
6. Open a design, then press New: the card shows again.
7. "Open the example" works on the preview server as well as the built site (today the preview at
   `/owd/app-files/` fails with "the file is not valid JSON").
