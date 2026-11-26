// Buildings, places and settlements in the web app (BLD-1): the built web app in headless Chrome. Run by
// e2e/run-web.sh. The data is synthetic (e2e/synthetic-places.mjs) beside the synthetic web example.
import { after, before, describe, it } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { serve } from "./web-server.mjs";
import { buildings, places, settlements } from "./synthetic-places.mjs";

const here = import.meta.dirname;
const FIXTURES = path.resolve(here, "../../examples/web");
const DIST = path.resolve(here, "../dist-web-test");
const CLI = process.env.OWD_CLI;
assert.ok(CLI, "OWD_CLI must be set (run e2e/run-web.sh)");
const SHOTS = process.env.OWD_WEB_SHOTS;

let origin, server;
const sessions = [];
before(async () => {
  ({ server, origin } = await serve(DIST));
});
after(async () => {
  for (const b of sessions) await b.deleteSession().catch(() => {});
  server?.close();
});

const chromeBinary = process.env.CHROME_BINARY ?? (fs.existsSync("/Applications/Google Chrome.app") ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : undefined);
async function start() {
  const b = await remote({
    logLevel: "warn",
    strictSelectors: false,
    capabilities: {
      browserName: "chrome",
      "wdio:enforceWebDriverClassic": true,
      "goog:chromeOptions": { ...(chromeBinary ? { binary: chromeBinary } : {}), args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] },
    },
  });
  sessions.push(b);
  await b.url(`${origin}/owd/app`);
  await until(b, "the app", async () => (await exists(b, "file-label")) && (await exists(b, "web-label")));
  await until(b, "the map", () => b.execute(() => !!window.__owdMap));
  return b;
}
const until = (b, what, check, timeout = 60000) => b.waitUntil(check, { timeout, interval: 150, timeoutMsg: `timed out waiting for: ${what}` });
const tid = (b, id) => b.$(`[data-testid="${id}"]`);
const text = async (b, id) => (await tid(b, id)).getText();
const exists = async (b, id) => (await tid(b, id)).isExisting();
async function pick(b, open, file) {
  await b.execute(() => {
    HTMLInputElement.prototype.click = function () {};
  });
  await open();
  const picker = await tid(b, "file-picker");
  await picker.waitForExist({ timeout: 5000 });
  await picker.setValue(file);
}
/** Load a feature file through its row's Load… button and the file chooser, as a user does. */
async function loadFeatures(b, role, name, data) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-features-file-"));
  const file = path.join(dir, name);
  fs.writeFileSync(file, JSON.stringify(data));
  await pick(b, async () => (await tid(b, `load-${role}`)).click(), file);
  return true;
}
const box = (b, id) =>
  b.execute((i) => {
    const c = document.querySelector(`[data-testid="layer-${i}"]`);
    return { checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
  }, id);
const featuresAttr = async (b, name) => (await tid(b, "map-features")).getAttribute(name);
const savedText = async (b) => {
  await b.execute(() => window.__owdPlatform.call("snapshot"));
  await new Promise((r) => setTimeout(r, 300));
  return b.execute(() => JSON.parse(localStorage.getItem("owd.lastDesign")).text);
};

/** The example with three pools, 1.0 m, 2.5 m and 4.0 m, as the MAP-4 smoke test makes it. */
function poolsDesign() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-features-"));
  const doc = path.join(dir, "river-pools.owd");
  fs.copyFileSync(path.join(FIXTURES, "river-example.owd"), doc);
  fs.copyFileSync(path.join(FIXTURES, "river-dem.tif"), path.join(dir, "river-dem.tif"));
  fs.writeFileSync(path.join(dir, "pools.csv"), "chainage_m,level_m\n0,1.0\n1000,2.5\n2000,4.0\n");
  execFileSync(CLI, ["import", doc, "--water-pools", path.join(dir, "pools.csv")], { encoding: "utf8" });
  return doc;
}

