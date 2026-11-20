// The 3D view's settings and tile addresses (MAP-2), no browser. Run: node --test e2e/terrain3d.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_EXAGGERATION, EXAGGERATIONS, TERRAIN_KEY, TERRAIN_PROTOCOL, defaultPrefs, loadTerrainPrefs, parsePrefs, parseTileUrl, saveTerrainPrefs, serialisePrefs, tileTemplate } from "../src/lib/terrain3d.ts";

describe("the 3D view settings", () => {
  it("start flat, at twice the height, and offer 1, 2, 5 and 10 times", () => {
    assert.deepEqual(defaultPrefs(), { on: false, exaggeration: 2 });
    assert.equal(DEFAULT_EXAGGERATION, 2);
    assert.deepEqual([...EXAGGERATIONS], [1, 2, 5, 10]);
  });
  it("read stored text that is not ours as the default", () => {
    for (const text of [null, "", "{", "[]", "7", '"on"', '{"on":"yes","exaggeration":3}', '{"exaggeration":"5"}']) assert.deepEqual(parsePrefs(text), defaultPrefs(), String(text));
  });
  it("round-trip through the stored text, which is empty for the default", () => {
    assert.equal(serialisePrefs(defaultPrefs()), null);
    for (const prefs of [{ on: true, exaggeration: 2 }, { on: false, exaggeration: 10 }, { on: true, exaggeration: 1 }]) assert.deepEqual(parsePrefs(serialisePrefs(prefs)), prefs);
    assert.equal(serialisePrefs({ on: true, exaggeration: 5 }), '{"on":true,"exaggeration":5}');
  });
  it("are remembered, and a blocked store gives the default without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveTerrainPrefs({ on: true, exaggeration: 5 });
    assert.equal(store.get(TERRAIN_KEY), '{"on":true,"exaggeration":5}');
    assert.deepEqual(loadTerrainPrefs(), { on: true, exaggeration: 5 });
    saveTerrainPrefs(defaultPrefs());
    assert.equal(store.size, 0, "the default leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveTerrainPrefs({ on: true, exaggeration: 5 }));
    assert.deepEqual(loadTerrainPrefs(), defaultPrefs());
    delete globalThis.localStorage;
  });
});

describe("the terrain tile address", () => {
  it("has a scheme of its own, so the map asks the page and never the network", () => {
    assert.equal(tileTemplate(3), "owd-dem://3/{z}/{x}/{y}");
    assert.ok(tileTemplate(1).startsWith(`${TERRAIN_PROTOCOL}://`));
    assert.doesNotMatch(tileTemplate(1), /^https?:/);
  });
  it("is read back to its DEM number and tile, and anything else is refused", () => {
    assert.deepEqual(parseTileUrl("owd-dem://3/14/8101/7958"), { id: 3, z: 14, x: 8101, y: 7958 });
    for (const url of ["https://example.org/14/1/2", "owd-dem://3/14/8101", "owd-dem://a/1/2/3", "owd-dem://3/14/8101/7958.png", "owd-dem://3/-1/0/0"]) assert.equal(parseTileUrl(url), null, url);
  });
});
