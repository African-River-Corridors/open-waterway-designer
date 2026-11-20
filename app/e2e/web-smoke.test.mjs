// Web smoke test (spec 0011, T4): the built web app in headless Chrome, served like the live site. Run by
// e2e/run-web.sh. Checks: the page loads with its web label and no Fetch DEM; the bundled example opens,
// a changed input moves the result, and the result equals native `owd run` (ADR 0005 tolerance); a
// design and its DEM load through the file picker and a full run (the pool search, in wasm) equals native;
// Cancel leaves the design as it was; a reload brings the design back; blocked storage still loads.
import { after, before, describe, it } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { serve } from "./web-server.mjs";
import { reportEngineHost } from "./report-engine-hosts.mjs";
import { catalogueHost } from "./catalogue-hosts.mjs";
import { endpoints, searches, serveReach, slow } from "./reach-server.mjs";
import { compare } from "../../tools/web_parity_compare.mjs";

const here = import.meta.dirname;
const FIXTURES = path.resolve(here, "../../examples/web");
const DIST = path.resolve(here, "../dist-web-test");
const CLI = process.env.OWD_CLI;
assert.ok(CLI, "OWD_CLI must be set (run e2e/run-web.sh)");
const SHOTS = process.env.OWD_WEB_SHOTS;

/** The document `owd run` leaves for a fixture, after the same `owd size` edit (if any) the app made. */
function nativeDoc(file, sizeArgs) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-smoke-"));
  const doc = path.join(dir, file);
  fs.copyFileSync(path.join(FIXTURES, file), doc);
  fs.copyFileSync(path.join(FIXTURES, "river-dem.tif"), path.join(dir, "river-dem.tif"));
  const owd = (args) => {
    try {
      execFileSync(CLI, args, { encoding: "utf8", maxBuffer: 1 << 28 });
    } catch (e) {
      if (e.status !== 1) throw e; // exit 1: a check failed; the document is still written
    }
  };
  if (sizeArgs) owd(["size", doc, ...sizeArgs]);
  owd(["run", doc]);
  return JSON.parse(fs.readFileSync(doc, "utf8"));
}

