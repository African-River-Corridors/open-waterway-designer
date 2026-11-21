// The channel and cut on the map (MAP-5), no browser: the terrain state, the tile addresses, the read-out,
// and the cross-section's cut face. Run: node --test e2e/channel.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { CUT_PROTOCOL, STATE_KEY, TERRAIN_STATES, chainageLabel, classLabel, cutTemplate, loadTerrainState, otherState, parseCutUrl, parseState, readout, saveTerrainState, serialiseState } from "../src/lib/channel.ts";
import { parseTileUrl, stateTileTemplate, tileTemplate } from "../src/lib/terrain3d.ts";
import { contourTemplate, parseContourUrl } from "../src/lib/contours.ts";
import { MAP_LAYERS, STARTS_OFF, STYLE_LAYERS, defaultTicks, parseTicks, serialiseTicks } from "../src/lib/mapLayers.ts";
import { faceView } from "../src/lib/crossSectionView.ts";

describe("the ground the map shows", () => {
  it("is Before or After, starts Before, and stored text that is not ours reads as Before", () => {
    assert.deepEqual(TERRAIN_STATES.map((s) => [s.id, s.label]), [["before", "Before"], ["after", "After"]]);
    for (const text of [null, "", "{", "pool", '"after"', "AFTER"]) assert.equal(parseState(text), "before", String(text));
    assert.equal(parseState("after"), "after");
    assert.equal(serialiseState("before"), null);
    assert.equal(serialiseState("after"), "after");
    assert.equal(otherState("before"), "after");
    assert.equal(otherState("after"), "before");
  });
  it("is remembered, and a blocked store gives Before without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveTerrainState("after");
    assert.equal(store.get(STATE_KEY), "after");
    assert.equal(loadTerrainState(), "after");
    saveTerrainState("before");
    assert.equal(store.size, 0, "Before leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveTerrainState("after"));
    assert.equal(loadTerrainState(), "before");
    delete globalThis.localStorage;
  });
});

describe("the tile addresses with a terrain state", () => {
  it("keep the DEM's own address for Before and name the state and the cut for After", () => {
    assert.equal(stateTileTemplate(3, "before", 7), tileTemplate(3));
    assert.equal(stateTileTemplate(3, "after", 7), "owd-dem://3/after/7/{z}/{x}/{y}");
    assert.deepEqual(parseTileUrl("owd-dem://3/after/7/14/8101/7958"), { id: 3, state: "after", z: 14, x: 8101, y: 7958 });
    assert.deepEqual(parseTileUrl("owd-dem://3/14/8101/7958"), { id: 3, state: "before", z: 14, x: 8101, y: 7958 });
    // A third state needs no change to the address: one more name.
    assert.equal(parseTileUrl("owd-dem://3/water/2/14/8101/7958").state, "water");
    for (const url of ["owd-dem://3/after/14/8101/7958/1/2", "owd-dem://3/After/7/14/1/2", "owd-dem://3/after/x/14/1/2"]) assert.equal(parseTileUrl(url), null, url);
  });
  it("do the same for the contours", () => {
    assert.equal(contourTemplate(3, "auto", "before", 7), "owd-contour://3/auto/{z}/{x}/{y}");
    assert.equal(contourTemplate(3, 5, "after", 7), "owd-contour://3/after/7/5/{z}/{x}/{y}");
    assert.deepEqual(parseContourUrl("owd-contour://3/after/7/5/14/8101/7958"), { id: 3, choice: 5, z: 14, x: 8101, y: 7958, state: "after" });
    assert.deepEqual(parseContourUrl("owd-contour://3/after/7/auto/14/8101/7958"), { id: 3, choice: "auto", z: 14, x: 8101, y: 7958, state: "after" });
    assert.equal(parseContourUrl("owd-contour://3/after/7/3/14/8101/7958"), null, "3 m is not an interval");
  });
  it("give the cut-depth picture a scheme of its own, so the map asks the page and never the network", () => {
    assert.equal(cutTemplate(3, 7), "owd-cut://3/7/{z}/{x}/{y}");
    assert.ok(cutTemplate(1, 1).startsWith(`${CUT_PROTOCOL}://`));
    assert.doesNotMatch(cutTemplate(1, 1), /^https?:/);
    assert.deepEqual(parseCutUrl("owd-cut://3/7/14/8101/7958"), { id: 3, cut: 7, z: 14, x: 8101, y: 7958 });
    for (const url of ["https://example.org/14/1/2", "owd-dem://3/14/8101/7958", "owd-cut://3/14/8101/7958", "owd-cut://3/7/14/8101/7958.png"]) assert.equal(parseCutUrl(url), null, url);
  });
});

describe("the Channel and Cut extent layers", () => {
  it("are in the Layers list after the design line: the channel on, the cut extent off", () => {
    const ids = MAP_LAYERS.map((l) => l.id);
    assert.deepEqual(ids.slice(ids.indexOf("design-line"), ids.indexOf("design-line") + 3), ["design-line", "channel", "cut"]);
    assert.deepEqual(MAP_LAYERS.filter((l) => l.id === "channel" || l.id === "cut").map((l) => l.label), ["Channel", "Cut extent"]);
    assert.equal(defaultTicks().channel, true);
    assert.equal(defaultTicks().cut, false);
    assert.ok(STARTS_OFF.includes("cut") && !STARTS_OFF.includes("channel"));
    assert.ok(STYLE_LAYERS.channel.includes("channel-bed") && STYLE_LAYERS.cut.includes("cut-depth"));
  });
  it("are remembered by naming the tick that is not the default, and older stored text reads as before", () => {
    const ticks = { ...defaultTicks(), channel: false, cut: true };
    assert.equal(serialiseTicks(ticks), '["channel","cut"]');
    assert.deepEqual(parseTicks('["channel","cut"]'), ticks);
    assert.deepEqual(parseTicks('["imagery"]'), { ...defaultTicks(), imagery: false });
  });
});

