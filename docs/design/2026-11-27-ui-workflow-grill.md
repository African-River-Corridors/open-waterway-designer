# UI and workflow grill, 2026-11-27

Three rounds with the maintainer, about the web app. The topic: how a river engineer moves from a
river to a finished concept design, and what each screen shows. The starting point was the
integration branch of 27 November (pull request 576).

## Decisions

- **The user** is a river engineer making a concept (screening) design. "Finished" means a design
  line, weirs and pools, quantities, drawings and a report.
- **The order of work** has seven steps: project and data; vessel and channel; design line; weirs
  and pools; quantities; checks; drawings and report. The engineer loops back often, so steps
  never lock.
- **The worst problems today** are the long left panel with its tables, and the crowded header.
  The build order is: menu bar, then the steps panel, then the results panel with the design
  summary.

## Specs from the grill

| Spec | Name | Round |
|---|---|---|
| 0018 | The start card | 1 |
| 0019 | The menu bar (File, Panels, one Run) | 1 |
| 0020 | One message bar | 1 |
| 0021 | The steps panel | 1, 2 |
| 0022 | Step settings windows | 1, 2, 3 |
| 0023 | The results panel and the design summary | 1, 2 |
| 0024 | The vessel in the bends (swept path) | 2 |
| 0025 | Sea chart layers | 2 |
| 0026 | An offshore structure with its 3D model | 2 |
| 0027 | Easier 3D navigation | 2 |
| 0028 | Design options in one file (changes spec 0017) | 2, 3 |

## Open questions

1. **Where an offshore structure lives in the design file** (spec 0026). The builder proposes a
   field to the maintainer before writing code. It must fit the "structures" seam of spec 0017
   (stage contracts).
2. **What step 7, Drawings and report, shows in the app.** The Sheets view exists. The reports
   (spec 0014) are still being built. We did not settle what this step shows, or its order of
   actions.
3. **The data notices in the map's corner.** These are the small "Data notices" list at the bottom
   right of the map. Should they also go into the message bar (spec 0020)? Not asked.
4. **Option colours against layer colours.** Spec 0028 gives each option its own colour. The
   channel, cut, buildings-in-cut and pool layers also use colour. We need one palette that keeps
   them apart.
5. **Items not chosen now:** a map legend, undo, and editing by hand on the map (moving a weir,
   dragging the line). None was picked in round 1. They are parked, not refused.
6. **The swept path model** (spec 0024). The acceptance check uses B + L²/(8R) with bow and stern
   on the line. The maintainer should confirm this is the right check for a concept design. The
   other choice is a PIANC bend-widening rule.
7. **Narrow screens.** The specs aim at 1280 px and wider. We did not cover tablets or phones.
8. **Official sea charts (ENC)** are later, and only if a licence allows (spec 0025).
9. **Items being built elsewhere, not specced here:** weir labels on by default, weirs missing on
   the map, a road layer, and 3D bridges.
