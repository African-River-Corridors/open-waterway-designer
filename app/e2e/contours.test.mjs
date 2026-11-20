// The contour interval and the contour tile addresses (MAP-3), no browser. Run: node --test e2e/contours.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { CONTOUR_DEEP_ZOOM, CONTOUR_KEY, CONTOUR_MIN_ZOOM, CONTOUR_PROTOCOL, DEFAULT_CHOICE, INTERVALS, LABEL_PREFIX, autoInterval, contourTemplate, intervalAt, loadContourChoice, parseChoice, parseContourUrl, saveContourChoice, serialiseChoice } from "../src/lib/contours.ts";

describe("the contour interval", () => {
  it("offers Auto, 1, 2, 5, 10 and 20 m, and starts on Auto", () => {
    assert.deepEqual([...INTERVALS], [1, 2, 5, 10, 20]);
    assert.equal(DEFAULT_CHOICE, "auto");
  });
  it("Auto picks the interval from the zoom level: 1 m from zoom 15 in, and coarser at each level out", () => {
    assert.deepEqual([9, 10, 11, 12, 13, 14, 15, 16, 18].map(autoInterval), [20, 20, 20, 10, 5, 2, 1, 1, 1]);
    // Zoomed out, Auto never draws more lines: the interval never falls as the zoom level falls.
    for (let z = 0; z < 22; z += 1) assert.ok(autoInterval(z) >= autoInterval(z + 1), `zoom ${z}`);
    for (let z = 0; z < 22; z += 1) assert.equal(intervalAt("auto", z), autoInterval(z));
  });
  it("a chosen interval is drawn from one zoom level further out than Auto picks it, and coarser beyond", () => {
    // The first zoom level each interval is drawn at as chosen.
    const first = (i) => [...Array(22).keys()].find((z) => intervalAt(i, z) === i);
    assert.deepEqual(INTERVALS.map(first), [14, 13, 12, 11, 0]);
    for (const i of INTERVALS) {
      for (let z = 0; z < 22; z += 1) {
        const drawn = intervalAt(i, z);
        assert.ok(drawn >= i, `${i} m at zoom ${z} is never finer than chosen`);
        assert.ok(drawn === i || drawn === autoInterval(z + 1), `${i} m at zoom ${z}: ${drawn}`);
        // Never more than one step finer than Auto: the map does not flood.
        assert.ok(drawn >= autoInterval(z + 1), `${i} m at zoom ${z}`);
      }
      assert.equal(intervalAt(i, CONTOUR_DEEP_ZOOM), i, `${i} m is drawn as chosen at the deepest tiles`);
    }
    assert.equal(intervalAt(1, 13), 2);
    assert.equal(intervalAt(1, 12), 5);
    assert.equal(intervalAt(5, 11), 10);
    assert.ok(CONTOUR_MIN_ZOOM < 11 && CONTOUR_DEEP_ZOOM === 15);
  });
  it("reads stored text that is not ours as Auto", () => {
    for (const text of [null, "", "{", "[]", "3", '"5"', '"auto"', "0", '{"interval":5}']) assert.equal(parseChoice(text), "auto", String(text));
  });
  it("round-trips through the stored text, which is empty for Auto", () => {
    assert.equal(serialiseChoice("auto"), null);
    for (const i of INTERVALS) assert.equal(parseChoice(serialiseChoice(i)), i);
    assert.equal(serialiseChoice(5), "5");
  });
  it("is remembered, and a blocked store gives Auto without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveContourChoice(10);
    assert.equal(store.get(CONTOUR_KEY), "10");
    assert.equal(loadContourChoice(), 10);
    saveContourChoice("auto");
    assert.equal(store.size, 0, "Auto leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveContourChoice(2));
    assert.equal(loadContourChoice(), "auto");
    delete globalThis.localStorage;
  });
});

describe("the contour tile address", () => {
  it("has a scheme of its own, so the map asks the page and never the network", () => {
    assert.equal(contourTemplate(3, "auto"), "owd-contour://3/auto/{z}/{x}/{y}");
    assert.equal(contourTemplate(4, 5), "owd-contour://4/5/{z}/{x}/{y}");
    assert.ok(contourTemplate(1, 1).startsWith(`${CONTOUR_PROTOCOL}://`));
    assert.doesNotMatch(contourTemplate(1, 1), /^https?:/);
    assert.doesNotMatch(LABEL_PREFIX, /^https?:/);
  });
  it("is read back to its DEM number, interval and tile, and anything else is refused", () => {
    assert.deepEqual(parseContourUrl("owd-contour://3/auto/14/8101/7958"), { id: 3, choice: "auto", z: 14, x: 8101, y: 7958 });
    assert.deepEqual(parseContourUrl("owd-contour://3/20/9/252/248"), { id: 3, choice: 20, z: 9, x: 252, y: 248 });
    for (const url of ["https://example.org/14/1/2", "owd-dem://3/14/8101/7958", "owd-contour://3/14/8101/7958", "owd-contour://3/3/14/8101/7958", "owd-contour://3/auto/14/8101", "owd-contour://a/auto/1/2/3", "owd-contour://3/auto/14/8101/7958.pbf"]) assert.equal(parseContourUrl(url), null, url);
  });
});
