// The map layer ticks (MAP-1), no browser. Run: node --test e2e/layers.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { LAYERS_KEY, MAP_LAYERS, STYLE_LAYERS, allOn, loadLayerTicks, parseTicks, saveLayerTicks, serialiseTicks } from "../src/lib/mapLayers.ts";

describe("the map layer ticks", () => {
  it("lists the seven layers, each with the map layers it shows or hides", () => {
    assert.deepEqual(MAP_LAYERS.map((l) => l.label), ["River polygon", "Centreline", "Bank lines", "Design line", "Constraint stretches", "Station marks", "Imagery"]);
    for (const l of MAP_LAYERS) assert.ok(STYLE_LAYERS[l.id].length > 0, l.id);
  });
  it("are all on by default, and for stored text that is not ours", () => {
    assert.ok(Object.values(allOn()).every(Boolean));
    for (const text of [null, "", "{", "{}", "7", '["no-such-layer"]']) assert.deepEqual(parseTicks(text), allOn(), String(text));
  });
  it("round-trip through the stored text, which names only the layers turned off", () => {
    const ticks = { ...allOn(), "river-polygon": false, imagery: false };
    assert.equal(serialiseTicks(ticks), '["river-polygon","imagery"]');
    assert.deepEqual(parseTicks(serialiseTicks(ticks)), ticks);
    assert.equal(serialiseTicks(allOn()), null);
  });
  it("are remembered, and a blocked store gives all on without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveLayerTicks({ ...allOn(), centreline: false });
    assert.equal(store.get(LAYERS_KEY), '["centreline"]');
    assert.equal(loadLayerTicks().centreline, false);
    saveLayerTicks(allOn());
    assert.equal(store.size, 0, "all on leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveLayerTicks({ ...allOn(), centreline: false }));
    assert.deepEqual(loadLayerTicks(), allOn());
    delete globalThis.localStorage;
  });
});
