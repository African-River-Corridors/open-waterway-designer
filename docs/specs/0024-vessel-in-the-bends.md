# Spec 0024: The vessel in the bends

Status: **agreed with the maintainer, 2026-11-27** (design round 1, note on question 12: "show
barges on the river navigating meanders"; round 2, question 9). Web app, 2D and 3D map.

## Problem

The engineer and their reader cannot see how the design vessel moves through the bends. R_min and
the bend widening are numbers; a reviewer wants to see the vessel turn and see where its swept path
leaves the channel.

## Agreed behaviour

- A **Vessel** control on the map (shown when the design has a vessel and a design line): Play,
  Pause, a slider along the line's chainage, and a speed choice (×1, ×10, ×50 of a stated vessel
  speed, default 10 km/h).
- The **design vessel** (length and beam from step 2, Vessel and channel) is drawn to scale as a
  hull outline moving along the design line, bow and stern on the line, heading along the line.
- Its **swept path** is drawn as a band: the area its hull outline covers as it moves. The band is
  drawn where the vessel has been and stays on after it passes.
- Where the band leaves the **channel bottom** (the design channel's bottom edge, as the Channel
  layer draws it), that part of the band is **red**. Elsewhere it is a light tint.
- In 3D the hull is a simple box of the vessel's length, beam and draught on the water surface (a
  glTF model can replace it later, as spec 0026 does for structures).
- Two map layer ticks: **Vessel** and **Swept path**, on when the control is used.
- The slider and the profile and cross-section panels share the station: moving one moves the others.

## What the screen shows

A small control bar at the bottom of the map: ▶/⏸, the slider, the chainage, the speed. The
results panel's Summary (spec 0023) gains one line under Line: "Swept path outside the channel:
N places, longest M m".

## Acceptance checks

1. Open the example, run steps 2 and 3, press Play: the hull moves from the start to the end of
   the line; Pause stops it; the slider moves it.
2. The hull's length and beam on the map equal the vessel's, to 0.5 m.
3. On a circular bend of radius R, the band's width equals the hull's swept width (the envelope of
   the outline: about B + L²/(8R) for a vessel of length L and beam B with bow and stern on the
   line) to within 5 %.
4. A synthetic bend tighter than R_min: red band shows there; a straight: no red.
5. In 3D the box rides on the water surface.
6. Moving the slider moves the profile's station, and the reverse.
