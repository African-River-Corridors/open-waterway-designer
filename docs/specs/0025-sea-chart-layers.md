# Spec 0025: Sea chart layers

Status: **agreed with the maintainer, 2026-11-27** (design round 1, note on question 12: "add
layers for the sea charts in offshore areas"; round 2, question 10). Web app.

## Problem

A river study that ends at the coast needs the sea beyond the river mouth: water depth offshore,
and the buoys, lights and channels a vessel uses. The map has no layer for any of these.

## Agreed behaviour

Open data first. Two new ticks in the map's Layers list, off by default:

- **Sea marks**: the OpenSeaMap sea-mark tiles (buoys, beacons, lights, harbours) drawn over the
  imagery.
- **Sea-bed depth**: GEBCO bathymetry (the global sea-bed grid), as a shaded depth layer with depth
  contours, and the depth in metres under the cursor.

Each layer shows its attribution in the map's attribution line while on, as the licence asks.

Official electronic navigational charts (ENC) are **later**, and only where a licence allows their
use and their display in an open-source tool.

## Acceptance checks

1. Tick Sea marks on a coastal view: the sea-mark symbols draw; the attribution shows "©
   OpenSeaMap contributors". Untick: both go.
2. Tick Sea-bed depth: the depth shading and contours draw offshore; hovering shows a depth in
   metres; the GEBCO attribution shows.
3. Over land and rivers the depth layer draws nothing.
4. With the network off, each layer greys and says "not available offline"; the rest of the map
   works.
5. A licence note for both sources is in the repository's third-party notices before the layer
   ships (checked by the builder: OpenSeaMap's licence and GEBCO's terms of use).
