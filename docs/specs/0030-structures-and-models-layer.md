# Spec 0030: The structures and models layer, a published standard

Status: **agreed with the maintainer, 2026-11-27** (design round 4, question 1, his note: "a
general models/structures layer that we reference in the design file and have a published standard
— it could be for bridges, quays, existing control structures, navigation aids, transshipment
points"). Replaces the storage part of spec 0026 (an offshore structure). Engine, format, web app,
and a standard document.

## Problem

Structures the design must show, and later respect, come from many places: bridges over the river,
quays, existing weirs and other control structures, navigation aids, a transshipment point
offshore. Each has a place, a size and often a 3D model. There is no one way to give them to OWD,
and no way to share them between designs, projects or other tools (for example Open Vessel Concept
Design, which makes models).

## Agreed behaviour

### A layer file, referenced by the design

- The structures of a study live in **one layer file** in the project folder, beside the design
  (for example `structures/structures.geojson`), with their 3D models in `models/`.
- The design file **references** the layer file (its path and its hash), as it references the DEM.
  It does not copy the structures in. All options of a design (spec 0028) share it.
- A change to the layer file makes the design see "structures changed" when it opens.

### A published standard

The layer file follows a **published, versioned standard**: the *OWD Structures Layer standard*,
version 1.0, written in `docs/standards/structures-layer.md` with a JSON schema in `schema/`. Any
tool can write a file to it. Each structure has:

| Field | Meaning |
|---|---|
| `id`, `name` | Its key and its name |
| `kind` | One of: `bridge`, `quay`, `control_structure` (existing weir, lock, barrage, culvert), `navigation_aid`, `transshipment_point`, `other` |
| geometry | A point with a heading, or a line (a bridge axis, a quay face), in WGS84 or the project CRS |
| `model` | Optional: a glTF file (`.glb` or `.gltf`) in `models/`, with its insertion point, heading and scale |
| `dimensions` | Optional: length, width, height in metres; for a bridge, its span and its clearance above a stated level |
| `status` | `existing` or `proposed` |
| `source` | Where the data came from, and its date |
| `checked` | Always `false` in version 1.0: OWD shows the structure; it does not check it |

A field the standard does not know is kept and passed through, not dropped.

### In the app

- **Load** the layer in step 1, Project and data, like the buildings file; or **add** a structure on
  the map (pick the kind, click its point or draw its line, set its heading, choose its model). An
  added structure is written to the layer file.
- **Map**: a **Structures** tick in the Layers list, with a sub-tick per kind. 2D: footprint (from
  the model or the dimensions) and name. 3D: the model at true size, or a plain box from its
  dimensions when it has no model.
- **Results panel** (spec 0023): a Structures tab, one row per structure; click a row and the map
  goes there.
- Every row says "shown, not checked".

### Later, not in this spec

Checks against structures: bridge clearance against the vessel's air draught, existing control
structures as given weirs for the weirs and pools step (the "given structures" input of spec 0017,
stage contracts). The standard's fields are chosen so these can come without a new version.

## Acceptance checks

1. The standard document and its schema are in the repository; the schema validates the example
   layer file and refuses a structure with no `kind`.
2. Load an example layer holding one of each kind: each draws in 2D; each with a model draws it in
   3D at true size; each without draws a box.
3. The design file holds the layer's path and hash, not the structures.
4. Edit the layer file outside OWD and reopen: the app says the structures changed, and draws the new
   ones.
5. Add a transshipment point on the map with a glTF model: it is written to the layer file; the model
   is copied into `models/`.
6. Two options of one design both show the same structures.
7. An unknown field in the layer file survives a load and a save.
8. Every structure's row says "shown, not checked".

## Open question

How the 3D bridges now being built (the bridges lane) fit this standard: their data should be
written as `kind: bridge` structures. The coordinator settles it with that lane.
