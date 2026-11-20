// The map layer ticks (MAP-1), no browser. Run: node --test e2e/layers.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { LAYERS_KEY, MAP_LAYERS, STARTS_OFF, STYLE_LAYERS, defaultTicks, loadLayerTicks, parseTicks, saveLayerTicks, serialiseTicks } from "../src/lib/mapLayers.ts";

describe("the map layer ticks", () => {
  it("lists the nine layers, each with the map layers it shows or hides", () => {
    assert.deepEqual(MAP_LAYERS.map((l) => l.label), ["River polygon", "Centreline", "Bank lines", "Design line", "Constraint stretches", "Station marks", "Terrain shading", "Contours", "Imagery"]);
    for (const l of MAP_LAYERS) assert.ok(STYLE_LAYERS[l.id].length > 0, l.id);
  });
  it("are all on by default but the terrain shading and the contours, and for stored text that is not ours", () => {
    assert.deepEqual(STARTS_OFF, ["terrain-shading", "contours"]);
    for (const l of MAP_LAYERS) assert.equal(defaultTicks()[l.id], l.id !== "terrain-shading" && l.id !== "contours", l.id);
    for (const text of [null, "", "{", "{}", "7", '["no-such-layer"]']) assert.deepEqual(parseTicks(text), defaultTicks(), String(text));
  });
  it("round-trip through the stored text, which names only the layers turned off", () => {
    const ticks = { ...defaultTicks(), "river-polygon": false, imagery: false };
    assert.equal(serialiseTicks(ticks), '["river-polygon","imagery"]');
    assert.deepEqual(parseTicks(serialiseTicks(ticks)), ticks);
    assert.equal(serialiseTicks(defaultTicks()), null);
  });
  it("keep a layer that starts off by naming it when it is on, and read the older stored text as before", () => {
    const shaded = { ...defaultTicks(), "terrain-shading": true };
    assert.equal(serialiseTicks(shaded), '["terrain-shading"]');
    assert.deepEqual(parseTicks('["terrain-shading"]'), shaded);
    // Text stored before the terrain shading was a layer: the same layers are off, and the shading too.
    assert.deepEqual(parseTicks('["imagery"]'), { ...defaultTicks(), imagery: false });
    const both = { ...defaultTicks(), imagery: false, "terrain-shading": true };
    assert.deepEqual(parseTicks(serialiseTicks(both)), both);
    // The contours (MAP-3) start off too, and text stored before them reads as before.
    const lines = { ...defaultTicks(), contours: true };
    assert.equal(serialiseTicks(lines), '["contours"]');
    assert.deepEqual(parseTicks('["contours"]'), lines);
    assert.equal(parseTicks('["terrain-shading","imagery"]').contours, false);
  });
  it("are remembered, and a blocked store gives all on without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveLayerTicks({ ...defaultTicks(), centreline: false });
    assert.equal(store.get(LAYERS_KEY), '["centreline"]');
    assert.equal(loadLayerTicks().centreline, false);
    saveLayerTicks(defaultTicks());
    assert.equal(store.size, 0, "all on leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveLayerTicks({ ...defaultTicks(), centreline: false }));
    assert.deepEqual(loadLayerTicks(), defaultTicks());
    delete globalThis.localStorage;
  });
});
