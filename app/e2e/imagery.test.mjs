// The imagery setting's rules (no browser). Run: node --test e2e/imagery.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { ATTRIBUTION, BASEMAP_TILES } from "../src/lib/basemap.ts";
import {
  DEFAULT_IMAGERY, ESRI_FAILED, ESRI_MIN_ZOOM, ESRI_POWERED, ESRI_TILES, esriAttribution, layerFor, loadImagery, maskKey, parse, saveImagery, serialise, usable, validTileUrl,
} from "../src/lib/imagery.ts";

const FAKE = "FAKE-KEY-not-real-abcd1234";

describe("the default", () => {
  it("is exactly today's open imagery and attribution", () => {
    const l = layerFor(DEFAULT_IMAGERY);
    assert.equal(l.source, "open");
    assert.equal(l.tiles, BASEMAP_TILES);
    assert.equal(l.attribution, ATTRIBUTION);
    assert.equal(l.tileSize, 256);
    assert.equal(l.maxzoom, 14);
  });
  it("is what bad or missing stored text gives", () => {
    assert.deepEqual(parse(null), DEFAULT_IMAGERY);
    assert.deepEqual(parse("not json"), DEFAULT_IMAGERY);
    assert.deepEqual(parse('{"source":"nonsense","esriKey":5}'), DEFAULT_IMAGERY);
  });
  it("stays open when Esri is chosen without a key, or custom without an attribution", () => {
    assert.equal(layerFor({ ...DEFAULT_IMAGERY, source: "esri" }).source, "open");
    assert.equal(layerFor({ ...DEFAULT_IMAGERY, source: "custom", customUrl: "https://t.example/{z}/{x}/{y}.png" }).source, "open");
  });
});

describe("the key", () => {
  it("is masked to its last four characters", () => {
    const m = maskKey(FAKE);
    assert.ok(m.endsWith("1234"));
    assert.ok(!m.includes("FAKE") && !m.includes("abcd"));
    assert.equal(maskKey(""), "");
    assert.ok(!maskKey("abcdef").includes("abc"), "a short key is hidden whole");
  });
  it("goes into the Esri tile address, and the Esri attribution names Esri", () => {
    const l = layerFor({ ...DEFAULT_IMAGERY, source: "esri", esriKey: FAKE }, BASEMAP_TILES, "Sources: Example data");
    assert.ok(l.tiles.includes(FAKE));
    assert.ok(l.tiles.includes("{z}/{y}/{x}"), "Esri orders the path level, row, column");
    assert.equal(l.tileSize, 256);
    assert.equal(l.minzoom, ESRI_MIN_ZOOM, "Esri tiles are asked for only when zoomed in");
    assert.equal(l.under, BASEMAP_TILES, "the open imagery shows under it, further out");
    assert.ok(l.attribution.startsWith(`${ESRI_POWERED}. Sources: Example data`));
    assert.ok(l.attribution.includes(ATTRIBUTION), "both pictures are credited");
    assert.ok(!ESRI_TILES.includes("/labels/"), "the labels style has no picture");
    assert.equal(esriAttribution(null), ESRI_POWERED);
    assert.ok(ESRI_TILES.startsWith("https://ibasemaps-api.arcgis.com/"));
  });
  it("is remembered, and a blocked store gives the default without an error", () => {
    const store = new Map();
    globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) };
    saveImagery({ ...DEFAULT_IMAGERY, source: "esri", esriKey: FAKE });
    assert.equal(loadImagery().esriKey, FAKE);
    saveImagery(DEFAULT_IMAGERY);
    assert.equal(store.size, 0, "the default leaves nothing behind");
    globalThis.localStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    assert.doesNotThrow(() => saveImagery({ ...DEFAULT_IMAGERY, source: "esri", esriKey: FAKE }));
    assert.deepEqual(loadImagery(), DEFAULT_IMAGERY);
    delete globalThis.localStorage;
  });
  it("round-trips through the stored text", () => {
    const s = { source: "custom", esriKey: "", customUrl: "https://t.example/{z}/{x}/{y}.png", customAttribution: "Example" };
    assert.deepEqual(parse(serialise(s)), s);
  });
});

describe("custom tiles", () => {
  it("need a web address with {z}, {x} and {y}, and an attribution", () => {
    assert.ok(validTileUrl("https://t.example/{z}/{x}/{y}.png"));
    assert.ok(!validTileUrl("https://t.example/{z}/{x}.png"));
    assert.ok(!validTileUrl("file:///t/{z}/{x}/{y}.png"));
    assert.ok(!usable({ ...DEFAULT_IMAGERY, source: "custom", customUrl: "https://t.example/{z}/{x}/{y}.png", customAttribution: "  " }));
    const l = layerFor({ source: "custom", esriKey: "", customUrl: "https://t.example/{z}/{x}/{y}.png", customAttribution: " Example " });
    assert.equal(l.source, "custom");
    assert.equal(l.attribution, "Example");
  });
});

describe("messages", () => {
  it("say what to do and never hold a key", () => {
    assert.equal(ESRI_FAILED, "Esri imagery could not load: check your key (it needs the basemap privilege) or choose another imagery source");
  });
});
