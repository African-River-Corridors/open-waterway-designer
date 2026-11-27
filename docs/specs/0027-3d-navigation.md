# Spec 0027: Easier 3D navigation

Status: **agreed with the maintainer, 2026-11-27** (design round 1, note on question 12: "the 3D
navigation needs to improve"; round 2, question 12). Web app, 3D view (MAP-2).

## Problem

In the 3D view, turning and tilting with the mouse is awkward: the map turns about the middle of
the screen, not the place the engineer is looking at. It is easy to lose one's place, and there is
no way back but to zoom out by hand. There are no keyboard controls the engineer can rely on.

## Agreed behaviour

- **Orbit about the point clicked**: right-drag (or Ctrl+drag) turns and tilts about the ground
  point under the cursor when the drag starts. That point stays fixed on the screen.
- **Reset view**: a button that sets north up and no tilt, keeping the centre and zoom.
- **Fit the study**: a button that frames the whole study (the design line, or the river when there
  is no line), at the current tilt.
- **Keyboard**, when the map has focus:
  - arrows: move;
  - Shift + left/right: turn; Shift + up/down: tilt;
  - + and −: zoom in and out;
  - Home: fit the study; N: north up (reset view).
- A small "?" on the map shows these keys.

## Acceptance checks

1. In 3D, right-drag starting on a weir: the weir stays under the cursor while the view turns and
   tilts.
2. Reset view: bearing 0, pitch 0, same centre.
3. Fit the study: the whole design line is in view, at any tilt.
4. Each key in the list does what it says, and does nothing while typing in a field.
5. The "?" lists the keys.