describe("the read-out of a point of the cut", () => {
  it("says the chainage, the levels before and after, the depth, the kind and the station's areas", () => {
    const rows = readout({ chainage_m: 12345.6, offset_m: -18.24, ground_m: 31.257, after_m: 12, depth_m: 19.257, kind: "dry", station_chainage_m: 12350, dredge_area_m2: 41.4, excav_area_m2: 1820.6, max_face_h_m: 17.25 });
    assert.deepEqual(rows, [
      ["Chainage", "12+345 (18.2 m left of the line)"],
      ["Ground before", "31.26 m"],
      ["Level after", "12.00 m"],
      ["Depth of cut", "19.26 m"],
      ["Kind", "dry cut"],
      ["Station 12+350", "dredge 41 m², dry cut 1821 m², face 17.3 m"],
    ]);
    assert.equal(readout({ ...{ chainage_m: 0, offset_m: 3, ground_m: 1, after_m: 0, depth_m: 1, station_chainage_m: 0, dredge_area_m2: 0, excav_area_m2: 0, max_face_h_m: 0 }, kind: "dredge" })[4][1], "dredge (in the river)");
    assert.equal(chainageLabel(999.9), "0+999");
    assert.equal(classLabel({ from_m: 2, to_m: 5, dredge: "#000000", dry: "#000000" }), "2 to 5 m");
    assert.equal(classLabel({ from_m: 20, to_m: null, dredge: "#000000", dry: "#000000" }), "20 m and more");
  });
});

/** A section as the engine stores it: flat ground at `ground`, the cut template out to +/-200 m. Bed 20 m
 * each side at level 0, wet slope 1 in 3 up to the water at 3 m, a 2 m berm, then 1 in 1.5 for ever. */
function section(ground) {
  const offsets_m = Array.from({ length: 401 }, (_, i) => i - 200);
  const face = (o) => {
    const a = Math.abs(o);
    if (a <= 20) return 0;
    if (a <= 29) return (a - 20) / 3;
    if (a <= 31) return 3;
    return 3 + (a - 31) / 1.5;
  };
  return { offsets_m, terrain_m: offsets_m.map((o) => (typeof ground === "function" ? ground(o) : ground)), cut_face_m: offsets_m.map(face), water_level_m: 3, design_invert_m: 0 };
}

describe("the cross-section's cut face", () => {
  it("is drawn only up to the daylight points, and the plot is scaled to the ground and the channel", () => {
    const s = section(9);
    // The stored template climbs to more than 110 m at the ends of the section: none of it is drawn.
    assert.ok(Math.max(...s.cut_face_m) > 110);
    const v = faceView(s);
    assert.equal(v.lo, 0);
    assert.equal(v.hi, 9, "scaled to the ground, not to the template");
    assert.ok(v.points.every((p) => p.level_m <= 9 + 1e-9));
    // The face meets ground at 9 m where 3 + (o - 31) / 1.5 = 9: 40 m each side.
    assert.deepEqual(v.daylight, { left: { offset_m: -40, level_m: 9 }, right: { offset_m: 40, level_m: 9 } });
    assert.deepEqual([v.points[0], v.points.at(-1)], [{ offset_m: -40, level_m: 9 }, { offset_m: 40, level_m: 9 }]);
    assert.equal(v.points.length, 81);
  });
  it("finds a daylight point between two samples, and further out on the high side of a hillside", () => {
    const v = faceView(section(9.5));
    assert.ok(Math.abs(v.daylight.right.offset_m - 40.75) < 1e-9 && Math.abs(v.daylight.right.level_m - 9.5) < 1e-9);
    assert.equal(v.points.at(-1).offset_m, 40.75);
    const hill = faceView(section((o) => 9 + 0.1 * o));
    assert.ok(hill.daylight.right.offset_m > -hill.daylight.left.offset_m + 5);
    for (const d of [hill.daylight.left, hill.daylight.right]) assert.ok(Math.abs(d.level_m - (9 + 0.1 * d.offset_m)) < 1e-9, "on the ground");
    assert.ok(hill.hi < 30, "the hill's own top, not the template's");
  });
  it("draws the channel under water where the ground is below the bed, with no daylight point", () => {
    const v = faceView(section(-4));
    assert.deepEqual(v.daylight, { left: null, right: null });
    assert.deepEqual([v.points[0].offset_m, v.points.at(-1).offset_m], [-31, 31], "out to the water's edge and the berm");
    assert.equal(v.lo, -4);
    assert.equal(v.hi, 3, "the water level is the highest thing drawn");
  });
  it("draws nothing of a section with no face, and is not thrown by missing ground", () => {
    const none = { offsets_m: [-1, 0, 1], terrain_m: [5, null, 5], cut_face_m: [null, null, null], water_level_m: 3, design_invert_m: 0 };
    assert.deepEqual(faceView(none).points, []);
    const gap = section(9);
    gap.terrain_m[240] = null;
    assert.equal(faceView(gap).daylight.right.offset_m, 39, "the sample itself when the next one cannot say");
  });
});
