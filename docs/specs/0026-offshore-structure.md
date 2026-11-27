# Spec 0026: An offshore structure with its 3D model

Status: **agreed with the maintainer, 2026-11-27** (design round 1, note on question 12: "add a
transshipment point offshore and show the 3D model of it"; round 2, question 11). Web app. Storage
in the design file is open (see the end).

## Problem

A river corridor often ends at a transshipment point offshore, where river barges meet sea-going
vessels. The engineer wants to place it on the map with the river design and show it in 3D, so a
reader sees the whole corridor.

## Agreed behaviour

- A new kind of **structure**: a placed object with a **name**, a **kind** (first kind:
  transshipment point), a **point**, a **heading**, and a **3D model** from a glTF file (`.glb` or
  `.gltf`), for example one made in Open Vessel Concept Design.
- **Place it**: in step 1 (Project and data), "Add a structure": pick the kind, click its point on
  the map, drag to set its heading, choose its glTF file. The model file is copied into the project
  folder (`models/`).
- **2D**: drawn as its footprint (the model's outline seen from above, at true size and heading)
  with its name.
- **3D**: drawn as the model, at true size, on the water surface (or the ground on land).
- **Move it**: drag its point; rotate it by its heading handle. **Delete it** from its row.
- A **Structures** tick in the Layers list.
- It has no engineering checks yet: the app says "placed for display; not checked" in its row.

## Acceptance checks

1. Add a transshipment point offshore with a test glTF box of 100 × 30 × 20 m: its 2D footprint is
   100 × 30 m at the heading set.
2. In 3D the model shows at that size, on the sea surface.
3. Save, close, reopen the project: the structure, its heading and its model are back.
4. The model file is in the project folder's `models/`.
5. Drag and rotate: the 2D footprint and the 3D model follow.
6. A file that is not glTF is refused with the reason.

## Storage: settled by spec 0030

The structure is kept in the structures and models layer (spec 0030), a layer file the design
references, as `kind: transshipment_point` (design round 4, question 1). Build spec 0030 first; this
spec is then its first kind with a model.