describe("buildings, places and settlements (BLD-1)", () => {
  it("the Buildings layer: greyed with none, loaded by Load…, footprints near, dots and a density far, remembered, flooded by the pool water with a column in the Land table, a click card, and the design text unchanged by the ticks", async () => {
    const b = await start();
    const doc = poolsDesign();
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box(b, "buildings"), { checked: false, disabled: true, row: "Buildings none loaded" });
    assert.deepEqual(await box(b, "places"), { checked: true, disabled: true, row: "Places none loaded" });
    assert.deepEqual(await box(b, "settlements"), { checked: false, disabled: true, row: "Settlements none loaded" });
    await (await tid(b, "layers-button")).click();
    await pick(b, async () => (await b.$("button=Open")).click(), doc);
    await until(b, "the design", async () => (await text(b, "file-label")).includes("river-pools.owd"));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));

    // 400 buildings within 300 m of the river: recorded by reference, so the design stays small.
    const sizeBefore = (await savedText(b)).length;
    assert.equal(await loadFeatures(b, "buildings", "buildings.geojson", buildings(400, { reach: 300 })), true);
    await until(b, "the buildings row", async () => /^400 \(\d+ counted\)$/.test(await text(b, "data-buildings"))).catch(async (e) => {
      throw new Error(`${e.message}: the row reads "${await text(b, "data-buildings")}"; ${JSON.stringify(await b.execute(async () => (await window.__owdPlatform.call("snapshot")).feature_inputs))}`);
    });
    const rec = (await b.execute(async () => (await window.__owdPlatform.call("snapshot")).document.inputs.buildings));
    assert.deepEqual([rec.path, rec.count, rec.crs, rec.min_confidence], ["buildings.geojson", 400, "WGS 84", 0.7]);
    assert.ok((await savedText(b)).length - sizeBefore < 400, "the design grows by the reference only");

    // The tick comes on; still off until ticked. Ticked: the map has the buildings.
    await (await tid(b, "layers-button")).click();
    await until(b, "the buildings tick", async () => !(await box(b, "buildings")).disabled);
    assert.deepEqual(await box(b, "buildings"), { checked: false, disabled: false, row: "Buildings" });
    const textBefore = await savedText(b);
    await (await tid(b, "layer-buildings")).click();
    await until(b, "the buildings on the map", async () => (await featuresAttr(b, "data-buildings")) === "on", 60000);
    const layers = await b.execute(() => ["buildings-density", "buildings-dots", "buildings-fill", "buildings-outline"].map((id) => [id, window.__owdMap.getLayoutProperty(id, "visibility"), window.__owdMap.getLayer(id).minzoom ?? 0, window.__owdMap.getLayer(id).maxzoom ?? 24]));
    assert.deepEqual(layers, [["buildings-density", "visible", 0, 13], ["buildings-dots", "visible", 12, 24], ["buildings-fill", "visible", 14, 24], ["buildings-outline", "visible", 14, 24]]);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["buildings"]');

    // The pool water: the buildings it covers turn the warning colour, and the Land table counts them by pool.
    await (await tid(b, "layer-pool-water")).click();
    await until(b, "the pool water", async () => (await (await tid(b, "plan-map")).getAttribute("data-pool-water")) === "on", 120000);
    await until(b, "the flooded buildings", async () => Number(await featuresAttr(b, "data-flooded")) > 0, 60000);
    const flooded = Number(await featuresAttr(b, "data-flooded"));
    const fids = await b.execute(async (id) => (await window.__owdPlatform.call("features_flooded", { key: "buildings", pool: id, minDepth: 0.3 })).fids, Number(await b.execute(async () => (await window.__owdPlatform.call("pool_water_info")).id)));
    assert.equal(fids.length, flooded);
    assert.equal(await b.execute((fid) => window.__owdMap.getFeatureState({ source: "buildings", id: fid }).flooded, fids[0]), true);
    await (await tid(b, "pool-land-button")).click();
    await until(b, "the land table", () => exists(b, "pool-land-table"));
    assert.equal(await text(b, "pool-land-buildings-head"), "Buildings flooded");
    const column = await b.execute(() => [...document.querySelectorAll('[data-testid="pool-land-buildings"]')].map((e) => Number(e.textContent)));
    assert.equal(column.length, 3);
    assert.ok(column.reduce((a, c) => a + c, 0) <= flooded, `${column} counts only counted buildings`);

    // A click on a flooded building: its card reads the area, the confidence, the ground, the water and the line.
    await b.execute((fid) => {
      const m = window.__owdMap;
      const f = m.getSource("building-points").serialize().data.features.find((x) => x.properties.fid === fid);
      m.jumpTo({ center: f.geometry.coordinates, zoom: 16 });
    }, fids[0]);
    await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()));
    const at = await b.execute((fid) => {
      const m = window.__owdMap;
      const f = m.getSource("building-points").serialize().data.features.find((x) => x.properties.fid === fid);
      const p = m.project({ lng: f.geometry.coordinates[0], lat: f.geometry.coordinates[1] });
      const r = m.getCanvas().getBoundingClientRect();
      return { x: Math.round(r.left + p.x), y: Math.round(r.top + p.y) };
    }, fids[0]);
    await b.action("pointer").move({ x: at.x, y: at.y, origin: "viewport" }).down().up().perform();
    await until(b, "the building card", () => exists(b, "feature-card"));
    const card = await text(b, "feature-card");
    for (const want of [/Area\s+\d+ m²/, /Confidence\s+0\.\d\d/, /Ground level\s+-?\d+\.\d\d m/, /Pool water level\s+\d\.\d\d m \(pool \d\)/, /Depth of flooding\s+\d+\.\d\d m/, /To the design line\s+\d+ m/, /To the (daylight line|channel's water edge)/, /Machine-detected footprint, not a survey/]) assert.match(card, want);
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-buildings-flooded.png"));
    await (await tid(b, "feature-card-close")).click();

    // The ticks never touch the design.
    await (await tid(b, "layer-pool-water")).click();
    await (await tid(b, "layer-buildings")).click();
    await (await tid(b, "layer-buildings")).click();
    assert.equal(await savedText(b), textBefore);
    // Remembered after a reload; the browser keeps the design but not the file, so the tick is greyed
    // and says the file is missing until it is loaded again.
    await b.refresh();
    await until(b, "the app", async () => await exists(b, "file-label"));
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["buildings"]');
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box(b, "buildings"), { checked: true, disabled: true, row: "Buildings file missing" });
    assert.match(await text(b, "data-buildings"), /^missing: buildings\.geojson$/);
  });


  it("places with their names, kinds and blocks, and settlements with the share a pool floods: loaded by Load…, decluttered with the larger place kept, a legend of the kinds, click cards, and the design text unchanged by the ticks", async () => {
    const b = await start();
    const doc = poolsDesign();
    await pick(b, async () => (await b.$("button=Open")).click(), doc);
    await until(b, "the design", async () => (await text(b, "file-label")).includes("river-pools.owd"));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    await loadFeatures(b, "points_of_interest", "places.geojson", places());
    await until(b, "the places row", async () => (await text(b, "data-points_of_interest")) === "6 places");
    await loadFeatures(b, "settlements", "settlements.geojson", settlements());
    await until(b, "the settlements row", async () => (await text(b, "data-settlements")) === "2 settlements");
    const textBefore = await savedText(b);

    // Places start on; their names are pictures the page draws, one per distinct kind and name.
    await until(b, "the places on the map", async () => Number(await featuresAttr(b, "data-places")) === 6);
    assert.equal(await featuresAttr(b, "data-place-kinds"), "community bridge quay other");
    assert.equal(await featuresAttr(b, "data-place-blocks"), "2");
    await until(b, "the legend", () => exists(b, "place-legend"));
    assert.equal((await text(b, "place-legend")).replace(/\s+/g, " ").trim(), "Community B Bridge Q Quay Other");
    const riverbend = places().features[0].geometry.coordinates;
    await b.execute((c) => void window.__owdMap.jumpTo({ center: c, zoom: 17 }), riverbend);
    await until(b, "every name drawn", async () => Number(await featuresAttr(b, "data-place-images")) >= 6, 30000);
    const ids = await b.execute(() => window.__owdMap.listImages().filter((i) => i.startsWith("owd-place:")).sort());
    for (const want of ["owd-place:community:Riverbend", "owd-place:community:Little Ford", "owd-place:bridge:Old Crossing", "owd-place:quay:North Wharf", "owd-place:other:Pump House"]) assert.ok(ids.includes(want), `${want} in ${ids}`);
    assert.equal(new Set(ids).size, ids.length);
    // Further out Riverbend (45.5 ha) and Little Ford (2.1 ha), 36 m apart, collide: the larger stays.
    await b.execute((c) => void window.__owdMap.jumpTo({ center: c, zoom: 13 }), riverbend);
    await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()));
    await new Promise((r) => setTimeout(r, 500));
    const shown = await b.execute(() => window.__owdMap.queryRenderedFeatures({ layers: ["places"] }).map((f) => f.properties.img));
    assert.ok(shown.includes("owd-place:community:Riverbend"), `${shown}`);
    assert.ok(!shown.includes("owd-place:community:Little Ford"), `${shown}`);
    // The bridge's block: a deck along its bearing, 120 m by 8 m, marked a placeholder.
    const deck = await b.execute(() => window.__owdMap.getSource("places-blocks").serialize().data.features.find((f) => f.properties.kind === "bridge"));
    const ring = deck.geometry.coordinates[0];
    const lat = (ring[1][1] - ring[0][1]) * 111320;
    assert.ok(Math.abs(lat - 120) < 0.01, `${lat}`);
    assert.equal(deck.properties.note, "placeholder model, not a design");

    // The pool water on; a click on Riverbend reads its name, kind, properties, ground, water and line.
    await (await tid(b, "layers-button")).click();
    await (await tid(b, "layer-pool-water")).click();
    await until(b, "the pool water", async () => (await (await tid(b, "plan-map")).getAttribute("data-pool-water")) === "on", 120000);
    const clickAt = async (lonlat) => {
      await b.execute((ll) => void window.__owdMap.jumpTo({ center: ll, zoom: 16 }), lonlat);
      await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()));
      await new Promise((r) => setTimeout(r, 400));
      const at = await b.execute((ll) => {
        const m = window.__owdMap;
        const p = m.project({ lng: ll[0], lat: ll[1] });
        const r = m.getCanvas().getBoundingClientRect();
        return { x: Math.round(r.left + p.x), y: Math.round(r.top + p.y) };
      }, lonlat);
      await b.action("pointer").move({ x: at.x, y: at.y, origin: "viewport" }).down().up().perform();
    };
    await clickAt(riverbend);
    await until(b, "the place card", async () => (await exists(b, "feature-card")) && /Riverbend/.test(await text(b, "feature-card")));
    const card = await text(b, "feature-card");
    for (const want of [/Place: Riverbend/, /Kind\s+Community/, /area_ha\s+45\.5/, /parts\s+3/, /Ground level\s+(-?\d+\.\d\d m|unknown)/, /(Flooding\s+dry|Depth of flooding)/, /To the design line\s+\d+ m/, /Chainage of the nearest point\s+\d+\.\d{3} km/]) assert.match(card, want);
    await (await tid(b, "feature-card-close")).click();

    // Settlements: off at first; ticked, a click inside one reads the share of it the pool water covers.
    assert.deepEqual(await box(b, "settlements"), { checked: false, disabled: false, row: "Settlements" });
    await (await tid(b, "layer-settlements")).click();
    await (await tid(b, "layer-places")).click();
    const inside = settlements().features[0].geometry.coordinates[0];
    await clickAt([(inside[0][0] + inside[2][0]) / 2, (inside[0][1] + inside[2][1]) / 2]);
    await until(b, "the settlement card", () => exists(b, "feature-card"));
    assert.match(await text(b, "feature-card"), /Area flooded\s+\d+ %/);
    assert.match(await text(b, "feature-card"), /Area\s+16\.0 ha/);
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-places.png"));

    // The ticks never touch the design.
    await (await tid(b, "layer-settlements")).click();
    await (await tid(b, "layer-places")).click();
    await (await tid(b, "layer-pool-water")).click();
    assert.equal(await savedText(b), textBefore);
  });

  it("measures the map with 36,000 buildings: the time to show them, and the frame time while panning", async (t) => {
    const b = await start();
    // Software WebGL is slow: the measuring script may take minutes.
    await b.setTimeout({ script: 600000 });
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    const data = buildings(36000, { reach: 1500 });
    const t0 = Date.now();
    assert.equal(await loadFeatures(b, "buildings", "buildings-36k.geojson", data), true);
    await until(b, "the 36,000 buildings in the design", async () => (await text(b, "data-buildings")).startsWith("36,000"), 120000);
    const loadMs = Date.now() - t0;
    await (await tid(b, "layers-button")).click();
    await until(b, "the buildings tick", async () => !(await box(b, "buildings")).disabled);
    await (await tid(b, "layer-buildings")).click();
    await until(b, "36,000 buildings on the map", async () => (await featuresAttr(b, "data-buildings")) === "on", 120000);
    const showMs = Number(await featuresAttr(b, "data-buildings-ms"));
    // Frames while panning at three zoom levels (the density, the dots and the footprints), with the
    // buildings shown and, as the baseline, hidden.
    const pan = () => b.execute(async () => {
      const m = window.__owdMap;
      const out = {};
      for (const z of [11, 13, 15]) {
        m.jumpTo({ center: [-2.9865, 5.4271], zoom: z });
        await new Promise((r) => m.once("idle", r));
        const times = [];
        let last = performance.now();
        const step = () =>
          new Promise((r) => {
            m.panBy([40, 0], { animate: false });
            requestAnimationFrame(() => {
              const now = performance.now();
              times.push(now - last);
              last = now;
              r();
            });
          });
        for (let k = 0; k < 30; k++) await step();
        times.sort((a, b) => a - b);
        out[z] = { median_ms: Math.round(times[15] * 10) / 10, p90_ms: Math.round(times[27] * 10) / 10 };
      }
      return out;
    });
    const frames = await pan();
    await (await tid(b, "layer-buildings")).click();
    const baseline = await pan();
    t.diagnostic(`36,000 buildings: load into the design ${loadMs} ms; tiled and shown ${showMs} ms; frames while panning (software WebGL) with the buildings ${JSON.stringify(frames)}, without ${JSON.stringify(baseline)}`);
    fs.writeFileSync(path.join(os.tmpdir(), "owd-buildings-36k-measure.json"), JSON.stringify({ loadMs, showMs, frames, baseline }, null, 2));
    assert.ok(showMs < 60000, `${showMs} ms`);
  });
});
