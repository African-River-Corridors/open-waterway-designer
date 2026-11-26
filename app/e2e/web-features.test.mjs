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
/** Load a feature file through the same command as Load… and drag and drop. */
const loadFeatures = (b, role, name, data) =>
  b.execute(
    async (role, name, text) => {
      const file = new File([text], name, { type: "application/geo+json" });
      return (await window.__owdPlatform.call("load_input", { role, path: file })) !== null;
    },
    role,
    name,
    JSON.stringify(data),
  );
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
    await until(b, "the buildings row", async () => /^400 \(\d+ counted\)$/.test(await text(b, "data-buildings")));
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
    await b.execute((fid) => {
      const m = window.__owdMap;
      const f = m.getSource("building-points").serialize().data.features.find((x) => x.properties.fid === fid);
      const ll = { lng: f.geometry.coordinates[0], lat: f.geometry.coordinates[1] };
      m.fire("click", { point: m.project(ll), lngLat: ll });
    }, fids[0]);
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

  it("measures the map with 36,000 buildings: the time to show them, and the frame time while panning", async (t) => {
    const b = await start();
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    const data = buildings(36000, { reach: 1500 });
    const t0 = Date.now();
    assert.equal(await loadFeatures(b, "buildings", "buildings-36k.geojson", data), true);
    const loadMs = Date.now() - t0;
    await (await tid(b, "layers-button")).click();
    await until(b, "the buildings tick", async () => !(await box(b, "buildings")).disabled);
    await (await tid(b, "layer-buildings")).click();
    await until(b, "36,000 buildings on the map", async () => (await featuresAttr(b, "data-buildings")) === "on", 120000);
    const showMs = Number(await featuresAttr(b, "data-buildings-ms"));
    // Frames while panning at three zoom levels: the density, the dots and the footprints.
    const frames = await b.execute(async () => {
      const m = window.__owdMap;
      const out = {};
      for (const z of [11, 13, 15]) {
        m.jumpTo({ center: [-2.9865, 5.4271], zoom: z });
        await new Promise((r) => m.once("idle", r));
        const times = [];
        let last = performance.now();
        const step = (k) =>
          new Promise((r) => {
            m.panBy([40, 0], { animate: false });
            requestAnimationFrame(() => {
              const now = performance.now();
              times.push(now - last);
              last = now;
              r();
            });
          });
        for (let k = 0; k < 30; k++) await step(k);
        times.sort((a, b) => a - b);
        out[z] = { median_ms: Math.round(times[15] * 10) / 10, p90_ms: Math.round(times[27] * 10) / 10 };
      }
      return out;
    });
    t.diagnostic(`36,000 buildings: load into the design ${loadMs} ms; tiled and shown ${showMs} ms; frames while panning (software WebGL) ${JSON.stringify(frames)}`);
    fs.writeFileSync(path.join(os.tmpdir(), "owd-buildings-36k-measure.json"), JSON.stringify({ loadMs, showMs, frames }, null, 2));
    assert.ok(showMs < 60000, `${showMs} ms`);
  });
});