const chrome = (extra = {}) => ({
  browserName: "chrome",
  "wdio:enforceWebDriverClassic": true,
  "goog:chromeOptions": {
    ...(process.env.CHROME_BINARY ? { binary: process.env.CHROME_BINARY } : fs.existsSync("/Applications/Google Chrome.app") ? { binary: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" } : {}),
    args: ["--headless=new", "--no-sandbox", "--disable-gpu", "--window-size=1500,950"],
    ...extra,
  },
});

let origin, server;
const sessions = [];
async function start(options = {}) {
  const b = await remote({ logLevel: "warn", strictSelectors: false, capabilities: chrome(options) });
  sessions.push(b);
  return b;
}
const until = (b, what, check, timeout = 60000) => b.waitUntil(check, { timeout, interval: 150, timeoutMsg: `timed out waiting for: ${what}` });
const tid = (b, id) => b.$(`[data-testid="${id}"]`);
const text = async (b, id) => (await tid(b, id)).getText();
const exists = async (b, id) => (await tid(b, id)).isExisting();
const loaded = (b) => until(b, "the app", async () => (await exists(b, "file-label")) && (await exists(b, "web-label")));
/** The document the page holds now. Only a build in test mode has the hook. */
const pageDoc = (b) => b.execute(async () => (await window.__owdPlatform.call("snapshot")).document);
const stageStatus = (b) => b.execute(async () => (await window.__owdPlatform.call("snapshot")).stages.map((s) => `${s.name}:${s.status}`).join(" "));
/** The browser's file chooser would be cancelled in headless Chrome: stop the page opening it, fill the input in. */
async function pick(b, open, file) {
  await b.execute(() => {
    HTMLInputElement.prototype.click = function () {};
  });
  await open();
  const picker = await tid(b, "file-picker");
  await picker.waitForExist({ timeout: 5000 });
  await picker.setValue(file);
}
const openButton = (b) => b.$("button=Open");
const sameAsNative = (page, native, what) => {
  // One design line per run (spec 0017): it is in the stages, so the stages are the whole answer.
  const r = compare(native.stages, page.stages);
  assert.deepEqual(r.problems.slice(0, 5), [], `${what}: stages equal to native (ADR 0005 tolerance)`);
  assert.ok(r.numbers > 0, `${what}: stages had numbers to compare`);
};

let reach;
before(async () => {
  ({ server, origin } = await serve(DIST));
  reach = await serveReach();
});
after(async () => {
  for (const b of sessions) await b.deleteSession().catch(() => {});
  server?.close();
  reach?.server.close();
});

/** The public hosts the web build may name: the open-data sources (spec 0012), the basemap, and links. */
const PUBLIC_HOSTS = [
  "overpass-api.de",
  "overpass.kumi.systems", // public Overpass mirrors, asked when the main server fails (#319)
  "overpass.private.coffee",
  "photon.komoot.io",
  "nominatim.openstreetmap.org",
  "earth-search.aws.element84.com",
  "sentinel-cogs.s3.us-west-2.amazonaws.com",
  "zenodo.org",
  "soto.podaac.earthdatacloud.nasa.gov",
  "storage.googleapis.com",
  "copernicus-dem-30m.s3.eu-central-1.amazonaws.com",
];

describe("OWD web smoke tests", () => {
  it("loads with the web label and the desktop link, and has no Fetch DEM", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    assert.match((await text(b, "web-label")).toLowerCase(), /web version/);
    assert.equal(await (await b.$("a=Get the desktop app")).getAttribute("href"), "/owd/docs#install");
    assert.equal(await exists(b, "fetch-dem"), false, "Fetch DEM is not offered in the browser");
    assert.equal(await exists(b, "error"), false);
  });

  it("opens the example, a changed input moves the result, and the result equals owd run", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the option table", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    assert.equal(await stageStatus(b), "sizing:fresh alignment:fresh optimise:not_run volumes:fresh");
    const dredge = async () => (await (await b.$("[data-testid=option-row]")).getText()).replace(/\s+/g, " ");
    const before = await dredge();
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await new Promise((r) => setTimeout(r, 1500));
      await b.saveScreenshot(path.join(SHOTS, "web-example.png"));
    }
    // Change one input: the draught.
    await (await b.$("input[aria-label=Draught]")).setValue("2.6");
    await until(b, "later stages to go stale", async () => (await stageStatus(b)).includes("volumes:stale"));
    await (await b.$("button=Run stale")).click();
    await until(b, "volumes fresh again", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:not_run volumes:fresh");
    const after = await dredge();
    assert.notEqual(after, before, "the result moved");
    sameAsNative(await pageDoc(b), nativeDoc("river-example.owd", ["--rule-set", "pianc-wg141", "--set-input", "se_quality=B", "--vessel", "CEMT_IV", "--draught", "2.6"]), "example at draught 2.6");
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-example-changed.png"));
    // The design is kept: a reload brings it back (with its unsaved changes, as saved text).
    await b.url(`${origin}/owd/app`);
    await until(b, "the restored design", async () => (await exists(b, "file-label")) && (await text(b, "file-label")).includes("river-example.owd"));
  });

  it("writes the screening report as a PDF download from the Report button (spec 0014)", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the option table", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    await (await tid(b, "sheets-toggle")).click();
    await (await tid(b, "report-toggle")).click();
    await (await tid(b, "export-report")).click();
    await until(b, "the report", async () => (await exists(b, "notice")) && /Report written: owd-.+-sr\.pdf/.test(await text(b, "notice")), 240000);
    assert.equal(await exists(b, "error"), false);
  });

  it("lists the shared vessel catalogue by family, finds a class by search, and sizes for it", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    const select = await b.$("select[aria-label='Vessel class']");
    // The example's class, by its catalogue id; the list groups the catalogue by family.
    assert.equal(await select.getValue(), "CEMT-IV");
    const groups = await b.execute(() => [...document.querySelectorAll("select[aria-label='Vessel class'] optgroup")].map((g) => g.label));
    assert.ok(groups.length > 3, `${groups}`);
    assert.equal(groups[0], "CEMT-1992");
    const count = await b.execute(() => document.querySelectorAll("select[aria-label='Vessel class'] option").length);
    assert.ok(count > 12, `${count} classes`);
    // The search keeps the classes it matches, and the one chosen.
    await (await tid(b, "vessel-search")).setValue("VIb");
    await until(b, "the search to narrow the list", async () => (await exists(b, "vessel-search-count")) && /of \d+ classes/.test(await text(b, "vessel-search-count")));
    const ids = await b.execute(() => [...document.querySelectorAll("select[aria-label='Vessel class'] option")].map((o) => o.value));
    assert.ok(ids.includes("CEMT-VIb-motor") && ids.includes("CEMT-IV") && ids.length < count, `${ids}`);
    await b.execute(() => {
      const sel = document.querySelector("select[aria-label='Vessel class']");
      sel.value = "CEMT-VIb-motor";
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "the design sized for CEMT-VIb-motor", async () => (await pageDoc(b)).stages.sizing.results?.vessel_standard_id === "CEMT-VIb-motor");
    const sized = (await pageDoc(b)).stages.sizing.results;
    assert.equal(sized.loa_m, 140);
    assert.equal(sized.beam_m, 15);
  });

  it("chooses Custom vessel…, sees the fields filled from the class, edits the beam, sizes, and sees the label; a refused beam turns Size off", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    const before = (await pageDoc(b)).stages.sizing.results;
    await b.execute(() => {
      const sel = document.querySelector("select[aria-label='Vessel class']");
      sel.value = "__custom__";
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "the custom vessel's fields", async () => exists(b, "custom-vessel"));
    const value = (id) => b.execute((i) => document.querySelector(`[data-testid=custom-${i}]`).value, id);
    // Filled from the class that was selected.
    assert.equal(await value("name"), "CEMT-IV (custom)");
    assert.equal(Number(await value("loa_m")), before.loa_m);
    assert.equal(Number(await value("beam_m")), before.beam_m);
    assert.equal(Number(await value("draught_m")), before.draught_m);
    assert.equal(await value("formation"), "motor");
    const type = (id, v) =>
      b.execute(
        (i, x) => {
          const el = document.querySelector(`[data-testid=custom-${i}]`);
          el.value = x;
          el.dispatchEvent(new Event("input", { bubbles: true }));
        },
        id,
        v,
      );
    const sizeOn = () => b.execute(() => !document.querySelector("[data-testid=custom-size]").disabled);
    // A beam not less than the length is refused next to its field, and Size stays off.
    await type("beam_m", "90");
    await until(b, "the beam refused at its field", async () => exists(b, "custom-error-beam_m"));
    assert.equal(await sizeOn(), false);
    // A beam that can be right: Size turns on, and sizing gives the label.
    await type("beam_m", "11");
    await until(b, "Size to turn on", async () => (await sizeOn()) && !(await exists(b, "custom-error-beam_m")));
    await (await tid(b, "custom-size")).click();
    const label = `Custom: CEMT-IV (custom), ${before.loa_m} × 11 × ${before.draught_m} m`;
    await until(b, "the design sized for the custom vessel", async () => (await pageDoc(b)).stages.sizing.results?.vessel_label === label);
    const doc = await pageDoc(b);
    assert.equal(doc.stages.sizing.settings.vessel_standard_id, null);
    assert.equal(doc.stages.sizing.settings.custom_vessel.beam_m, 11);
    assert.equal(await text(b, "shape-vessel"), label);
  });

  it("picks the quality in the Rule Set picker, runs, shows the verdict, design basis and stretches, and a picked stretch on the map", async () => {
    // The map needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.equal(await (await tid(b, "rule-set-0")).getValue(), "pianc-wg141");
    assert.equal(await (await tid(b, "set-input-se_quality")).getValue(), "B");
    assert.match(await text(b, "source-input-se_quality"), /typed/);
    // Pick quality A: sizing reruns at once and the later stages go stale.
    await b.execute(() => {
      const s = document.querySelector("[data-testid=set-input-se_quality]");
      s.value = "A";
      s.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "sizing at quality A", async () => (await text(b, "shape-rule-sets")).includes("se_quality A"));
    await until(b, "later stages to go stale", async () => (await stageStatus(b)).includes("volumes:stale"));
    assert.ok((await b.$$("[data-testid=basis-row]")).length >= 3, "the design basis table");
    assert.match(await text(b, "rules-notice"), /unverified/);
    await (await b.$("button=Run stale")).click();
    await until(b, "volumes fresh again", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:not_run volumes:fresh", 120000);
    // The same as `owd size --set-input se_quality=A` then `owd run`.
    sameAsNative(await pageDoc(b), nativeDoc("river-example.owd", ["--set-input", "se_quality=A"]), "example at quality A");
    // The verdict, in words, and the stretches of the option, as the document holds them.
    const doc = await pageDoc(b);
    const sc = doc.stages.volumes.results.screening;
    await until(b, "the verdict", async () => (await b.$$("[data-testid=verdict]")).length === 1);
    assert.equal(await (await tid(b, "verdict")).getAttribute("data-verdict"), sc.verdict);
    assert.match(await text(b, "verdict"), new RegExp(sc.verdict.replace(/-/g, " ")));
    // #312: the radius column and the R_min check read the same design line and agree; the verdict says why.
    const rminPassed = await b.execute(() => [...document.querySelectorAll("[data-testid=check]")].find((c) => /\bR_min:/.test(c.textContent))?.dataset.passed);
    const radiusCell = await b.execute(() => document.querySelector("[data-testid=option-row] [data-col=radius-ok]").textContent);
    assert.equal(radiusCell.includes("✓"), rminPassed === "true", `radius column "${radiusCell}", R_min check ${rminPassed}`);
    assert.ok((await text(b, "verdict-reason")).length > 0, "the verdict's reason is shown");
    await (await tid(b, "tab-stretches")).click();
    assert.ok(sc.constraint_stretches.length > 0, "the example falls short of the Rules somewhere");
    await until(b, "the stretch table", async () => (await b.$$("[data-testid=stretch-row]")).length === sc.constraint_stretches.length);
    assert.match(await text(b, "screening-notice"), /Screening/);
    const last = sc.constraint_stretches.length - 1;
    await b.execute((i) => document.querySelector(`[data-testid=stretch-row][data-index="${i}"]`).click(), last);
    const option = doc.stages.volumes.results.id;
    await until(b, "the map to show the picked stretch", async () => (await (await tid(b, "plan-map")).getAttribute("data-stretch")) === `${option}:${last}`);
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-rule-sets.png"));
    }
  });

  it("edits a bank vertex: the mark turns edited, the stages go stale, a crossing is refused, and undo brings it back", async () => {
    // The map needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    // The example has no bank lines: load two of our own (marked `user`), and rerun what that made stale.
    const bankFile = (side) => path.join(FIXTURES, `river-bank-${side}.geojson`);
    await pick(b, async () => (await tid(b, "load-left_bank")).click(), bankFile("left"));
    await until(b, "the left bank", async () => (await (await tid(b, "data-left_bank")).getText()).length > 0 && !(await (await tid(b, "data-left_bank")).getText()).includes("not loaded"));
    await pick(b, async () => (await tid(b, "load-right_bank")).click(), bankFile("right"));
    await until(b, "the editor to be ready", async () => (await tid(b, "bank-edit-toggle")).isEnabled());
    await (await b.$("button=Run stale")).click();
    const fresh = "sizing:fresh alignment:fresh optimise:not_run volumes:fresh";
    await until(b, "stages fresh", async () => (await stageStatus(b)) === fresh, 120000);
    await (await tid(b, "bank-edit-toggle")).click();
    assert.equal(await (await tid(b, "mark-left_bank")).getAttribute("data-mark"), "user", "a loaded line is marked user");
    await (await tid(b, "bank-tool-delete")).click();
    await until(b, "vertex handles", async () => (await b.$$("[data-testid=bank-vertex]")).length > 2);
    const handles = await b.$$("[data-testid=bank-vertex]");
    const open = [];
    for (const h of handles) if (await h.isClickable()) open.push(h);
    assert.ok(open.length > 2, "some vertex handles are clear of the panel");
    await open[Math.floor(open.length / 2)].click();
    await until(b, "the left bank to be marked edited", async () => (await (await tid(b, "mark-left_bank")).getAttribute("data-mark")) === "edited");
    assert.equal(await (await tid(b, "mark-river_polygon")).getAttribute("data-mark"), "edited");
    assert.equal(await (await tid(b, "mark-centreline")).getAttribute("data-mark"), "edited");
    assert.ok((await stageStatus(b)).includes("alignment:stale"), await stageStatus(b));
    // A move that crosses the other bank is refused with a message, and nothing changes.
    const before = await pageDoc(b);
    const refusal = await b.execute(async () => {
      const g = await window.__owdPlatform.call("map_geometry");
      const right = g.banks.find((x) => x.side === "right").line.coordinates;
      try {
        await window.__owdPlatform.call("edit_bank", { side: "left_bank", edit: { kind: "move", index: 1, to: right[Math.floor(right.length / 2)] } });
        return "accepted";
      } catch (e) {
        return String(e);
      }
    });
    assert.match(refusal, /cross/);
    assert.deepEqual((await pageDoc(b)).inputs, before.inputs, "a refused edit changes nothing");
    await (await tid(b, "bank-undo")).click();
    await until(b, "the stages to be fresh again", async () => (await stageStatus(b)) === fresh);
    assert.equal(await (await tid(b, "mark-left_bank")).getAttribute("data-mark"), "user", "undo brings the old record back");
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-bank-editor.png"));
    }
  });

  it("finds a river as you type, draws it from its mouth to its upstream end, and cuts a reach out of it", async () => {
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    // The first Overpass server is busy (502): the next one answers, and the panel says so (#319).
    const at = { ...endpoints(reach.origin), overpass: `${reach.origin}/down/api/interpreter,${endpoints(reach.origin).overpass}` };
    await b.execute(async (at) => window.__owdPlatform.call("set_reach_endpoints", { endpoints: at }), at);
    await (await tid(b, "start-from-river")).click();
    const before = searches.length;
    // Typed key by key: one request, after the pause, for "Ank" (none for "A" or "An").
    await (await tid(b, "reach-river")).setValue("Ank");
    const ankobra = '[data-testid="reach-hit"][data-osm="relation/3328713"]';
    await until(b, "the list", async () => (await b.$(ankobra)).isExisting());
    assert.match(await (await b.$(ankobra)).getText(), /Ankobra River — Western Region, Ghana/);
    assert.deepEqual(searches.slice(before), ["Ank"], "one request per pause, none under 3 letters");
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-river-search.png"));
    }
    await (await b.$(`${ankobra} button`)).click();
    await until(b, "the whole river", async () => (await exists(b, "reach-length")) && /km/.test(await text(b, "reach-length")));
    assert.equal(await (await tid(b, "reach-start")).getValue(), "-2.26885, 4.89839", "Start is the mouth");
    assert.match(await text(b, "reach-route-notes"), /Overpass: 127\.0\.0\.1:\d+ failed \(.*502.*\); 127\.0\.0\.1:\d+ answered/);
    assert.equal(await (await tid(b, "reach-end")).getValue(), "-2.13492, 5.72949", "End is the upstream end");
    const whole = Number((await text(b, "reach-length")).match(/([\d.]+) km/)[1]);
    const route = (await pageDoc(b)).provenance.route;
    assert.equal(route.settings.osm_id, "relation/3328713");
    assert.equal(route.settings.river_search.source, "osm-photon");
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-river-route.png"));
    // Move End to the middle of the river (as a drag does) and cut again: shorter, no new request.
    const asked = searches.length;
    await (await tid(b, "reach-end")).setValue("-2.1533567, 5.3025015");
    await (await tid(b, "reach-route")).click();
    await until(b, "the shorter reach", async () => Number((await text(b, "reach-length")).match(/([\d.]+) km/)[1]) < whole);
    assert.equal(searches.length, asked);
  });

  it("says plainly when the river search fails, and Enter asks Nominatim once", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await b.execute(async (at) => window.__owdPlatform.call("set_reach_endpoints", { endpoints: at }), endpoints(reach.origin));
    await (await tid(b, "start-from-river")).click();
    const before = searches.length;
    const box = await tid(b, "reach-river");
    await box.setValue("Ankobra");
    await until(b, "the failure", async () => exists(b, "reach-search-failed"));
    assert.match(await text(b, "reach-search-failed"), /Photon\) is not answering.*Press Enter/s);
    assert.match(await text(b, "reach-search-credit"), /OpenStreetMap contributors \(ODbL\)/);
    await b.keys("Enter");
    const ankobra = '[data-testid="reach-hit"][data-osm="relation/3328713"]';
    await until(b, "the Nominatim list", async () => (await b.$(ankobra)).isExisting());
    assert.match(await text(b, "reach-search-credit"), /Nominatim/);
    assert.deepEqual(searches.slice(before), ["Ankobra", "nominatim:Ankobra"], "Photon once, then Nominatim once");
    await (await b.$(`${ankobra} button`)).click();
    await until(b, "the whole river", async () => (await exists(b, "reach-length")) && /km/.test(await text(b, "reach-length")));
    const route = (await pageDoc(b)).provenance.route;
    assert.equal(route.settings.river_search.source, "osm-nominatim");
    assert.equal(route.settings.osm_id, "relation/3328713");
  });

  it("shows what a long step is doing, with a bar, the time and Cancel, which leaves the study as it was", async () => {
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await b.execute(async (at) => window.__owdPlatform.call("set_reach_endpoints", { endpoints: at }), endpoints(reach.origin));
    await (await tid(b, "start-from-river")).click();
    await (await tid(b, "reach-start")).setValue("-6.0715, 4.966");
    await (await tid(b, "reach-end")).setValue("-6.07, 4.983");
    try {
      slow(300);
      await (await tid(b, "reach-route")).click();
      await until(b, "the route's status", async () => exists(b, "reach-status"));
      assert.match(await text(b, "reach-progress"), /Finding the route/);
      await until(b, "the route", async () => (await exists(b, "reach-length")) && /km/.test(await text(b, "reach-length")));
      // The buttons are enabled again once the status is gone.
      await until(b, "the route's status gone", async () => !(await exists(b, "reach-status")));
      const tile = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "owd-jrc-")), "occurrence_10W_10Nv1_4_2021.tif");
      fs.copyFileSync(path.resolve(here, "../../examples/reach/jrc_occurrence_sassandra_estuary.tif"), tile);
      await pick(b, async () => (await tid(b, "reach-add-tile")).click(), tile);
      await until(b, "the tile given", async () => (await (await tid(b, "reach-jrc-occurrence_10W_10Nv1_4_2021.tif")).getAttribute("data-given")) === "true");
      const before = await pageDoc(b);
      await (await tid(b, "reach-generate")).click();
      await until(b, "a request named in the status line", async () => (await exists(b, "reach-now")) && /\(request \d+\)/.test(await text(b, "reach-now")), 30000);
      assert.match(await text(b, "reach-progress"), /Step 1 of \d+: /);
      assert.match(await text(b, "reach-elapsed"), /^\d+:\d\d$/);
      assert.ok(await exists(b, "reach-bar"));
      await (await tid(b, "reach-cancel")).click();
      await until(b, "the status gone", async () => !(await exists(b, "reach-status")), 30000);
      assert.match(await text(b, "error"), /cancelled/i);
      const after = await pageDoc(b);
      assert.deepEqual(after.provenance, before.provenance, "Cancel leaves the study as it was");
      assert.equal(after.inputs?.left_bank, before.inputs?.left_bank);
    } finally {
      slow(0);
    }
  });

  it("starts a study from a river: route, inputs with badges, a depth, the class table, and Use this vessel", async () => {
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    // The recorded answers stand in for the public sources.
    await b.execute(async (at) => window.__owdPlatform.call("set_reach_endpoints", { endpoints: at }), endpoints(reach.origin));
    await (await tid(b, "start-from-river")).click();
    await (await tid(b, "reach-start")).setValue("-6.0715, 4.966");
    await (await tid(b, "reach-end")).setValue("-6.07, 4.983");
    await (await tid(b, "reach-route")).click();
    await until(b, "the route", async () => (await exists(b, "reach-length")) && /km/.test(await text(b, "reach-length")));
    const km = await text(b, "reach-length");
    assert.equal(await (await tid(b, "mark-route")).getAttribute("data-mark"), "default");
    // The JRC tile has no CORS: it is listed as a link, downloaded, and given back.
    const tile = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "owd-jrc-")), "occurrence_10W_10Nv1_4_2021.tif");
    fs.copyFileSync(path.resolve(here, "../../examples/reach/jrc_occurrence_sassandra_estuary.tif"), tile);
    await pick(b, async () => (await tid(b, "reach-add-tile")).click(), tile);
    await until(b, "the tile given", async () => (await (await tid(b, "reach-jrc-occurrence_10W_10Nv1_4_2021.tif")).getAttribute("data-given")) === "true");
    // The DEM the level falls back on: a GLO-30 window, through the file picker.
    await pick(b, async () => (await tid(b, "reach-load-dem")).click(), path.resolve(here, "../../examples/reach/glo30_sassandra_lower.tif"));
    await until(b, "the DEM", async () => (await (await tid(b, "mark-dem")).getAttribute("data-mark")) === "user");
    await (await tid(b, "reach-generate")).click();
    await until(b, "the banks moved onto Sentinel-2", async () => (await (await tid(b, "reach-input-left_bank")).getText()).includes("sentinel2-l2a"), 120000);
    await until(b, "the water level", async () => (await (await tid(b, "mark-water_level")).getAttribute("data-mark")) === "default", 120000);
    for (const input of ["left_bank", "right_bank", "river_polygon", "centreline"]) {
      const badge = await tid(b, `mark-${input}`);
      assert.equal(await badge.getAttribute("data-mark"), "default", input);
      const shown = await b.execute((el) => el.textContent, badge);
      assert.match(shown, /◇\s*default/, `${input}: a shape and a word, not colour alone (${shown})`);
    }
    assert.match(await text(b, "map-notices"), /OpenStreetMap/);
    assert.match(await text(b, "map-notices"), /Sentinel/);
    // The QA overlay (spec 0013): drawn in the worker and saved as two downloads, named in the notice.
    await (await tid(b, "reach-qa-overlay")).click();
    await until(b, "the QA overlay", async () => (await tid(b, "reach-qa-saved")).isExisting(), 120000);
    assert.match(await text(b, "reach-qa-saved"), /QA overlay\.png, .*QA overlay\.kmz/);
    await (await tid(b, "bed-depth")).setValue("3");
    await (await tid(b, "bed-set")).click();
    await until(b, "the depth", async () => (await (await tid(b, "mark-bed")).getAttribute("data-mark")) === "user");
    await (await tid(b, "fits-run")).click();
    await until(b, "the class table", async () => (await b.$$("[data-testid^=fits-row-]")).length > 3);
    // Spec 0016 T4: the largest vessel, through the shell (in wasm): the series table, the chart, and
    // the per-km strip on the map, as the study keeps them.
    await (await tid(b, "envelope-run")).click();
    await until(b, "the series table", async () => (await b.$$("[data-testid=envelope-row]")).length === 5, 120000);
    const env = (await pageDoc(b)).stages.envelope.results;
    assert.equal(await text(b, "envelope-largest"), env.largest ? env.largest.label : "no size of the series");
    for (const z of env.sizes) {
      const r = await b.$(`[data-testid=envelope-row][data-size="${z.label}"]`);
      assert.equal(await r.getAttribute("data-passes"), String(z.passes), z.label);
      assert.match(await r.getText(), z.passes ? /✓ passes/ : /✕ blocked/, "in words and a shape, not colour alone");
    }
    assert.ok(await (await tid(b, "envelope-chart")).isExisting(), "the chart");
    assert.match(await text(b, "envelope-caveat"), /banks/);
    assert.ok(env.per_km.length > 0, "a reach of a km or more");
    await until(b, "the strip on the map", async () => Number(await (await tid(b, "envelope-panel")).getAttribute("data-strip-drawn")) === env.per_km.length);
    const row = (await b.$$("[data-testid^=fits-row-]"))[0];
    const vessel = (await row.getAttribute("data-testid")).replace("fits-row-", "");
    assert.match(await row.getText(), /✓ fits|✕ does not fit/, "the row says in words and a shape whether it fits");
    await (await tid(b, `use-vessel-${vessel}`)).click();
    await until(b, "the vessel set", async () => (await pageDoc(b)).stages.sizing.settings?.vessel_standard_id === vessel);
    assert.match(await text(b, "notice"), /Vessel set/);
    assert.equal(await (await b.$("select[aria-label='Vessel class']")).getValue(), vessel, "today's flow shows the vessel");
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-start-from-river.png"));
    }

    // Open the saved study again: the panel draws its route again and keeps its inputs and steps (#318).
    const study = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "owd-study-")), "reach-study.owd");
    fs.writeFileSync(study, JSON.stringify(await pageDoc(b)));
    await b.execute(() => {
      window.confirm = () => true; // discard the unsaved design
    });
    await pick(b, async () => (await openButton(b)).click(), study);
    const view = await tid(b, "start-from-river");
    if ((await view.getAttribute("aria-pressed")) !== "true") await view.click();
    await until(b, "the route drawn again", async () => (await exists(b, "reach-length")) && (await text(b, "reach-length")) === km);
    assert.match(await (await tid(b, "reach-input-left_bank")).getText(), /sentinel2-l2a/);
    assert.ok(await exists(b, "reach-load-dem"), "step 3 is shown after Open");
  });

  it("names no server of ours: every host in the web bundle is a public source", () => {
    const hosts = new Set();
    const walk = (dir) => {
      for (const f of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, f.name);
        if (f.isDirectory()) walk(p);
        else if (/\.(js|wasm|html)$/.test(f.name)) for (const m of fs.readFileSync(p, "latin1").matchAll(/https?:\/\/([a-z0-9.-]+\.[a-z]{2,})/gi)) hosts.add(m[1].toLowerCase());
      }
    };
    walk(DIST);
    // Hosts that are not data sources: the basemaps (EOX, and Esri for a user who enters their own key), and standards and docs that libraries name in
    // comments and messages. None is a server of ours; a new host fails here until it is looked at.
    // tribby.com and scripts.sil.org are in the name table of the drawing font (Barlow, SIL OFL), which
    // the PDF writer embeds in the wasm: its designer's site and its licence's. Text, never fetched.
    // www.opengis.net and www.google.com are the KML 2.2 namespace and its `gx` extension, written into
    // the QA overlay's KMZ (spec 0013): names of XML vocabularies, never fetched.
    const KNOWN = ["tiles.maps.eox.at", "cloudless.eox.at", "ibasemaps-api.arcgis.com", "www.w3.org", "svelte.dev", "maplibre.org", "maplibre.invalid", "github.com", "wiki.openstreetmap.org", "json-schema.org", "tribby.com", "scripts.sil.org", "www.opengis.net", "www.google.com"];
    const allowed = (h) => [...PUBLIC_HOSTS, ...KNOWN].some((k) => h === k || h.startsWith(k)) || reportEngineHost(h) || catalogueHost(h);
    const unknown = [...hosts].filter((h) => !allowed(h));
    assert.deepEqual(unknown, [], `hosts in the web bundle that are neither a public source nor known: ${unknown.join(" ")}`);
    for (const h of ["overpass-api.de", "earth-search.aws.element84.com", "zenodo.org", "soto.podaac.earthdatacloud.nasa.gov"]) assert.ok(hosts.has(h), `the bundle reads ${h}`);
  });

  it("opens a design and its DEM through the file picker, and a full run equals owd run", async () => {
    const FILE = "river-rolling-circle.owd";
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), path.join(FIXTURES, FILE));
    await until(b, "the file name", async () => (await text(b, "file-label")).includes(FILE));
    // The document names its DEM; the browser does not have it yet.
    await until(b, "the missing-DEM warning", async () => (await exists(b, "warning")) && (await text(b, "warning")).toLowerCase().includes("dem"));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    await until(b, "the warning to go", async () => !(await exists(b, "warning")));
    assert.equal(await stageStatus(b), "sizing:fresh alignment:stale optimise:stale volumes:stale");
    await (await b.$("button=Run stale")).click();
    await until(b, "every stage fresh", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:fresh volumes:fresh", 240000);
    sameAsNative(await pageDoc(b), nativeDoc(FILE), FILE);
    assert.equal(await exists(b, "error"), false);
  });

  it("names the line on screen with its stage, says a missing DEM plainly, clears the DEM banner on load, and sets the cost per metre of line", async () => {
    const FILE = "river-rolling-circle.owd";
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), path.join(FIXTURES, FILE));
    await until(b, "the file name", async () => (await text(b, "file-label")).includes(FILE));
    // The DEM is not loaded: the banner says so, and the volumes note says so, not "stale".
    await until(b, "the missing-DEM banner", async () => (await exists(b, "warning")) && (await text(b, "warning")).toLowerCase().includes("dem"));
    await until(b, "the volumes note", async () => await exists(b, "volumes-note"));
    assert.equal(await text(b, "volumes-note"), "Volumes cannot be checked: the elevation model is not loaded. Load the DEM.");
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    await until(b, "the banner to go", async () => !(await exists(b, "warning")));
    // A real stale stage keeps the stale wording.
    await until(b, "the stale note", async () => (await exists(b, "volumes-note")) && (await text(b, "volumes-note")).startsWith("Volumes are stale"));
    // Loading only a DEM leaves the title unsaved.
    assert.match(await text(b, "file-label"), /\(unsaved\)/);
    await (await b.$("button=Run stale")).click();
    await until(b, "every stage fresh", async () => (await stageStatus(b)) === "sizing:fresh alignment:fresh optimise:fresh volumes:fresh", 240000);
    // Optimise is fresh and holds the lattice's line: the screen draws it, and the label says so.
    await until(b, "the optimise line named", async () => /^Line on screen: .+ \(from Optimise\)$/.test(await text(b, "line-on-screen")));
    const onScreen = await text(b, "line-on-screen");
    const tableName = await b.execute(() => document.querySelector("[data-testid=option-row] td.name").textContent.trim());
    assert.equal(onScreen, `Line on screen: ${tableName} (from Optimise)`, "the label names the table's line");
    // Set the cost per metre: the design holds it, the stored value shows, optimise goes stale, the alignment's line is on screen.
    assert.equal(await text(b, "length-weight-stored"), "Stored: 0");
    await (await tid(b, "length-weight")).setValue("2.5");
    await (await tid(b, "set-length-weight")).click();
    await until(b, "the setting stored", async () => (await pageDoc(b)).stages.optimise.settings?.length_weight === 2.5);
    assert.equal(await text(b, "length-weight-stored"), "Stored: 2.5");
    await until(b, "optimise stale", async () => (await stageStatus(b)).includes("optimise:stale"));
    await until(b, "the alignment line named", async () => /\(from Alignment\)$/.test(await text(b, "line-on-screen")));
    assert.match(await text(b, "length-weight-help"), /A high value shortens the line and raises the cut\./);
    assert.equal(await exists(b, "error"), false);
  });

  it("opens an older-format design with one quiet information row and the DEM warning; Details lists the notes, Dismiss removes a row, another design clears them (UI-3)", async () => {
    // A format 0.4 file whose stages were all up to date (a synthetic river; its DEM is not loaded).
    const OLD = path.resolve(here, "../../crates/owd-format/tests/data/format-0.4-lattice-only.owd");
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), OLD);
    await until(b, "the file name", async () => (await text(b, "file-label")).includes("format-0.4-lattice-only.owd"));
    // One information row, not a warning, and one warning: the DEM, the only thing to act on.
    await until(b, "the information row", async () => await exists(b, "open-info"));
    assert.equal((await b.$$('[data-testid="open-info"]')).length, 1);
    assert.match(await text(b, "open-info"), /Opened from an older format\. Nothing was lost\./);
    assert.doesNotMatch(await (await tid(b, "open-info")).getAttribute("class"), /warn/);
    assert.equal((await b.$$('[data-testid="warning"]')).length, 1, "one warning row");
    assert.match(await text(b, "warning"), /The DEM file river-dem\.tif was not found/);
    // The notes are folded away until Details is pressed: then the four of them show.
    assert.equal(await exists(b, "open-info-note"), false);
    await (await tid(b, "open-info-details")).click();
    await until(b, "the details", async () => (await b.$$('[data-testid="open-info-note"]')).length === 4);
    const notes = [];
    for (const n of await b.$$('[data-testid="open-info-note"]')) notes.push(await n.getText());
    assert.match(notes[0], /^This design was saved in an older format/);
    assert.match(notes[1], /^Its design line is lattice/);
    assert.match(notes[2], /kept fresh/);
    assert.match(notes[3], /^The mouth is the first end/);
    await (await tid(b, "open-info-details")).click();
    await until(b, "the details to fold", async () => !(await exists(b, "open-info-note")));
    // Dismiss removes the information row and leaves the warning; the warning has its own Dismiss.
    await (await tid(b, "open-info-dismiss")).click();
    await until(b, "the information row to go", async () => !(await exists(b, "open-info")));
    assert.equal(await exists(b, "warning"), true);
    await (await tid(b, "warning-dismiss")).click();
    await until(b, "the warning to go", async () => !(await exists(b, "warning")));
    // Open the same file again: both rows are back. Then New: none is left.
    await pick(b, async () => (await openButton(b)).click(), OLD);
    await until(b, "the rows again", async () => (await exists(b, "open-info")) && (await exists(b, "warning")));
    await (await b.$("button=New")).click();
    await until(b, "a new design", async () => !(await text(b, "file-label")).includes("format-0.4"));
    assert.equal(await exists(b, "open-info"), false);
    assert.equal(await exists(b, "warning"), false);
  });

  it("loads a Rule Set file through Load Rule Set file, sizes with it, and shows a bad file's refusal with the list unchanged (#499)", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    const options = () => b.execute(() => [...document.querySelectorAll("[data-testid=rule-set-0] option")].map((o) => [o.value, o.textContent]));
    const before = await options();
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-rule-set-"));
    const builtIn = fs.readFileSync(path.resolve(here, "../../crates/owd-core/rules/pianc-wg141/0.3.0.json"), "utf8");
    const mine = path.join(dir, "wider-bottom.json");
    fs.writeFileSync(mine, builtIn.replace('"k": 3.2,', '"k": 3.6,').replace('"version": "0.3.0"', '"version": "0.9.0"'));
    await pick(b, async () => (await tid(b, "load-rule-set")).click(), mine);
    await until(b, "the file's set in the list", async () => (await options()).some(([v]) => v === "pianc-wg141@0.9.0"));
    assert.match((await options()).find(([v]) => v === "pianc-wg141@0.9.0")[1], /from a file/);
    await b.execute(() => {
      const s = document.querySelector("[data-testid=rule-set-0]");
      s.value = "pianc-wg141@0.9.0";
      s.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "sizing with the file's set", async () => (await text(b, "shape-rule-sets")).includes("pianc-wg141@0.9.0"));
    // A bad file: the core's refusal, and the list as it was.
    const bad = path.join(dir, "not-a-rule-set.json");
    fs.writeFileSync(bad, "{ not a rule set");
    const now = await options();
    await pick(b, async () => (await tid(b, "load-rule-set")).click(), bad);
    await until(b, "the refusal", async () => exists(b, "rule-set-load-error"));
    assert.match(await text(b, "rule-set-load-error"), /the Rule Set file is refused/);
    assert.deepEqual(await options(), now);
    assert.equal(before.length + 1, now.length);
  });

  it("sets the rolling circle's water mode: Ignore the water equals owd align --use-water false, Default equals owd align (#494)", async () => {
    const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.owd");
    /** The document `owd align <doc> --method rolling-circle` leaves for the example, with `flags`. */
    const aligned = (flags) => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-use-water-"));
      const doc = path.join(dir, "synthetic-river.owd");
      fs.copyFileSync(EXAMPLE, doc);
      try {
        execFileSync(CLI, ["align", doc, "--method", "rolling-circle", ...flags], { encoding: "utf8", maxBuffer: 1 << 28 });
      } catch (e) {
        if (e.status !== 1) throw e; // exit 1: a check failed; the document is still written
      }
      return JSON.parse(fs.readFileSync(doc, "utf8"));
    };
    const stored = async () => (await pageDoc(b)).stages.alignment.settings?.rolling_circle?.use_water;
    const pickWater = (value) =>
      b.execute((v) => {
        const s = document.querySelector("[data-testid=use-water]");
        s.value = v;
        s.dispatchEvent(new Event("change", { bubbles: true }));
      }, value);
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), EXAMPLE);
    await until(b, "the example", async () => (await text(b, "file-label")).includes("synthetic-river.owd"));
    assert.equal(await (await tid(b, "use-water")).getValue(), "default");
    // The binding: the three states are false, true and unset in the design's settings.
    await pickWater("true");
    await until(b, "use_water true", async () => (await stored()) === true);
    await pickWater("default");
    await until(b, "use_water unset", async () => (await stored()) === null);
    await pickWater("false");
    await until(b, "use_water false", async () => (await stored()) === false);
    // Ignore the water, run: the document `owd align --use-water false` makes.
    await (await tid(b, "run-alignment")).click();
    await until(b, "alignment fresh", async () => (await text(b, "status-alignment")) === "fresh", 120000);
    sameAsNative(await pageDoc(b), aligned(["--use-water", "false"]), "Ignore the water");
    assert.match(await text(b, "use-water-last"), /Ignore the water/);
    // Default: the setting changes, the stage goes stale; run: the line of `owd align` with no flag.
    await pickWater("default");
    await until(b, "alignment stale", async () => (await text(b, "status-alignment")) === "stale");
    await (await tid(b, "run-alignment")).click();
    await until(b, "alignment fresh again", async () => (await text(b, "status-alignment")) === "fresh", 120000);
    const page = await pageDoc(b);
    // `owd align` with no flag stores the setting unset (null), as Default does.
    assert.equal(page.stages.alignment.settings.rolling_circle.use_water, null);
    sameAsNative(page, aligned([]), "Default");
    // The design line records the value the run used: the example has a river polygon, so on.
    assert.equal(page.stages.alignment.results.settings.use_water, true);
    assert.match(await text(b, "use-water-last"), /Keep to the water/);
  });

  it("states the river mouth: downstream with nothing to tell the ends apart is refused, and a point gives the document owd import --mouth gives (spec 0017)", async () => {
    const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.owd");
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-mouth-"));
    const doc = path.join(dir, "synthetic-river.owd");
    fs.copyFileSync(EXAMPLE, doc);
    // The far end of the left bank, in longitude and latitude, from the command line's own export.
    const banks = path.join(dir, "banks.geojson");
    execFileSync(CLI, ["export", doc, "--banks", banks, "--wgs84"], { encoding: "utf8" });
    const left = JSON.parse(fs.readFileSync(banks, "utf8")).features.find((f) => f.properties.role === "left_bank");
    const far = left.geometry.coordinates[left.geometry.coordinates.length - 1];
    const mouth = `${far[0]},${far[1]}`;
    let refused = "";
    try {
      execFileSync(CLI, ["import", doc, "--mouth", "downstream"], { encoding: "utf8", stdio: "pipe" });
    } catch (e) {
      refused = String(e.stderr);
    }
    assert.match(refused, /RIVER_MOUTH_UNKNOWN/);
    execFileSync(CLI, ["import", doc, "--mouth", mouth], { encoding: "utf8" });
    const native = JSON.parse(fs.readFileSync(doc, "utf8"));
    assert.equal(native.inputs.mouth.stated_as, "point");

    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), EXAMPLE);
    await until(b, "the example", async () => (await text(b, "file-label")).includes("synthetic-river.owd"));
    const set = async (value) => {
      const field = await tid(b, "river-mouth");
      await field.setValue(value);
      await (await tid(b, "set-river-mouth")).click();
    };
    await set("downstream");
    await until(b, "the refusal", async () => (await exists(b, "error")) && /RIVER_MOUTH_UNKNOWN/.test(await text(b, "error")));
    await set(mouth);
    await until(b, "the mouth stored", async () => (await pageDoc(b)).inputs.mouth?.stated_as === "point");
    const page = await pageDoc(b);
    assert.deepEqual(page.inputs, native.inputs, "the same inputs, turned to run from the mouth");
    sameAsNative(page, native, "the mouth");
  });

  it("sets the run's objective, limits and target level: an unknown measure is refused, and the inputs equal owd run-inputs (spec 0017 ticket 11)", async () => {
    const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.owd");
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-run-inputs-"));
    const doc = path.join(dir, "synthetic-river.owd");
    fs.copyFileSync(EXAMPLE, doc);
    execFileSync(CLI, ["run-inputs", doc, "--objective", "minimise:total_cut", "--limit", "line_length>=500", "--limit", "cost<=1000000 USD", "--target-level", "pre-feasibility"], { encoding: "utf8" });
    const native = JSON.parse(fs.readFileSync(doc, "utf8"));
    assert.equal(native.inputs.run.target_level, "pre_feasibility");

    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), EXAMPLE);
    await until(b, "the example", async () => (await text(b, "file-label")).includes("synthetic-river.owd"));
    const set = async (objective, limits, level) => {
      await (await tid(b, "run-objective")).setValue(objective);
      await (await tid(b, "run-limits")).setValue(limits);
      await (await tid(b, "run-target-level")).selectByAttribute("value", level);
      await (await tid(b, "set-run-inputs")).click();
    };
    await set("minimise:beauty", "", "concept");
    await until(b, "the refusal", async () => (await exists(b, "error")) && /RUN_MEASURE_UNKNOWN/.test(await text(b, "error")));
    assert.equal((await pageDoc(b)).inputs.run, undefined, "a refusal changes nothing");
    await set("minimise:total_cut", "line_length>=500\ncost<=1000000 USD", "pre-feasibility");
    await until(b, "the run inputs stored", async () => (await pageDoc(b)).inputs.run?.target_level === "pre_feasibility");
    const page = await pageDoc(b);
    assert.deepEqual(page.inputs, native.inputs, "the same inputs as owd run-inputs");
    assert.deepEqual(page.stages, native.stages, "no stage moved");
  });

  it("Cancel leaves the design exactly as it was", async () => {
    const FILE = "river-rolling-circle.owd";
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), path.join(FIXTURES, FILE));
    await until(b, "the file name", async () => (await text(b, "file-label")).includes(FILE));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    const docBefore = await pageDoc(b);
    const statusBefore = await stageStatus(b);
    // Alignment is quick; the pool search after it is not. Start the whole chain, cancel in the search.
    await (await tid(b, "run-alignment")).click();
    await until(b, "alignment fresh", async () => (await stageStatus(b)).includes("alignment:fresh"));
    const docBeforeOptimise = await pageDoc(b);
    await (await tid(b, "run-optimise")).click();
    await (await tid(b, "cancel-optimise")).waitForExist({ timeout: 10000 });
    await (await tid(b, "cancel-optimise")).click();
    await until(b, "the cancelled note", async () => (await exists(b, "notice")) && (await text(b, "notice")).includes("cancelled"));
    // The worker was restarted from the design as it was before the optimise run (after alignment).
    const status = await stageStatus(b);
    assert.ok(status.includes("alignment:fresh") && status.includes("optimise:stale"), `the optimise run left nothing behind: ${status}`);
    assert.notEqual(status, statusBefore, "alignment did run");
    assert.deepEqual(await pageDoc(b), docBeforeOptimise, "the document is exactly as it was before the cancelled run");
    assert.equal(await exists(b, "error"), false);
    // And the app still works: an edit reaches the restarted worker and marks the stages stale.
    await (await b.$("input[aria-label=Draught]")).setValue("2.7");
    await until(b, "the edit to land", async () => (await stageStatus(b)).includes("alignment:stale"));
    assert.equal(await exists(b, "error"), false);
    assert.ok(docBefore.stages.optimise.results !== undefined, "the design was opened with every stage run");
  });

  it("Save as asks once: a second press or Enter while it runs does not ask again, and the buttons say what they do", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    // No save dialog in this browser: the name prompt and the download are the fallback, and the buttons say so.
    // (The page looks again after each file action, so opening the example makes it notice.)
    await b.execute(() => {
      Object.defineProperty(window, "showSaveFilePicker", { value: undefined, configurable: true });
      window.__prompts = 0;
      window.prompt = () => {
        window.__prompts += 1;
        return "again.owd";
      };
    });
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.match(await (await tid(b, "save-as")).getAttribute("title"), /downloads/i);
    assert.match(await (await tid(b, "save")).getAttribute("title"), /downloads/i);
    // Two presses in one breath: the second finds the first still running.
    await b.execute(() => {
      const button = document.querySelector("[data-testid=save-as]");
      button.click();
      button.click();
    });
    await until(b, "the file name", async () => (await text(b, "file-label")).includes("again.owd"));
    // Enter after the press: the button must not hold focus to take it.
    await b.keys("Enter");
    await new Promise((r) => setTimeout(r, 800));
    assert.equal(await b.execute(() => window.__prompts), 1, "the prompt opened once");
    assert.match(await text(b, "notice"), /Downloaded again\.owd/);
  });

  it("Save writes through the browser's save dialog when there is one, and Save again writes to the same file", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await b.execute(() => {
      window.__writes = [];
      window.__pickers = 0;
      window.prompt = () => {
        throw new Error("no prompt expected");
      };
      window.showSaveFilePicker = async (o) => {
        window.__pickers += 1;
        return {
          name: o.suggestedName,
          createWritable: async () => {
            let t = "";
            return { write: async (x) => (t += x), close: async () => window.__writes.push(t) };
          },
        };
      };
    });
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await (await tid(b, "save")).click();
    await until(b, "one write", async () => (await b.execute(() => window.__writes.length)) === 1);
    await (await tid(b, "save")).click();
    await until(b, "two writes", async () => (await b.execute(() => window.__writes.length)) === 2);
    assert.equal(await b.execute(() => window.__pickers), 1, "the dialog opened once; the second Save reused the file");
    const doc = JSON.parse(await b.execute(() => window.__writes[1]));
    assert.ok(doc.inputs, "the written file is the design");
    assert.match(await text(b, "notice"), /Saved river-example\.owd/);
  });

  it("says one true thing everywhere: design line heading, line on screen, bank words, DEM name, mouth with the river, fall and its sign, plain refusal", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the table", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    // 5: the table is headed with the design line, not Options.
    assert.equal(await text(b, "tab-options"), "Design line");
    const head = await (await b.$("[data-testid=option-table] thead")).getText();
    assert.match(head, /Design line/);
    assert.doesNotMatch(head, /Option/);
    // 4: the line on screen and what the next run uses are both said.
    assert.match(await text(b, "line-on-screen"), /Line on screen: \S/);
    assert.match(await text(b, "line-next-run"), /Next run uses: Rolling circle/);
    // 6: a river polygon stands in for the banks.
    assert.equal(await text(b, "data-left_bank"), "not needed (river polygon loaded)");
    // 8: the DEM row shows the file name, the full path in its title.
    assert.equal(await text(b, "data-dem"), "river-dem.tif");
    assert.ok((await (await tid(b, "data-dem")).getAttribute("title")).endsWith("river-dem.tif"));
    // 9: the mouth field sits with the river rows, above the DEM row.
    const order = await b.execute(() => {
      const pos = (id) => document.querySelector(`[data-testid="${id}"]`).getBoundingClientRect().top;
      return [pos("data-centreline"), pos("river-mouth"), pos("data-dem")];
    });
    assert.ok(order[0] < order[1] && order[1] < order[2], `mouth is between the river rows and the DEM: ${order}`);
    // 7: a long fall is shown rounded, kept exact, with a line on the sign.
    await (await tid(b, "water-level")).setValue("10");
    await (await tid(b, "water-fall")).setValue("-0.0004925721857999721");
    await (await tid(b, "set-water")).click();
    await until(b, "the fall to show rounded", async () => (await (await tid(b, "water-fall")).getValue()) === "-0.000493");
    const pts = (await pageDoc(b)).inputs.water_level.points;
    const stored = (pts[0][1] - pts[1][1]) / (pts[1][0] - pts[0][0]);
    assert.ok(Math.abs(stored + 0.0004925721858) < 1e-9, `the stored fall stays exact: ${stored}`);
    assert.match(await text(b, "water-fall-help"), /Negative: the water level rises|Negative: it rises/);
    // 3: a stale alignment gives one true reason in the section panel.
    await (await b.$("input[aria-label=Draught]")).setValue("2.6");
    await until(b, "alignment stale", async () => (await stageStatus(b)).includes("alignment:stale"));
    await until(b, "the section note", async () => (await exists(b, "section-note")) && (await text(b, "section-note")).includes("No sections yet"));
    assert.equal(await text(b, "section-note"), "No sections yet: the alignment is out of date. Run alignment, then volumes.");
  });

  it("the Imagery setting: custom tiles change the attribution and are remembered; a failing Esri request says so and falls back; the key stays out of the design", async () => {
    // Esri is never called in a test: its requests are stubbed. Headless Chrome here has no WebGL, so no tile is asked for at all.
    const KEY = "FAKE-KEY-not-real-7f3a9c1d";
    {
      const b = await start();
      await b.url(`${origin}/owd/app`);
      await loaded(b);
      assert.match(await text(b, "attribution"), /EOxCloudless/, "the open default is shown at first");
      assert.equal(await (await tid(b, "attribution")).getAttribute("data-source"), "open");
      // Esri requests never leave the machine in a test: refuse them in the page.
      await b.execute(() => {
        window.__esriCalls = 0;
        const real = window.fetch;
        window.fetch = (u, ...rest) => {
          if (String(u).includes("arcgis.com")) {
            window.__esriCalls += 1;
            return Promise.reject(new TypeError("blocked in the test"));
          }
          return real(u, ...rest);
        };
        window.showSaveFilePicker = async () => ({
          name: "x.owd",
          createWritable: async () => {
            let t = "";
            return { write: async (x) => (t += x), close: async () => (window.__saved = t) };
          },
        });
      });
      // Custom tiles.
      await (await tid(b, "imagery-toggle")).click();
      await (await tid(b, "imagery-select")).selectByAttribute("value", "custom");
      const apply = await tid(b, "imagery-custom-apply");
      assert.equal(await apply.isEnabled(), false, "custom tiles need an address and an attribution");
      await (await tid(b, "imagery-url")).setValue(`http://127.0.0.1:9/{z}/{x}/{y}.png`);
      await (await tid(b, "imagery-attribution")).setValue("Tiles by the Test Tile Server");
      await apply.click();
      await until(b, "the custom attribution", async () => (await text(b, "attribution")) === "Tiles by the Test Tile Server");
      assert.equal(await (await tid(b, "attribution")).getAttribute("data-source"), "custom");
      // Remembered on this machine: a reload shows it again.
      await b.refresh();
      await loaded(b);
      await until(b, "the custom attribution after a reload", async () => (await text(b, "attribution")) === "Tiles by the Test Tile Server");
      // Esri: a failing request shows one line and the open default is back.
      await b.execute(() => {
        window.showSaveFilePicker = async () => ({
          name: "x.owd",
          createWritable: async () => {
            let t = "";
            return { write: async (x) => (t += x), close: async () => (window.__saved = t) };
          },
        });
        const real = window.fetch;
        window.fetch = (u, ...rest) => {
          if (String(u).includes("arcgis.com")) {
            window.__esriCalls = (window.__esriCalls ?? 0) + 1;
            return Promise.reject(new TypeError("blocked in the test"));
          }
          return real(u, ...rest);
        };
      });
      await (await tid(b, "imagery-toggle")).click();
      await (await tid(b, "imagery-select")).selectByAttribute("value", "esri");
      await (await tid(b, "imagery-key")).setValue(KEY);
      await (await tid(b, "imagery-key-set")).click();
      await until(b, "the Esri failure line", async () => await exists(b, "imagery-note"));
      assert.equal(await text(b, "imagery-note"), "Esri imagery could not load: check your key (it needs the basemap privilege) or choose another imagery source\nDismiss");
      assert.match(await text(b, "attribution"), /EOxCloudless/, "the open default is back, with its own attribution");
      assert.equal(await (await tid(b, "attribution")).getAttribute("data-source"), "open");
      assert.ok((await b.execute(() => window.__esriCalls)) >= 1, "the stub saw the Esri request");
      // The key is masked in the field, with a Clear button.
      const masked = await text(b, "imagery-key-masked");
      assert.ok(masked.endsWith("9c1d"), "the last four characters show");
      assert.ok(!masked.includes("7f3a"), "and no more");
      assert.ok(!masked.includes("FAKE"));
      assert.equal(await exists(b, "imagery-key-clear"), true);
      // The key is kept in this browser, and nowhere in the design, the page title or the page text.
      assert.ok(await b.execute((k) => localStorage.getItem("owd-imagery").includes(k), KEY), "the key is stored on this machine");
      await (await tid(b, "open-example")).click();
      await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
      await (await tid(b, "save")).click();
      await until(b, "the saved design", async () => (await b.execute(() => window.__saved)) !== undefined && (await b.execute(() => window.__saved)) !== null);
      const saved = await b.execute(() => window.__saved);
      assert.ok(saved.length > 1000 && JSON.parse(saved).inputs, "the saved text is the design");
      assert.ok(!saved.includes(KEY), "the key is not in the saved design");
      assert.ok(!saved.includes("owd-imagery"));
      assert.ok(!(await b.getTitle()).includes(KEY));
      const page = await b.execute(() => document.body.innerText + document.documentElement.outerHTML.replace(/<input[^>]*>/g, ""));
      assert.ok(!page.includes(KEY), "the key is not shown anywhere on the page");
      // Clear removes it.
      await (await tid(b, "imagery-key-clear")).click();
      assert.equal(await b.execute(() => (localStorage.getItem("owd-imagery") ?? "").includes("7f3a9c1d")), false);
    }
  });

  it("the Layers list (MAP-1): seven ticks, a layer the design lacks is greyed, a tick is remembered after a reload and stays out of the design, and the legend follows", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const IDS = ["river-polygon", "centreline", "bank-lines", "design-line", "constraint-stretches", "station-marks", "imagery"];
    const box = (id) => b.execute((i) => {
      const c = document.querySelector(`[data-testid="layer-${i}"]`);
      return { type: c.type, checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
    }, id);
    const off = async () => (await tid(b, "plan-map")).getAttribute("data-layers-off");
    // A real button opens the list; it says whether it is open.
    const button = await tid(b, "layers-button");
    assert.equal(await button.getTagName(), "button");
    assert.equal(await button.getAttribute("aria-label"), "Layers");
    assert.equal(await button.getAttribute("aria-expanded"), "false");
    assert.equal(await exists(b, "layers-list"), false);
    await button.click();
    assert.equal(await button.getAttribute("aria-expanded"), "true");
    assert.equal((await b.$$("[data-testid=layers-list] input[type=checkbox]")).length, IDS.length);
    // No design yet: only the imagery is there. The rest are greyed and say so; every tick is on by default.
    for (const id of IDS) {
      const c = await box(id);
      assert.equal(c.type, "checkbox");
      assert.equal(c.checked, true, `${id} is on by default`);
      assert.equal(c.disabled, id !== "imagery", id);
      assert.equal(/none loaded/.test(c.row), id !== "imagery", c.row);
    }
    assert.equal((await box("river-polygon")).row, "River polygon none loaded");
    assert.equal(await exists(b, "legend-river-polygon"), false, "the legend lists nothing the design lacks");
    // The example has a river polygon and a centreline and no bank lines.
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the river polygon in the list", async () => !(await box("river-polygon")).disabled);
    const g = await b.execute(async () => {
      const m = await window.__owdPlatform.call("map_geometry");
      return { polygon: m.river_polygon.coordinates[0], centre: !!m.centreline, banks: m.banks.length, options: m.options.length };
    });
    const stored = (await pageDoc(b)).inputs.river_polygon.coordinates[0];
    assert.ok(stored.length > 100, "the example's polygon has many positions");
    assert.equal(g.polygon.length, stored.length, "every position of the polygon reaches the map");
    for (const p of g.polygon) assert.ok(Math.abs(p[0]) <= 180 && Math.abs(p[1]) <= 90, `the polygon reaches the page in lon/lat: ${p}`);
    assert.equal((await box("river-polygon")).row, "River polygon");
    assert.equal((await box("centreline")).disabled, !g.centre);
    assert.equal(g.banks, 0, "the example has no bank lines");
    assert.equal((await box("bank-lines")).disabled, true);
    assert.equal((await box("bank-lines")).row, "Bank lines none loaded");
    assert.ok(g.options > 0, "the example has a design line");
    assert.equal((await box("design-line")).disabled, false);
    // The legend: what is there and ticked on, and nothing else.
    await until(b, "the polygon in the legend", () => exists(b, "legend-river-polygon"));
    assert.equal(await exists(b, "legend-centreline"), true);
    assert.equal(await exists(b, "legend-bank-lines"), false);
    assert.equal((await b.$$("[data-testid=legend-option]")).length, g.options);
    assert.equal(await off(), "");
    // Untick the river polygon and the design line: gone from the map and the legend at once.
    await (await tid(b, "layer-river-polygon")).click();
    await (await tid(b, "layer-design-line")).click();
    await until(b, "the legend without the polygon", async () => !(await exists(b, "legend-river-polygon")));
    assert.equal((await b.$$("[data-testid=legend-option]")).length, 0);
    assert.equal(await exists(b, "legend-centreline"), true, "the other layers stay");
    assert.equal(await off(), "river-polygon design-line");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["river-polygon","design-line"]');
    // Never in the design file.
    assert.doesNotMatch(JSON.stringify(await pageDoc(b)), /owd-map-layers|"design-line"/);
    // Remembered on this machine: a reload shows the same ticks.
    await b.refresh();
    await loaded(b);
    await until(b, "the example after a reload", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.equal(await off(), "river-polygon design-line");
    await (await tid(b, "layers-button")).click();
    assert.equal((await box("river-polygon")).checked, false);
    assert.equal((await box("design-line")).checked, false);
    assert.equal((await box("centreline")).checked, true);
    await until(b, "the centreline in the legend", () => exists(b, "legend-centreline"));
    assert.equal(await exists(b, "legend-river-polygon"), false);
    // Tick them again: back in the legend, and nothing is left in the store when all are on.
    await (await tid(b, "layer-river-polygon")).click();
    await until(b, "the polygon in the legend again", () => exists(b, "legend-river-polygon"));
    await (await tid(b, "layer-design-line")).click();
    assert.equal(await off(), "");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), null);
    // Escape closes the list.
    await b.keys("Escape");
    await until(b, "the list to close", async () => !(await exists(b, "layers-list")));
  });

  it("moves the cross section with the station slider: asked once it settles, working meanwhile, a refusal said", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the first cross section", async () => (await exists(b, "cs-chainage")), 120000);
    const first = await text(b, "cs-chainage");
    // Count the cross sections asked for, and slow each one down so the panel's working state shows.
    await b.execute(() => {
      const p = window.__owdPlatform;
      const call = p.call.bind(p);
      window.__crossAsked = 0;
      window.__crossRefuse = null;
      p.call = (command, args) => {
        if (command !== "cross_section") return call(command, args);
        window.__crossAsked += 1;
        if (window.__crossRefuse) return Promise.reject(window.__crossRefuse);
        return new Promise((r) => setTimeout(r, 800)).then(() => call(command, args));
      };
    });
    // Drag the slider through several stations in one go: one request, when it settles.
    const slide = (to) =>
      b.execute((to) => {
        const el = document.querySelector("[data-testid=station-slider]");
        for (let v = Number(el.value) + 1; v <= to; v++) {
          el.value = String(v);
          el.dispatchEvent(new Event("input", { bubbles: true }));
        }
      }, to);
    await slide(30);
    await until(b, "the working state", async () => await exists(b, "cross-busy"), 5000);
    await until(b, "the section to move", async () => !(await exists(b, "cross-busy")) && (await exists(b, "cs-chainage")) && (await text(b, "cs-chainage")) !== first);
    assert.equal(await b.execute(() => window.__crossAsked), 1, "one request for the whole slide");
    assert.notEqual(await text(b, "cs-chainage"), first);
    // The engine refuses: the panel says why, instead of keeping the old section in silence.
    await b.execute(() => {
      window.__crossRefuse = "the volumes stage keeps the section profile only at the stations a cross-section sheet draws by default: a section at another station needs the DEM (the elevation model) to cut it: load the DEM";
    });
    await slide(35);
    await until(b, "the refusal", async () => (await exists(b, "cross-refusal")) && /load the DEM/.test(await text(b, "cross-refusal")));
    assert.equal(await exists(b, "cs-chainage"), false, "no section drawn under a refusal");
    assert.equal(await exists(b, "cross-busy"), false);
  });

  it("still loads when browser storage is blocked", async () => {
    // Blocking cookies and site data makes `localStorage` throw.
    const b = await start({ prefs: { "profile.default_content_setting_values.cookies": 2 } });
    await b.url(`${origin}/owd/app`);
    const blocked = await b.execute(() => {
      try {
        window.localStorage.getItem("x");
        return false;
      } catch {
        return true;
      }
    });
    assert.ok(blocked, "storage is blocked in this browser");
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.equal(await exists(b, "error"), false);
  });
});
