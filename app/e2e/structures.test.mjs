// The weirs, locks and pools on the map (MAP-4): labels, tile addresses and the 3D block heights, no browser.
// Run: node --test e2e/structures.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { PLACEHOLDER_NOTE, POOL_PROTOCOL, POOL_SURFACE_PROTOCOL, chamberText, extrusionFeatures, extrusionHeight, km, parsePoolUrl, partCentre, partFeatures, poolFeatures, poolLabel, poolSurfaceTemplate, poolTemplate, stopColour, waterReadout, weirLabel } from "../src/lib/structures.ts";

const square = (x, y, half) => [[x - half, y - half], [x + half, y - half], [x + half, y + half], [x - half, y + half], [x - half, y - half]];
const model = { note: PLACEHOLDER_NOTE, vessel_length_m: 85, vessel_beam_m: 9.5, chamber_length_m: 102, chamber_width_m: 11.5, wall_thickness_m: 3, wall_freeboard_m: 1.5, default_weir_length_m: 100, rules: "" };
const weir = { number: 7, chainage_m: 110_312, position: [-3, 5], bearing_deg: 90, upstream_level_m: 49, downstream_level_m: 42, lift_m: 7, upstream_pool: 8, downstream_pool: 7, weir_length_m: 240, clipped_to_river: true, parts: [{ kind: "weir", ring: square(-3, 5, 0.001), top_m: 49 }, { kind: "lock_chamber", ring: square(-3.01, 5, 0.001), top_m: 42 }] };
const pool = { number: 8, from_m: 110_312, to_m: 131_000, level_m: 49, line: [[-3, 5], [-2.9, 5.1]], mid: [-2.95, 5.05] };
const list = { model, pools: [pool], weirs: [weir] };

describe("the words of the weirs and pools", () => {
  it("label a weir with its number, its chainage, the pool level it holds and its lift", () => {
    assert.equal(weirLabel(weir), "Weir 7 · km 110.3 · pool 49.0 m · lift 7.0 m");
    assert.equal(km(999), "km 1.0");
  });
  it("label a pool with its number and level, and read out a click on the water", () => {
    assert.equal(poolLabel(pool), "Pool 8 · 49.0 m");
    assert.equal(waterReadout({ pool: 8, water_level_m: 49, ground_m: 44.5, depth_m: 4.5 }), "water level 49.0 m, ground 44.5 m, depth 4.5 m, pool 8");
  });
  it("say the chamber size used, or that no lock is drawn before a vessel is chosen", () => {
    assert.equal(chamberText(list), "102.0 m × 11.5 m");
    assert.match(chamberText({ ...list, model: { ...model, chamber_length_m: null, chamber_width_m: null } }), /no lock drawn/);
    assert.equal(PLACEHOLDER_NOTE, "placeholder model, not a design");
  });
});

describe("the shapes the map draws", () => {
  it("are one polygon for each block and one line for each pool, and none for a design with no pools", () => {
    const parts = partFeatures(list);
    assert.deepEqual(parts.map((f) => [f.geometry.type, f.properties.weir, f.properties.kind]), [["Polygon", 7, "weir"], ["Polygon", 7, "lock_chamber"]]);
    assert.deepEqual(poolFeatures(list).map((f) => [f.geometry.type, f.properties.pool, f.properties.odd]), [["LineString", 8, false]]);
    assert.deepEqual(partFeatures(null), []);
    assert.deepEqual(poolFeatures({ model, pools: [], weirs: [] }), []);
  });
  it("stand a block's top at its level: the map measures height from the drawn terrain at the block's middle", () => {
    // Ground at 40 m, drawn twice as tall (80 m): a top at 49 m is drawn at 98 m, so the block is 18 m tall.
    assert.equal(extrusionHeight(49, 80, 2), 18);
    assert.equal(extrusionHeight(49, 40, 1), 9);
    // A top under the terrain there (water standing over it) has no height.
    assert.equal(extrusionHeight(42, 98, 2), 0);
    const centre = partCentre(weir.parts[0]);
    assert.ok(Math.abs(centre[0] + 3) < 1e-9 && Math.abs(centre[1] - 5) < 1e-9, "the mean of the corners, the closing point left out");
    const asked = [];
    const blocks = extrusionFeatures(list, 2, (at) => (asked.push(at), 80));
    assert.deepEqual(blocks.map((f) => [f.properties.kind, f.properties.top_m, f.properties.height]), [["weir", 49, 18], ["lock_chamber", 42, 4]]);
    assert.equal(asked.length, 2);
  });
});

describe("the pool water tile address", () => {
  it("has schemes of its own, so the map asks the page and never the network", () => {
    assert.equal(poolTemplate(3), "owd-pool://3/{z}/{x}/{y}");
    assert.equal(poolSurfaceTemplate(3), "owd-pool-dem://3/{z}/{x}/{y}");
    assert.ok(poolTemplate(1).startsWith(`${POOL_PROTOCOL}://`) && poolSurfaceTemplate(1).startsWith(`${POOL_SURFACE_PROTOCOL}://`));
  });
  it("is read back to its flood mask and tile, and anything else is refused", () => {
    assert.deepEqual(parsePoolUrl("owd-pool://3/14/8101/7958"), { surface: false, id: 3, z: 14, x: 8101, y: 7958 });
    assert.deepEqual(parsePoolUrl("owd-pool-dem://4/9/1/2"), { surface: true, id: 4, z: 9, x: 1, y: 2 });
    for (const url of ["https://example.org/14/1/2", "owd-pool://3/14/8101", "owd-dem://3/1/2/3", "owd-pool://a/1/2/3", "owd-pool://3/14/8101/7958.png"]) assert.equal(parsePoolUrl(url), null, url);
  });
  it("gives a depth stop as a colour the legend can draw", () => {
    assert.equal(stopColour([8, 48, 107, 255]), "rgba(8, 48, 107, 1.00)");
  });
});
