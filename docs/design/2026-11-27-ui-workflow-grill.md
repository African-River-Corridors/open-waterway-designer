# UI and workflow grill, 2026-11-27

Four rounds with the maintainer, about the web app. The topic: how a river engineer moves from a
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
| 0028 | Design options in one file (changes spec 0017) | 2, 3, 4 |
| 0030 | The structures and models layer, a published standard | 4 |
| 0031 | Step 7, Drawings and report | 4 |
| 0032 | The map legend | 4 |

## Settled in round 4

- An offshore structure is kept in a general structures and models layer. The layer file is
  referenced by the design and follows a published standard (spec 0030). This replaces the storage
  part of spec 0026.
- Step 7 lists the sheets with previews, makes the report (PDF, HTML, Word) and gives the exports
  (spec 0031).
- The data notices move into the Summary tab, as "Data and limits" (spec 0023).
- Each option's line has its own colour. Only the active option draws its channel, cut, buildings
  and pools (spec 0028).
- A map legend is specced (spec 0032). Undo and editing by hand stay parked.
- The swept path shows the hull envelope, plus dashed lines at the PIANC WG 141 width (spec 0024).
- Screens 1280 px wide and up only, for now (spec 0019).

## Open questions

1. **3D bridges and the structures layer.** The bridges lane should write its data as `kind: bridge`
   structures under spec 0030. The coordinator settles this with that lane.
2. **One 0.6 format plan.** Spec 0028 (options) and FMT-2 (the split design file) both move the
   format to 0.6. The architecture check settles it (coordinator task T-256081). Spec 0030's layer
   reference belongs in the same plan.
3. **Checks against structures**, later: bridge clearance against the vessel's air draught, and
   existing control structures as given weirs.
4. **Parked:** undo and redo, moving a weir by hand, dragging the line by hand.
5. **Official sea charts (ENC)** are later, and only if a licence allows (spec 0025).
6. **Items being built elsewhere, not specced here:** weir labels on by default, weirs missing on
   the map, a road layer, and 3D bridges.
