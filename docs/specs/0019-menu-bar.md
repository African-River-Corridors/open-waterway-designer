# Spec 0019: The menu bar

Status: **agreed with the maintainer, 2026-11-27** (design round 1, questions 5 and 7, and the
note on question 3: "We also need to clean up the menu bar"). Web app.

## Problem

The header has 15 buttons in one row: New, Open the example, Open, Open project, Start from a
river, Save, Save as, Sheets, Profile, Section, Export GeoJSON, Export CSV, Run stale, Imagery and
Theme, then three lines of small text (web version, file name, study title, engine version). On a
laptop screen the buttons wrap into two or three lines and push the map down.

## Agreed behaviour

The header becomes one menu bar with four groups, left to right:

1. **Study name** and its state: the study title, then the file name, then "unsaved" when there are
   unsaved changes. Click the name to rename the study.
2. **File** menu: New, Open project, Open, Open the example, Start from a river, Save, Save as, then
   a divider, then Export GeoJSON and Export CSV. Shortcuts stay: Ctrl/Cmd+N, O, S, Shift+S.
3. **Panels** menu: a tick list that shows or hides each panel: Steps, Results, Profile, Section,
   Sheets. A ticked panel is showing. Hiding a panel keeps its place and size for when it comes back.
4. **Run** button: one button. It runs the steps that wait for Run and are out of date, in order
   (today's "Run stale"). Greyed with the words "Up to date" when nothing is due. While running it
   shows the step and "Cancel".

At the right end: the Imagery control, a Settings menu (Theme, and later other user settings) and
a small "?" with the engine version, the "web version: nothing is uploaded" line and the link to
the desktop app.

## What the screen shows

- One row, never wrapping, at 1280 px wide and above. Below that, the File and Panels menus keep
  their place and the study name is cut short with "…".
- **Narrow screens** (design round 4, question 7): the app is made for screens 1280 px wide and up.
  In a narrower window (a tablet or a phone) one line under the menu bar says so; the map can still
  be looked at, and nothing else is promised.
- "Screening — not for construction" stays in the footer.

## Acceptance checks

1. At 1280 × 800 the header is one row; the map starts directly under it.
2. Every action of today's header is still reachable: each of the 15 buttons has a menu item or
   button, and each does what it did.
3. The keyboard shortcuts New, Open, Save and Save as still work.
4. Panels menu: untick Profile and the profile panel hides; tick it and it comes back where it was.
5. Run is greyed and says "Up to date" when no step is due; with a step due it runs it and offers
   Cancel while running.
6. Unsaved changes show "unsaved" beside the file name; Save clears it.
