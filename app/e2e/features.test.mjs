// Buildings, places and settlements on the map (BLD-1): the pure helpers, no browser.
// Run: node --test e2e/features.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { NAME_MAX, PLACE_BLOCK_NOTE, PLACE_KINDS, cardRows, kindsPresent, parsePlaceImageId, placeBlock, placeImageId, placeKind, placeSortKey, ringCentre, shortName } from "../src/lib/features.ts";

describe("places, buildings and settlements on the map", () => {
  it("know eight kinds of place, each with a mark; an unknown kind is other, and the legend lists the kinds present in order", () => {
    assert.deepEqual(PLACE_KINDS.map((k) => k.kind), ["community", "bridge", "quay", "loading_site", "ferry", "intake", "structure", "other"]);
    for (const k of PLACE_KINDS) assert.ok(k.shape && k.label, k.kind);
    assert.equal(placeKind("Loading site"), "loading_site");
    assert.equal(placeKind("loading-site"), "loading_site");
    assert.equal(placeKind("water tower"), "other");
    assert.equal(placeKind(undefined), "community");
    assert.deepEqual(kindsPresent(["quay", "community", "water tower", "community"]), ["community", "quay", "other"]);
  });
  it("name one picture per distinct kind and name, read it back, and cut a long name", () => {
    const id = placeImageId("community", "Riverbend: north");
    assert.deepEqual(parsePlaceImageId(id), { kind: "community", name: "Riverbend: north" });
    assert.equal(parsePlaceImageId("owd-contour-level:10"), null);
    const long = "A".repeat(NAME_MAX + 10);
    assert.equal(shortName(long).length, NAME_MAX);
    assert.ok(shortName(long).endsWith("…"));
    assert.equal(shortName("Riverbend"), "Riverbend");
  });
  it("place the larger place first: the sort key falls as the area grows; no area is placed last", () => {
    assert.ok(placeSortKey(45.5) < placeSortKey(2.1));
    assert.ok(placeSortKey(2.1) < placeSortKey(undefined));
    assert.equal(placeSortKey("not a number"), 0);
  });
  it("draw a building zoomed out at the mean of its corners", () => {
    assert.deepEqual(ringCentre({ type: "Polygon", coordinates: [[[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]]] }), [1, 1]);
    assert.deepEqual(ringCentre({ type: "Point", coordinates: [3, 4] }), [3, 4]);
  });
  it("give a bridge a deck block along its bearing at its deck level, a quay a slab, and other kinds none", () => {
    const at = [-3, 5.43];
    const b = placeBlock({ kind: "bridge", bearing_deg: 0, length_m: 120, width_m: 8, deck_level_m: 36 }, at);
    assert.equal(b.kind, "bridge");
    assert.equal(b.top_m, 36);
    assert.equal(b.ring.length, 5);
    assert.deepEqual(b.ring[0], b.ring[4]);
    // Bearing 0: the long side runs north; 120 m of latitude and 8 m of longitude.
    const lat = (b.ring[1][1] - b.ring[0][1]) * 111_320;
    const lon = (b.ring[2][0] - b.ring[1][0]) * 111_320 * Math.cos((5.43 * Math.PI) / 180);
    assert.ok(Math.abs(lat - 120) < 1e-6 && Math.abs(lon - 8) < 1e-6, `${lat} ${lon}`);
    // Bearing 90: along the east.
    const q = placeBlock({ kind: "quay", bearing_deg: 90, length_m: 60, width_m: 15, height_m: 2 }, at);
    assert.ok(Math.abs((q.ring[1][0] - q.ring[0][0]) * 111_320 * Math.cos((5.43 * Math.PI) / 180) - 60) < 1e-6);
    assert.deepEqual([q.top_m, q.height_m], [null, 2]);
    assert.equal(placeBlock({ kind: "community", length_m: 10, width_m: 10 }, at), null);
    assert.equal(placeBlock({ kind: "bridge" }, at), null, "a bridge with no size has no block");
  });
  it("read a card in plain rows: a building's area, confidence, ground, water, line and cut; a place's name, kind, properties and model", () => {
    const rows = cardRows({ input: "buildings", fid: 3, id: "4", name: "", kind: "", properties: {}, lon_lat: [-3, 5.4], area_m2: 81.2, confidence: 0.65, below_confidence: true, ground_m: 3.456, line_distance_m: 212.4, line_chainage_m: 1234, cut: { chainage_m: 1225, offset_m: 210, outside_m: 180, daylight: true, from_volumes: true, relation: "clear" }, water: { pool: 2, water_level_m: 4, ground_m: 3.456, depth_m: 0.544 } });
    assert.deepEqual(rows, [
      ["Area", "81 m²"],
      ["Confidence", "0.65 (below the threshold: not costed)"],
      ["Ground level", "3.46 m"],
      ["Pool water level", "4.00 m (pool 2)"],
      ["Depth of flooding", "0.54 m"],
      ["To the design line", "212 m"],
      ["Chainage of the nearest point", "1.234 km"],
      ["To the daylight line", "180 m outside it"],
    ]);
    const place = cardRows({ input: "points_of_interest", fid: 0, id: "1", name: "Old Crossing", kind: "bridge", properties: { name: "Old Crossing", kind: "bridge", length_m: 120, width_m: 8, deck_level_m: 36 }, lon_lat: [-3, 5.4], area_m2: null, confidence: null, ground_m: null, water: null });
    assert.deepEqual(place.slice(0, 2), [["Name", "Old Crossing"], ["Kind", "Bridge"]]);
    assert.ok(place.some(([k, v]) => k === "Flooding" && v === "dry"));
    assert.ok(place.some(([k, v]) => k === "Model" && v === PLACE_BLOCK_NOTE));
    const other = cardRows({ input: "points_of_interest", fid: 1, id: "2", name: "Pump House", kind: "water tower", properties: { name: "Pump House", kind: "water tower" }, lon_lat: [-3, 5.4], area_m2: null, confidence: null, ground_m: 4 });
    assert.deepEqual(other[1], ["Kind", "Other (water tower)"]);
  });
});
