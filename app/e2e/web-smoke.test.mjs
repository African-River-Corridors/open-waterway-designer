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
    // The stretches are in a drawer, closed until asked for.
    assert.equal(await exists(b, "stretches-panel"), false, "the stretches drawer starts closed");
    assert.equal(await text(b, "stretches-toggle"), `Constraint stretches (${sc.constraint_stretches.length})`);
    await (await tid(b, "stretches-toggle")).click();
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
    const tableName = await b.execute(() => document.querySelector("[data-testid=option-row] [data-col=name]").textContent.trim());
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
    // The cost per m² of land, the box under it: the design holds it beside the length cost, and the help is two plain sentences.
    assert.equal(await text(b, "land-weight-stored"), "Stored: 0");
    await (await tid(b, "land-weight")).setValue("3.5");
    await (await tid(b, "set-land-weight")).click();
    await until(b, "the land weight stored", async () => (await pageDoc(b)).stages.optimise.settings?.land_weight === 3.5);
    assert.equal(await text(b, "land-weight-stored"), "Stored: 3.5");
    assert.equal((await pageDoc(b)).stages.optimise.settings.length_weight, 2.5, "the length cost is kept");
    assert.equal(
      await text(b, "land-weight-help"),
      "The pool search pays this for every m² of channel outside the river polygon, even where nothing is cut, so the line keeps to the river. A value equal to the channel depth prices land like a full-depth cut.",
    );
    // The corridor tick box: off and not in the design; ticked, the design holds it; unticked, it goes.
    assert.equal(await (await tid(b, "corridor-limit")).isSelected(), false);
    assert.equal((await pageDoc(b)).stages.optimise.settings.corridor_limit, undefined);
    await (await tid(b, "corridor-limit")).click();
    await until(b, "the corridor limit stored", async () => (await pageDoc(b)).stages.optimise.settings?.corridor_limit === true);
    await until(b, "the box ticked", async () => await (await tid(b, "corridor-limit")).isSelected());
    assert.equal(
      await text(b, "corridor-limit-help"),
      "The pool search may not go further from the baseline than the corridor's half-width. A pool with no line inside the corridor keeps the baseline and says so.",
    );
    await (await tid(b, "corridor-limit")).click();
    await until(b, "the corridor limit gone", async () => (await pageDoc(b)).stages.optimise.settings?.corridor_limit === undefined);
    assert.equal((await pageDoc(b)).stages.optimise.settings.land_weight, 3.5, "the land cost is kept");
    assert.equal(await exists(b, "error"), false);
    // The join settings (OPT-3): two tick boxes and three lengths, each with one plain sentence of help; none is in the design at its default.
    const opt = async () => (await pageDoc(b)).stages.optimise.settings;
    for (const [id, key] of [["seams-on-straights", "seams_on_straights"], ["free-seams", "free_seams"]]) {
      assert.equal(await (await tid(b, id)).isSelected(), false);
      assert.equal((await opt())[key], undefined);
      await (await tid(b, id)).click();
      await until(b, `${key} stored`, async () => (await opt())?.[key] === true);
      await until(b, `${id} ticked`, async () => await (await tid(b, id)).isSelected());
      assert.match(await text(b, `${id}-help`), /^[^.]+\.$/, "one sentence of help");
    }
    await (await tid(b, "seams-on-straights")).click();
    await until(b, "seams on straights gone", async () => (await opt())?.seams_on_straights === undefined);
    for (const [id, key, unset, value] of [["seam-window", "seam_window_m", "1500", 900], ["min-straight", "min_straight_m", "0", 90], ["weir-straight", "weir_straight_m", "", 180]]) {
      assert.equal(await text(b, `${id}-stored`), `Stored: ${unset}`.trimEnd());
      assert.equal((await opt())[key], undefined);
      await (await tid(b, id)).setValue(String(value));
      await (await tid(b, `set-${id}`)).click();
      await until(b, `${key} stored`, async () => (await opt())?.[key] === value);
      assert.equal(await text(b, `${id}-stored`), `Stored: ${value}`);
      assert.match(await text(b, `${id}-help`), /^[^.]+\.$/, "one sentence of help");
    }
    assert.equal((await opt()).free_seams, true, "the free seams are kept");
    assert.equal(await exists(b, "error"), false);
    // A length below 0 is refused in plain words, and the design keeps what it held.
    await (await tid(b, "min-straight")).setValue("-5");
    await (await tid(b, "set-min-straight")).click();
    await until(b, "the length refusal", async () => (await exists(b, "error")) && (await text(b, "error")).includes("The length must be a number of metres, 0 or more."));
    assert.equal((await opt()).min_straight_m, 90);
    // A value below 0 is refused in plain words, and the design keeps what it held.
    await (await tid(b, "land-weight")).setValue("-1");
    await (await tid(b, "set-land-weight")).click();
    await until(b, "the refusal", async () => (await exists(b, "error")) && (await text(b, "error")).includes("The cost per m² of land must be a number, 0 or more."));
    assert.equal((await pageDoc(b)).stages.optimise.settings.land_weight, 3.5);
    // The flood settings (OPT-2): off until set, each with its help; a bad value is refused in words and stores nothing.
    assert.equal(await text(b, "flood-cost-stored"), "Stored: 0");
    assert.equal(await text(b, "bank-limit-stored"), "Stored: no limit");
    assert.match(await text(b, "flood-cost-help"), /A starting value is 3000, in the money of the cut cost; the right value is yours to set\./);
    assert.match(await text(b, "bank-limit-help"), /Bank-full is an estimate from the DEM beside the river/);
    assert.equal(await exists(b, "flood-pools"), false, "no flood table before a run with a flood setting");
    await (await tid(b, "flood-cost")).setValue("-5");
    await (await tid(b, "set-flood-cost")).click();
    await until(b, "the refusal", async () => (await exists(b, "error")) && /cost per hectare of flooded land must be a number, 0 or more/.test(await text(b, "error")));
    assert.equal((await pageDoc(b)).stages.optimise.settings?.flood_cost_per_ha, undefined);
    await (await tid(b, "flood-cost")).setValue("3000");
    await (await tid(b, "set-flood-cost")).click();
    await until(b, "the flood cost stored", async () => (await pageDoc(b)).stages.optimise.settings?.flood_cost_per_ha === 3000);
    assert.equal(await text(b, "flood-cost-stored"), "Stored: 3000");
    await (await tid(b, "bank-limit")).setValue("2.5");
    await (await tid(b, "set-bank-limit")).click();
    await until(b, "the limit stored", async () => (await pageDoc(b)).stages.optimise.settings?.max_above_bank_m === 2.5);
    assert.equal(await text(b, "bank-limit-stored"), "Stored: 2.5");
    // Run optimise with them: the table has a row for each pool, with its level, lift, flooded hectares and height above bank-full.
    await (await tid(b, "run-optimise")).click();
    await until(b, "optimise fresh", async () => (await stageStatus(b)).includes("optimise:fresh"), 240000);
    const flood = (await pageDoc(b)).stages.optimise.results.cascade.flood;
    assert.equal(flood.flood_cost_per_ha, 3000);
    assert.equal(flood.max_above_bank_m, 2.5);
    assert.ok(flood.pools.every((p) => p.max_above_bank_m <= 2.5 + 1e-9), "no pool stands above the limit");
    await until(b, "the flood table", async () => await exists(b, "flood-pools"));
    const rows = await b.execute(() => [...document.querySelectorAll("[data-testid=flood-pool]")].map((r) => [...r.querySelectorAll("td")].map((c) => c.textContent.trim())));
    assert.equal(rows.length, flood.pools.length);
    assert.deepEqual(rows[0], [flood.pools[0].level_m.toFixed(2), "–", flood.pools[0].flooded_ha.toFixed(1), flood.pools[0].max_above_bank_m.toFixed(2)]);
    assert.deepEqual(await b.execute(() => [...document.querySelectorAll("[data-testid=flood-pools] th")].map((c) => c.textContent.trim())), ["Pool level (m)", "Lift (m)", "Flooded (ha)", "Above bank-full (m)"]);
    assert.match(await text(b, "flood-total"), /^Flooded land: \d+\.\d ha outside the river, flood cost /);
    // An empty limit is no limit: the key leaves the design.
    await b.execute(() => {
      const box = document.querySelector('[data-testid="bank-limit"]');
      box.value = "";
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await (await tid(b, "set-bank-limit")).click();
    await until(b, "the limit gone", async () => (await pageDoc(b)).stages.optimise.settings?.max_above_bank_m === undefined);
    assert.equal(await text(b, "bank-limit-stored"), "Stored: no limit");
    // The window optimiser (OPT-5): the method list says the pool lattice and the design holds no method; choosing the
    // window optimiser stores it with its settings and shows the window length; a length is stored with a band that takes
    // it; a bad length is refused in words; choosing the lattice again takes the method out of the design.
    assert.equal(await (await tid(b, "optimise-method")).getValue(), "lattice");
    assert.equal((await opt()).method, undefined);
    assert.equal(await exists(b, "window-length"), false);
    await b.execute(() => {
      const sel = document.querySelector('[data-testid="optimise-method"]');
      sel.value = "windows";
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "the window method stored", async () => (await opt())?.method === "windows");
    assert.equal((await opt()).windows.window_target_m, 1500);
    await until(b, "the window length box", async () => await exists(b, "window-length"));
    await (await tid(b, "window-length")).setValue("3000");
    await (await tid(b, "set-window-length")).click();
    await until(b, "the window length stored", async () => (await opt())?.windows?.window_target_m === 3000);
    assert.equal((await opt()).windows.window_max_m, 3000);
    assert.equal(await text(b, "window-length-stored"), "Stored: 3000");
    await (await tid(b, "window-length")).setValue("abc");
    await (await tid(b, "set-window-length")).click();
    await until(b, "the window length refusal", async () => (await exists(b, "error")) && (await text(b, "error")).includes("The window length must be a number of metres"));
    await b.execute(() => {
      const sel = document.querySelector('[data-testid="optimise-method"]');
      sel.value = "lattice";
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await until(b, "the method gone", async () => (await opt())?.method === undefined && (await opt())?.windows === undefined);
  });

  it("shows that a design with no DEM was optimised on plan area, not volume (OPT-1)", async () => {
    // The synthetic example names no DEM. The command line optimises it on plan area; the app opens the result.
    const EXAMPLE = path.resolve(here, "../../examples/synthetic-river.owd");
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-plan-area-"));
    const doc = path.join(dir, "synthetic-river.owd");
    fs.copyFileSync(EXAMPLE, doc);
    const said = execFileSync(CLI, ["optimise", doc, "--length-weight", "1"], { encoding: "utf8", stdio: "pipe" });
    assert.match(said, /optimised on plan area, not volume \(no DEM\)/);
    const native = JSON.parse(fs.readFileSync(doc, "utf8"));
    assert.equal(native.stages.optimise.results.optimised_on, "plan_area");
    assert.deepEqual(native.stages.optimise.results.pools, []);

    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await pick(b, async () => (await openButton(b)).click(), doc);
    await until(b, "the design", async () => (await text(b, "file-label")).includes("synthetic-river.owd"));
    await until(b, "the plan-area note", async () => await exists(b, "plan-area-note"));
    assert.equal(
      await text(b, "plan-area-note"),
      "Optimised on plan area, not volume: no elevation model is loaded. The search kept the channel inside the river polygon; it chose no weirs. Load a DEM and run Optimise again to optimise on volume.",
    );
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
    // 5: the card is headed with the design line, not Options.
    assert.equal((await text(b, "design-line-heading")).toLowerCase(), "design line");
    const card = await text(b, "option-table");
    assert.match(card, /Dredge m³/);
    assert.doesNotMatch(card, /Option/);
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
    await (await tid(b, "profile-toggle")).click();
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
    // The seven layers of MAP-1, and the terrain shading (MAP-2) and the contours (MAP-3), which have tests of their own below.
    // And the weirs and locks, the pool levels and the pool water (MAP-4), tested below too.
    // And the channel and the cut extent (MAP-5), tested below as well.
    assert.equal((await b.$$("[data-testid=layers-list] input[type=checkbox]")).length, IDS.length + 7);
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
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    // No section is asked for while the panels are hidden; the Section toggle shows the panel and its slider.
    assert.equal(await exists(b, "station-slider"), false);
    await (await tid(b, "section-toggle")).click();
    await until(b, "the first cross section", async () => (await exists(b, "cs-chainage")), 120000);
    assert.ok(await b.execute(() => document.querySelector("[data-testid=section-panel]").contains(document.querySelector("[data-testid=station-slider]"))), "the Station slider is in the floating section panel");
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

  it("map-first layout: the design line card and the checks are in the left panel, and the map fills the rest (UI-4)", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the design line card", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    await until(b, "the checks", async () => (await b.$$("[data-testid=check]")).length > 0);
    const at = await b.execute(() => {
      const box = (sel) => {
        const r = document.querySelector(sel).getBoundingClientRect();
        return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width };
      };
      const left = document.querySelector("section.left");
      const inLeft = (id) => left.contains(document.querySelector(`[data-testid="${id}"]`));
      const order = ["Stages", "Design line", "Checks", "Data"].map((name) => left.querySelector(`section[aria-label="${name}"]`).getBoundingClientRect().top + left.scrollTop);
      return {
        card: inLeft("option-table"), checks: inLeft("checks-list"), toggle: inLeft("stretches-toggle"), order,
        cardBox: box("[data-testid=option-row]"), left: box("section.left"), map: box("[data-testid=plan-map]"), area: box("[data-testid=map-area]"),
        footerTop: document.querySelector("[data-testid=status-line]").getBoundingClientRect().top, wide: window.innerWidth,
      };
    });
    assert.ok(at.card && at.checks && at.toggle, "the design line card, the checks and the stretches button are in the left panel");
    assert.deepEqual([...at.order].sort((x, y) => x - y), at.order, `Stages, Design line, Checks, Data, in that order: ${at.order}`);
    assert.ok(at.cardBox.width <= at.left.width && at.cardBox.right <= at.left.right + 1, "the card fits the panel's width");
    // The card names each number, stacked.
    const card = await text(b, "option-row");
    for (const label of ["Dredge m³", "Dry cut m³", "Tallest face m", "Min radius m", "Radius ok", "Checks"]) assert.ok(card.includes(label), `the card shows ${label}`);
    // The map takes all the space right of the left panel, down to the status line: no fixed rows under it.
    assert.ok(Math.abs(at.map.right - at.wide) <= 1, `the map reaches the right edge: ${at.map.right} of ${at.wide}`);
    assert.ok(Math.abs(at.map.bottom - at.footerTop) <= 1, `the map reaches the status line: ${at.map.bottom} of ${at.footerTop}`);
    assert.ok(at.map.left - at.left.right <= 8, "the map starts beside the left panel");
    assert.equal(await exists(b, "bottom-panel"), false);
    assert.equal(await exists(b, "profile-panel"), false, "the profile starts hidden");
    assert.equal(await exists(b, "section-panel"), false, "the section starts hidden");
    assert.equal(await exists(b, "stretches-panel"), false, "the stretches start closed");
  });

  it("the Profile and Section toggles show and hide the floating panels; Close and Escape hide them; the slider moves the station (UI-4)", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the design line card", async () => (await b.$$("[data-testid=option-row]")).length === 1);
    const pressed = async (id) => (await tid(b, id)).getAttribute("aria-pressed");
    assert.equal(await text(b, "profile-toggle"), "Profile");
    assert.equal(await text(b, "section-toggle"), "Section");
    assert.equal(await (await tid(b, "profile-toggle")).getTagName(), "button");
    assert.equal(await pressed("profile-toggle"), "false");
    // Profile: the long section, over the map.
    await (await tid(b, "profile-toggle")).click();
    await until(b, "the profile panel", async () => await exists(b, "profile-panel"));
    assert.equal(await pressed("profile-toggle"), "true");
    await until(b, "the long section", async () => (await exists(b, "long-section")) && Number(await (await tid(b, "long-section")).getAttribute("data-stations")) > 2, 120000);
    assert.equal(await exists(b, "section-panel"), false, "Profile does not show the section");
    assert.equal(await (await tid(b, "profile-panel")).getAttribute("aria-label"), "Profile");
    // Section: the cross-section with its slider.
    await (await tid(b, "section-toggle")).click();
    await until(b, "the section panel", async () => await exists(b, "section-panel"));
    await until(b, "the first cross section", async () => await exists(b, "cs-chainage"), 120000);
    const over = await b.execute(() => {
      const r = (id) => document.querySelector(`[data-testid="${id}"]`).getBoundingClientRect();
      const [m, p, s] = [r("map-area"), r("profile-panel"), r("section-panel")];
      const inside = (x) => x.left >= m.left && x.right <= m.right + 1 && x.top >= m.top && x.bottom <= m.bottom + 1;
      const ground = getComputedStyle(document.querySelector("[data-testid=section-panel]")).backgroundColor;
      return { inside: inside(p) && inside(s), ground };
    });
    assert.ok(over.inside, "both panels float inside the map area");
    const alpha = /\/\s*([\d.]+)\)|rgba\([^)]*,\s*([\d.]+)\)/.exec(over.ground);
    assert.ok(alpha && Number(alpha[1] ?? alpha[2]) < 1 && Number(alpha[1] ?? alpha[2]) > 0.5, `the panel ground is see-through but readable: ${over.ground}`);
    // The slider in the floating panel changes the station and the section drawn.
    const drawn = () => b.execute(() => ({ chainage: document.querySelector("[data-testid=cross-section]").dataset.chainage, terrain: document.querySelector("[data-testid=cs-terrain]")?.getAttribute("d") ?? "" }));
    const first = await drawn();
    await b.execute(() => {
      const el = document.querySelector("[data-testid=section-panel] [data-testid=station-slider]");
      el.value = String(Math.floor(Number(el.max) / 2));
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await until(b, "the section to move", async () => (await drawn()).chainage !== first.chainage && !(await exists(b, "cross-busy")));
    assert.notEqual((await drawn()).terrain, first.terrain, "another station has another ground profile");
    // Close hides the section; Escape hides the focused profile and gives the focus back to its toggle.
    await (await tid(b, "section-close")).click();
    await until(b, "the section to hide", async () => !(await exists(b, "section-panel")));
    assert.equal(await pressed("section-toggle"), "false");
    await (await tid(b, "profile-toggle")).click();
    await until(b, "the profile to hide", async () => !(await exists(b, "profile-panel")));
    await (await tid(b, "profile-toggle")).click();
    await until(b, "the profile again", async () => await exists(b, "profile-panel"));
    await until(b, "the focus in the profile", async () => await b.execute(() => document.querySelector("[data-testid=profile-panel]").contains(document.activeElement)));
    await b.keys("Escape");
    await until(b, "Escape to hide the profile", async () => !(await exists(b, "profile-panel")));
    assert.equal(await pressed("profile-toggle"), "false");
    assert.ok(await b.execute(() => document.activeElement === document.querySelector("[data-testid=profile-toggle]")), "the focus is back on the toggle");
    // Both hidden: only the map is in the map area.
    assert.equal(await exists(b, "section-panel"), false);
    assert.equal(await exists(b, "error"), false);
  });

  it("the constraint stretches drawer opens from the left panel, lists the rows, picks one for the map, and closes (UI-4)", async () => {
    // The map needs WebGL to draw the picked stretch: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await (await b.$("button=Run stale")).click();
    await until(b, "volumes fresh", async () => (await stageStatus(b)).includes("volumes:fresh"), 120000);
    const doc = await pageDoc(b);
    const n = doc.stages.volumes.results.screening.constraint_stretches.length;
    assert.ok(n > 0, "the example has stretches");
    await until(b, "the count on the button", async () => (await text(b, "stretches-toggle")) === `Constraint stretches (${n})`);
    const toggle = await tid(b, "stretches-toggle");
    assert.equal(await toggle.getTagName(), "button");
    assert.equal(await toggle.getAttribute("aria-expanded"), "false");
    assert.equal(await exists(b, "stretch-row"), false, "no rows are drawn while the drawer is closed");
    await toggle.click();
    await until(b, "the rows", async () => (await b.$$("[data-testid=stretch-row]")).length === n);
    assert.equal(await toggle.getAttribute("aria-expanded"), "true");
    const under = await b.execute(() => {
      const r = (id) => document.querySelector(`[data-testid="${id}"]`).getBoundingClientRect();
      return { mapBottom: r("map-area").bottom, top: r("stretches-panel").top, bottom: r("stretches-panel").bottom, footer: r("status-line").top, mapHeight: r("map-area").height };
    });
    assert.ok(Math.abs(under.top - under.mapBottom) <= 1 && Math.abs(under.bottom - under.footer) <= 1, `the drawer is under the map: ${JSON.stringify(under)}`);
    assert.ok(under.mapHeight > 300, "the map keeps most of the height");
    await b.execute(() => document.querySelector('[data-testid=stretch-row][data-index="0"]').click());
    await until(b, "the map to show the picked stretch", async () => (await (await tid(b, "plan-map")).getAttribute("data-stretch")) === `${doc.stages.volumes.results.id}:0`);
    assert.equal(await (await b.$('[data-testid=stretch-row][data-index="0"]')).getAttribute("aria-selected"), "true");
    await (await tid(b, "stretches-close")).click();
    await until(b, "the drawer to close", async () => !(await exists(b, "stretches-panel")));
    assert.equal(await toggle.getAttribute("aria-expanded"), "false");
    // The button opens it again, and Escape closes it from inside.
    await toggle.click();
    await until(b, "the drawer again", async () => await exists(b, "stretches-panel"));
    await until(b, "the focus in the drawer", async () => await b.execute(() => document.querySelector("[data-testid=stretches-panel]").contains(document.activeElement)));
    await b.keys("Escape");
    await until(b, "Escape to close the drawer", async () => !(await exists(b, "stretches-panel")));
  });

  it("remembers where a floating panel was put, and its size, after a reload; nothing of it is in the design (UI-4)", async () => {
    const b = await start();
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await (await tid(b, "section-toggle")).click();
    await until(b, "the section panel", async () => await exists(b, "section-panel"));
    const rect = () =>
      b.execute(() => {
        const s = document.querySelector("[data-testid=section-panel]").style;
        return [s.left, s.top, s.width, s.height].map((v) => parseFloat(v));
      });
    await until(b, "the panel to be placed", async () => (await rect())[2] > 0);
    const start0 = await rect();
    // Drag by the title bar, then resize from the corner, with the pointer.
    const drag = (id, dx, dy) =>
      b.execute((id, dx, dy) => {
        const el = document.querySelector(`[data-testid="${id}"]`);
        const r = el.getBoundingClientRect();
        const [x, y] = [r.left + Math.min(r.width / 2, 40), r.top + r.height / 2];
        const fire = (type, cx, cy) => el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: cx, clientY: cy }));
        fire("pointerdown", x, y);
        fire("pointermove", x + dx / 2, y + dy / 2);
        fire("pointermove", x + dx, y + dy);
        fire("pointerup", x + dx, y + dy);
      }, id, dx, dy);
    await drag("section-bar", -180, -140);
    const moved = await rect();
    assert.deepEqual([moved[0], moved[1]], [start0[0] - 180, start0[1] - 140], "the title bar moves the panel");
    assert.deepEqual([moved[2], moved[3]], [start0[2], start0[3]], "moving keeps the size");
    await drag("section-grip", 60, 50);
    const sized = await rect();
    assert.deepEqual(sized, [moved[0], moved[1], moved[2] + 60, moved[3] + 50], "the corner resizes the panel");
    const stored = await b.execute(() => localStorage.getItem("owd-panels"));
    assert.deepEqual(JSON.parse(stored).section, { x: sized[0], y: sized[1], w: sized[2], h: sized[3] });
    assert.ok(!JSON.stringify(await pageDoc(b)).includes("owd-panels"), "the placement is not in the design");
    await b.refresh();
    await loaded(b);
    await until(b, "the design back", async () => (await text(b, "file-label")).includes("river-example.owd"));
    assert.equal(await exists(b, "section-panel"), false, "the panels start hidden after a reload");
    await (await tid(b, "section-toggle")).click();
    await until(b, "the panel where it was put", async () => (await exists(b, "section-panel")) && (await rect())[2] > 0);
    assert.deepEqual(await rect(), sized, "the place and size are remembered");
    // The profile was never placed: it has its own default, and no entry.
    assert.equal(JSON.parse(await b.execute(() => localStorage.getItem("owd-panels"))).profile, undefined);
  });

  it("the 3D view of the DEM (MAP-2): off with no DEM, on it sets the map's terrain and tilt, the exaggeration and the terrain shading work, all is remembered, and the design does not change", async () => {
    // Terrain needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const mapAttr = async (name) => (await tid(b, "plan-map")).getAttribute(name);
    const mapState = () => b.execute(() => {
      const m = window.__owdMap;
      const t = m.getTerrain();
      return { terrain: t ? { source: t.source, exaggeration: t.exaggeration } : null, pitch: m.getPitch(), bearing: m.getBearing(), shading: m.getLayer("terrain-shading") ? m.getLayoutProperty("terrain-shading", "visibility") : "absent", sources: ["dem", "dem-shade"].filter((id) => m.getSource(id)) };
    });
    /** The saved text of the design, as the page keeps it for a reload: every byte of the file. */
    const savedText = async () => {
      await b.execute(() => window.__owdPlatform.call("snapshot"));
      await new Promise((r) => setTimeout(r, 300));
      return b.execute(() => JSON.parse(localStorage.getItem("owd.lastDesign")).text);
    };
    await until(b, "the map", () => b.execute(() => !!window.__owdMap));
    // No DEM in memory: a real button, off, that says why. No exaggeration to choose; the shading has nothing to draw.
    const button = await tid(b, "terrain-3d");
    assert.equal(await button.getTagName(), "button");
    assert.equal(await button.getText(), "3D");
    assert.equal(await button.isEnabled(), false);
    assert.match(await button.getAttribute("title"), /load a DEM first/);
    assert.equal(await button.getAttribute("aria-pressed"), "false");
    assert.equal(await exists(b, "terrain-exaggeration"), false);
    assert.equal(await b.execute(() => window.__owdPlatform.call("terrain_info")), null);
    await (await tid(b, "layers-button")).click();
    const shadeBox = () => b.execute(() => {
      const c = document.querySelector('[data-testid="layer-terrain-shading"]');
      return { checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
    });
    assert.deepEqual(await shadeBox(), { checked: false, disabled: true, row: "Terrain shading none loaded" });
    assert.deepEqual((await mapState()).sources, []);

    // The example brings its DEM: the button comes on, the map has the DEM's tiles, and nothing is in 3D yet.
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the 3D button to come on", () => button.isEnabled());
    assert.match(await button.getAttribute("title"), /^3D: see the DEM as a surface/);
    await until(b, "the DEM's tiles on the map", async () => (await mapState()).sources.length === 2);
    await b.execute(() => {
      window.__mapErrors = [];
      window.__owdMap.on("error", (e) => window.__mapErrors.push(`${e.sourceId ?? ""}: ${e.error?.message ?? e.error}`));
    });
    const info = await b.execute(() => window.__owdPlatform.call("terrain_info"));
    assert.equal(info.encoding, "terrarium");
    assert.equal(info.tile_size, 256);
    assert.ok(info.min_m < info.max_m && info.base_m >= info.min_m && info.base_m <= info.max_m, JSON.stringify(info));
    const g = await b.execute(async () => (await window.__owdPlatform.call("map_geometry")).bbox);
    const [w, s, e, n] = info.bounds;
    assert.ok(w < g[0] && e > g[2] && s < g[1] && n > g[3], `the DEM ${info.bounds} holds the design ${g}`);
    // A tile the engine makes here: a PNG whose heights, read back, are the DEM's own.
    const tile = await b.execute(async (bounds, zoom) => {
      const [w, s, e, n] = bounds;
      const lon = (w + e) / 2, lat = (s + n) / 2, k = 2 ** zoom;
      const x = Math.floor(((lon + 180) / 360) * k);
      const sin = Math.sin((lat * Math.PI) / 180);
      const y = Math.floor((0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * k);
      const t0 = performance.now();
      const png = await window.__owdPlatform.call("terrain_tile", { z: zoom, x, y });
      const first = performance.now() - t0;
      const t1 = performance.now();
      await window.__owdPlatform.call("terrain_tile", { z: zoom, x, y });
      const again = performance.now() - t1;
      const bitmap = await createImageBitmap(new Blob([png], { type: "image/png" }), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(bitmap, 0, 0);
      const px = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let lo = Infinity, hi = -Infinity;
      for (let i = 0; i < px.length; i += 4) {
        const h = px[i] * 256 + px[i + 1] + px[i + 2] / 256 - 32768;
        lo = Math.min(lo, h);
        hi = Math.max(hi, h);
      }
      const far = await window.__owdPlatform.call("terrain_tile", { z: 3, x: 0, y: 0 });
      return { signature: [...png.slice(0, 8)], width: bitmap.width, height: bitmap.height, lo, hi, first, again, far: far.length };
    }, info.bounds, info.max_zoom);
    assert.deepEqual(tile.signature, [137, 80, 78, 71, 13, 10, 26, 10], "a PNG");
    assert.equal(`${tile.width}x${tile.height}`, "256x256");
    assert.ok(tile.lo >= info.min_m - 0.01 && tile.hi <= info.max_m + 0.01 && tile.hi > tile.lo, `the tile's heights ${tile.lo} to ${tile.hi} are the DEM's (${info.min_m} to ${info.max_m})`);
    assert.ok(tile.far > 0, "a tile off the DEM is still a tile (flat), never a hole");
    console.log(`terrain tile in the browser (wasm, SwiftShader run): first ${tile.first.toFixed(1)} ms, cached ${tile.again.toFixed(1)} ms`);
    assert.deepEqual((await mapState()).terrain, null);
    assert.equal(await mapAttr("data-terrain"), "off");
    const before = await savedText();

    // On: the map takes the DEM as its terrain at twice the height, and tilts.
    await button.click();
    await until(b, "the terrain on the map", async () => (await mapAttr("data-terrain")) === "on");
    assert.deepEqual((await mapState()).terrain, { source: "dem", exaggeration: 2 });
    await until(b, "the map to tilt", async () => Math.abs((await mapState()).pitch - 60) < 0.5);
    assert.equal(await button.getAttribute("aria-pressed"), "true");
    const select = await tid(b, "terrain-exaggeration");
    assert.equal(await select.getTagName(), "select");
    assert.equal(await select.getValue(), "2");
    assert.deepEqual(await b.execute(() => [...document.querySelectorAll('[data-testid="terrain-exaggeration"] option')].map((o) => o.textContent)), ["1×", "2×", "5×", "10×"]);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-3d")), '{"on":true,"exaggeration":2}');
    // The surface is the DEM: the map can read a height from it once its tiles are in.
    await until(b, "the terrain tiles", () => b.execute(() => window.__owdMap.isSourceLoaded("dem")), 120000);
    // The exaggeration.
    await select.selectByAttribute("value", "5");
    await until(b, "five times the height", async () => (await mapState()).terrain?.exaggeration === 5);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-3d")), '{"on":true,"exaggeration":5}');
    assert.ok(Math.abs((await mapState()).pitch - 60) < 0.5, "a change of height does not move the camera");
    // The compass is there to turn the map back.
    assert.equal(await (await b.$(".maplibregl-ctrl-compass")).isExisting(), true);

    // Terrain shading: a layer of its own, with the imagery or without it.
    assert.deepEqual(await shadeBox(), { checked: false, disabled: false, row: "Terrain shading" });
    assert.equal((await mapState()).shading, "none");
    await (await tid(b, "layer-terrain-shading")).click();
    await until(b, "the shading", async () => (await mapState()).shading === "visible");
    assert.equal(await mapAttr("data-terrain-shading"), "shown");
    assert.equal(await mapAttr("data-layers-off"), "");
    await (await tid(b, "layer-imagery")).click();
    await until(b, "the imagery to go", () => b.execute(() => window.__owdMap.getLayoutProperty("basemap", "visibility") === "none"));
    assert.equal((await mapState()).shading, "visible", "the shaded DEM stays with the imagery off");
    assert.equal(await mapAttr("data-layers-off"), "imagery");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["terrain-shading","imagery"]');
    await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()), 120000);
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-terrain-3d.png"));
    }
    assert.deepEqual(await b.execute(() => window.__mapErrors.filter((m) => /^dem/.test(m))), [], "no terrain tile failed");

    // In 3D the station pick still answers: a click on the design line sets the station and opens the panels.
    await (await tid(b, "layer-imagery")).click();
    const lineAt = await b.execute(async () => {
      const m = window.__owdMap;
      const line = (await window.__owdPlatform.call("map_geometry")).options[0].line.coordinates;
      m.jumpTo({ center: line[Math.floor(line.length / 2)], zoom: 14 });
      await new Promise((r) => m.once("idle", r));
      // A point of the line the camera can see: in 3D a bank can hide the bed behind it, and a click
      // lands on what is in front. Seen means the map reads the same place back from its pixel.
      const metres = (a, b) => Math.hypot((a[0] - b[0]) * 111320 * Math.cos((a[1] * Math.PI) / 180), (a[1] - b[1]) * 110540);
      const box = m.getCanvas().getBoundingClientRect();
      const seen = line.map((at) => ({ at, p: m.project(at) })).filter(({ at, p }) => {
        const back = m.unproject(p);
        return p.x > 200 && p.x < box.width - 200 && p.y > 150 && p.y < box.height - 150 && metres([back.lng, back.lat], at) < 2;
      });
      const { at, p } = seen[Math.floor(seen.length / 2)];
      return { x: Math.round(box.left + p.x), y: Math.round(box.top + p.y), lon: at[0], lat: at[1], seen: seen.length, of: line.length };
    });
    assert.ok(lineAt.seen > 0, "some of the design line is in sight");
    await b.action("pointer").move({ x: lineAt.x, y: lineAt.y }).down().up().perform();
    await until(b, "the station marker", () => exists(b, "station-marker"));
    const marker = (await (await tid(b, "station-marker")).getAttribute("data-position")).split(",").map(Number);
    const metres = Math.hypot((marker[0] - lineAt.lon) * 111320 * Math.cos((lineAt.lat * Math.PI) / 180), (marker[1] - lineAt.lat) * 110540);
    console.log(`station pick on terrain (5x): the marker is ${metres.toFixed(1)} m from the point clicked on the design line; ${lineAt.seen} of ${lineAt.of} line points were in sight`);
    assert.ok(metres < 60, `the station is where the click was, on the terrain (${metres.toFixed(1)} m off)`);
    await until(b, "the floating panels", async () => (await exists(b, "profile-panel")) && (await exists(b, "section-panel")));

    // Nothing of this is in the design: the saved text is the same, byte for byte.
    const after = await savedText();
    assert.equal(after, before, "the design file is byte-identical after 3D, the exaggeration and the shading");
    assert.doesNotMatch(after, /owd-map-3d|terrain-shading|exaggeration|owd-dem/);

    // Remembered on this machine. A reload has the design and not the DEM (a browser keeps no file): 3D waits for it.
    await b.refresh();
    await loaded(b);
    await until(b, "the example after a reload", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the map again", () => b.execute(() => !!window.__owdMap));
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-3d")), '{"on":true,"exaggeration":5}');
    const again = await tid(b, "terrain-3d");
    assert.equal(await again.isEnabled(), false, "no DEM in memory after a reload");
    assert.match(await again.getAttribute("title"), /load a DEM first/);
    assert.equal(await mapAttr("data-terrain"), "off");
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "3D to come back with the DEM", async () => (await mapAttr("data-terrain")) === "on", 60000);
    assert.deepEqual((await mapState()).terrain, { source: "dem", exaggeration: 5 });
    await until(b, "the tilt to come back", async () => Math.abs((await mapState()).pitch - 60) < 0.5);
    assert.equal(await (await tid(b, "terrain-exaggeration")).getValue(), "5");
    assert.equal((await mapState()).shading, "visible", "the shading tick is remembered too");

    // Off: flat and north up again, and the exaggeration goes.
    await b.execute(() => void window.__owdMap.jumpTo({ bearing: 40 }));
    await again.click();
    await until(b, "the terrain off", async () => (await mapAttr("data-terrain")) === "off");
    await until(b, "the flat view, north up", async () => {
      const m = await mapState();
      return m.terrain === null && Math.abs(m.pitch) < 0.01 && Math.abs(m.bearing) < 0.01;
    });
    assert.equal(await again.getAttribute("aria-pressed"), "false");
    assert.equal(await exists(b, "terrain-exaggeration"), false);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-3d")), '{"on":false,"exaggeration":5}');
    assert.equal(await exists(b, "error"), false);
  });

  it("contours of the DEM (MAP-3): off with no DEM, on they are a map layer with lines at the interval, in 2D and in 3D, the interval and the tick are remembered, and the design does not change", async () => {
    // The map needs WebGL to load tiles: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const mapAttr = async (name) => (await tid(b, "plan-map")).getAttribute(name);
    const box = () => b.execute(() => {
      const c = document.querySelector('[data-testid="layer-contours"]');
      return { type: c.type, checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
    });
    /** What the map itself holds of the contours: its layers in drawing order, the tile address, and the lines of the tiles in memory. */
    const mapState = () => b.execute(() => {
      const m = window.__owdMap;
      const layers = m.getStyle().layers.map((l) => l.id);
      const source = m.getSource("contours");
      const features = source ? m.querySourceFeatures("contours", { sourceLayer: "contours" }) : [];
      return {
        layers: layers.filter((id) => /^contours/.test(id)),
        order: ["terrain-shading", "contours-casing", "contours", "contours-labels", "polygon"].map((id) => layers.indexOf(id)),
        tiles: source ? m.getStyle().sources.contours.tiles : null,
        loaded: source ? m.isSourceLoaded("contours") : false,
        features: features.length,
        levels: [...new Set(features.map((f) => f.properties.level))].sort((a, c) => a - c),
        index: [...new Set(features.filter((f) => f.properties.index).map((f) => f.properties.level))].sort((a, c) => a - c),
        types: [...new Set(features.map((f) => f.geometry.type))].sort(),
        zoom: m.getZoom(),
      };
    });
    const savedText = async () => {
      await b.execute(() => window.__owdPlatform.call("snapshot"));
      await new Promise((r) => setTimeout(r, 300));
      return b.execute(() => JSON.parse(localStorage.getItem("owd.lastDesign")).text);
    };
    await until(b, "the map", () => b.execute(() => !!window.__owdMap));
    // No DEM in memory: a tick box, off, greyed, that says why. No interval to choose, nothing on the map or in the legend.
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box(), { type: "checkbox", checked: false, disabled: true, row: "Contours none loaded" });
    assert.equal(await exists(b, "contour-interval"), false);
    assert.equal(await mapAttr("data-contours"), "off");
    assert.equal(await exists(b, "legend-contours"), false);
    assert.deepEqual((await mapState()).layers, []);

    // The example brings its DEM: the tick can be set. It is off at first, and the map has asked for no contour.
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the contours tick to come on", async () => !(await box()).disabled);
    assert.deepEqual(await box(), { type: "checkbox", checked: false, disabled: false, row: "Contours" });
    assert.equal((await mapState()).tiles, null, "no contour source while the tick is off");
    assert.equal(await exists(b, "contour-interval"), false);
    await b.execute(() => {
      window.__mapErrors = [];
      window.__owdMap.on("error", (e) => window.__mapErrors.push(`${e.sourceId ?? ""}: ${e.error?.message ?? e.error}`));
    });
    const info = await b.execute(() => window.__owdPlatform.call("terrain_info"));
    const before = await savedText();

    // On: the map has the contour layers, over the terrain shading and under the design, from tiles the page makes.
    await (await tid(b, "layer-contours")).click();
    await until(b, "the contours on the map", async () => (await mapAttr("data-contours")) === "on");
    let state = await mapState();
    assert.deepEqual(state.layers, ["contours-casing", "contours", "contours-labels"]);
    assert.ok(state.order.every((at, i) => at >= 0 && (i === 0 || at > state.order[i - 1])), `over the shading, under the design: ${state.order}`);
    assert.deepEqual(state.tiles, [`owd-contour://${info.id}/auto/{z}/{x}/{y}`]);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["contours"]');
    assert.equal(await mapAttr("data-layers-off"), "", "a layer that starts off is not listed as turned off");
    // The interval choice sits beside the tick: Auto first.
    const select = await tid(b, "contour-interval");
    assert.equal(await select.getTagName(), "select");
    assert.equal(await select.getValue(), "auto");
    assert.deepEqual(await b.execute(() => [...document.querySelectorAll('[data-testid="contour-interval"] option')].map((o) => o.textContent)), ["Auto", "1 m", "2 m", "5 m", "10 m", "20 m"]);
    assert.equal(await b.execute(() => document.querySelector('[data-testid="contour-interval"]').closest("label")), null, "the choice is beside the tick's label, not inside it");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-contours")), null, "Auto leaves nothing behind");
    // The lines: the map has features, every level a whole multiple of the interval Auto picks at this zoom level.
    const auto = (z) => (z >= 15 ? 1 : z >= 14 ? 2 : z >= 13 ? 5 : z >= 12 ? 10 : 20);
    await until(b, "the contour tiles", async () => (await mapState()).loaded, 120000);
    await until(b, "contour lines on the map", async () => (await mapState()).features > 0, 120000);
    state = await mapState();
    const interval = auto(Math.floor(state.zoom));
    assert.equal(await mapAttr("data-contour-interval"), String(interval));
    assert.ok(state.levels.length >= 2, `more than one level at ${interval} m: ${state.levels}`);
    for (const level of state.levels) {
      assert.ok(Number.isInteger(level / interval), `${level} m is a multiple of ${interval} m`);
      assert.ok(level > info.min_m && level <= info.max_m, `${level} m is a height of the DEM (${info.min_m} to ${info.max_m})`);
    }
    for (const level of state.index) assert.ok(Number.isInteger(level / (interval * 5)), `${level} m is an index contour at ${interval} m`);
    assert.ok(state.types.every((t) => t === "LineString" || t === "MultiLineString"), state.types.join(" "));
    // The legend lists them, with the interval drawn.
    assert.equal((await text(b, "legend-contours")).trim(), `contour, every ${interval} m`);
    assert.equal((await text(b, "legend-contours-index")).trim(), `index contour, every ${interval * 5} m, level in metres`);

    // A tile the engine makes here: a vector tile for a place on the DEM, nothing for a place off it.
    const tile = await b.execute(async (bounds) => {
      const [w, s, e, n] = bounds;
      const lon = (w + e) / 2, lat = (s + n) / 2, k = 2 ** 14;
      const x = Math.floor(((lon + 180) / 360) * k);
      const sin = Math.sin((lat * Math.PI) / 180);
      const y = Math.floor((0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * k);
      const t0 = performance.now();
      const one = await window.__owdPlatform.call("contour_tile", { z: 14, x, y, interval: 0.5 });
      const first = performance.now() - t0;
      const t1 = performance.now();
      await window.__owdPlatform.call("contour_tile", { z: 14, x, y, interval: 0.5 });
      const again = performance.now() - t1;
      const five = await window.__owdPlatform.call("contour_tile", { z: 14, x, y, interval: 5 });
      const far = await window.__owdPlatform.call("contour_tile", { z: 3, x: 0, y: 0, interval: 5 });
      const refused = await window.__owdPlatform.call("contour_tile", { z: 14, x, y, interval: 0 }).then(() => "", (e) => String(e));
      return { one: one.length, five: five.length, far: far.length, first, again, refused, name: new TextDecoder().decode(one.slice(0, 16)) };
    }, info.bounds);
    assert.ok(tile.five > 0 && tile.one > tile.five, `a finer interval has more lines (${tile.one} and ${tile.five} bytes)`);
    assert.match(tile.name, /contours/, "the vector tile's layer is named");
    assert.equal(tile.far, 0, "no lines off the DEM");
    assert.match(tile.refused, /interval/);
    console.log(`contour tile in the browser (wasm, SwiftShader run): first ${tile.first.toFixed(1)} ms, cached ${tile.again.toFixed(1)} ms, ${tile.one} bytes at 0.5 m`);

    // The interval: 5 m, zoomed in. The map asks for other tiles, and every level is a multiple of 5 m.
    await select.selectByAttribute("value", "5");
    await until(b, "the 5 m tiles", async () => (await mapState()).tiles?.[0] === `owd-contour://${info.id}/5/{z}/{x}/{y}`);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-contours")), "5");
    await b.execute((bounds) => void window.__owdMap.jumpTo({ center: [(bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2], zoom: 14.2 }), info.bounds);
    await until(b, "the 5 m lines", async () => {
      const s = await mapState();
      return s.loaded && s.features > 0;
    }, 120000);
    state = await mapState();
    assert.equal(await mapAttr("data-contour-interval"), "5");
    for (const level of state.levels) assert.ok(Number.isInteger(level / 5), `${level} m at 5 m`);
    for (const level of state.index) assert.ok(Number.isInteger(level / 25), `${level} m is an index contour at 5 m`);
    assert.equal((await text(b, "legend-contours")).trim(), "contour, every 5 m");
    // Zoomed far out the chosen interval is drawn coarser, and the legend says which; further out, no contour.
    await b.execute(() => void window.__owdMap.jumpTo({ zoom: 11.4 }));
    await until(b, "10 m at zoom 11", async () => (await mapAttr("data-contour-interval")) === "10");
    assert.equal((await text(b, "legend-contours")).trim(), "contour, every 10 m");
    await b.execute(() => void window.__owdMap.jumpTo({ zoom: 7 }));
    await until(b, "no contour far out", async () => (await mapAttr("data-contour-interval")) === "");
    assert.match(await text(b, "legend-contours"), /zoom in/);
    assert.equal(await exists(b, "legend-contours-index"), false);

    // Auto again, close in: 1 m lines, the index contours every 5 m, and the page has drawn their levels.
    await select.selectByAttribute("value", "auto");
    await b.execute((bounds) => void window.__owdMap.jumpTo({ center: [(bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2], zoom: 15.3 }), info.bounds);
    await until(b, "1 m at zoom 15", async () => (await mapAttr("data-contour-interval")) === "1");
    await until(b, "the 1 m lines", async () => {
      const s = await mapState();
      return s.loaded && s.index.length > 0;
    }, 120000);
    state = await mapState();
    for (const level of state.index) assert.ok(Number.isInteger(level / 5), `${level} m is an index contour at 1 m`);
    assert.ok(state.levels.some((l) => l % 5 !== 0), `plain contours between the index contours: ${state.levels}`);
    await until(b, "the levels along the index contours", () => b.execute(() => window.__owdMap.listImages().some((id) => /^owd-contour-level:-?\d+$/.test(id))), 120000);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-contours")), null);

    // In 3D the lines are still drawn, on the surface; over the terrain shading and without the imagery too.
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the terrain on the map", async () => (await mapAttr("data-terrain")) === "on");
    await (await tid(b, "layer-terrain-shading")).click();
    await (await tid(b, "layer-imagery")).click();
    await until(b, "the map to settle in 3D", () => b.execute(() => window.__owdMap.loaded() && !window.__owdMap.isMoving()), 120000);
    await until(b, "contour lines drawn in 3D", () => b.execute(() => window.__owdMap.queryRenderedFeatures({ layers: ["contours"] }).length > 0), 120000);
    state = await mapState();
    assert.ok(state.order.every((at, i) => at >= 0 && (i === 0 || at > state.order[i - 1])), `over the shading, under the design: ${state.order}`);
    assert.equal(await mapAttr("data-contours"), "on");
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-contours-3d.png"));
    }
    assert.deepEqual(await b.execute(() => window.__mapErrors.filter((m) => /^(contours|dem)/.test(m))), [], "no contour or terrain tile failed");
    await (await tid(b, "terrain-3d")).click();
    await (await tid(b, "layer-terrain-shading")).click();
    await (await tid(b, "layer-imagery")).click();
    await until(b, "the terrain off", async () => (await mapAttr("data-terrain")) === "off");

    // Nothing of this is in the design: the saved text is the same, byte for byte.
    await select.selectByAttribute("value", "5");
    const after = await savedText();
    assert.equal(after, before, "the design file is byte-identical after the contours and the interval");
    assert.doesNotMatch(after, /contour/i);

    // Remembered on this machine. A reload has the design and not the DEM (a browser keeps no file): the tick waits for it.
    await b.refresh();
    await loaded(b);
    await until(b, "the example after a reload", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the map again", () => b.execute(() => !!window.__owdMap));
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["contours"]');
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-contours")), "5");
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box(), { type: "checkbox", checked: true, disabled: true, row: "Contours none loaded" });
    assert.equal(await mapAttr("data-contours"), "off");
    assert.equal(await exists(b, "contour-interval"), false);
    assert.equal(await exists(b, "legend-contours"), false);
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the contours to come back with the DEM", async () => (await mapAttr("data-contours")) === "on", 60000);
    assert.equal(await (await tid(b, "contour-interval")).getValue(), "5");
    assert.match((await mapState()).tiles[0], /^owd-contour:\/\/\d+\/5\//);
    assert.equal(await exists(b, "legend-contours"), true);

    // Off: the layers and their source go, and the interval choice with them.
    await (await tid(b, "layer-contours")).click();
    await until(b, "the contours off", async () => (await mapAttr("data-contours")) === "off");
    state = await mapState();
    assert.deepEqual(state.layers, []);
    assert.equal(state.tiles, null);
    assert.equal(await exists(b, "contour-interval"), false);
    assert.equal(await exists(b, "legend-contours"), false);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), null);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-contours")), "5", "the interval is kept for the next time");
    assert.equal(await exists(b, "error"), false);
  });

  it("weirs, locks and pools (MAP-4): the layers are greyed without pools or a DEM, a weir has its label and its card, the pool water is a map layer with a read-out and a land table, the 3D view stands on the water, the ticks are remembered, and the design does not change", async () => {
    // The synthetic example with three pools: 1.0 m from 0 m, 2.5 m from 1000 m and 4.0 m from 2000 m of river.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-pools-"));
    const doc = path.join(dir, "river-pools.owd");
    fs.copyFileSync(path.join(FIXTURES, "river-example.owd"), doc);
    fs.writeFileSync(path.join(dir, "pools.csv"), "chainage_m,level_m\n0,1.0\n1000,2.5\n2000,4.0\n");
    execFileSync(CLI, ["import", doc, "--water-pools", path.join(dir, "pools.csv")], { encoding: "utf8" });
    // The map needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const mapAttr = async (name) => (await tid(b, "plan-map")).getAttribute(name);
    const box = (id) => b.execute((i) => {
      const c = document.querySelector(`[data-testid="layer-${i}"]`);
      return { checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
    }, id);
    const savedText = async () => {
      await b.execute(() => window.__owdPlatform.call("snapshot"));
      await new Promise((r) => setTimeout(r, 300));
      return b.execute(() => JSON.parse(localStorage.getItem("owd.lastDesign")).text);
    };
    const mapState = () => b.execute(() => {
      const m = window.__owdMap;
      const t = m.getTerrain();
      const layers = m.getStyle().layers.map((l) => l.id);
      return { terrain: t ? t.source : null, sources: ["pool-water", "dem-water"].filter((id) => m.getSource(id)), tiles: m.getSource("pool-water") ? m.getStyle().sources["pool-water"].tiles : null, order: ["basemap", "pool-water", "polygon", "options", "structures-weir", "structures-3d"].map((id) => layers.indexOf(id)), blocks: m.getLayer("structures-3d") ? m.getLayoutProperty("structures-3d", "visibility") : "absent" };
    });
    await until(b, "the map", () => b.execute(() => !!window.__owdMap));

    // No design: the three layers are greyed and say "none". The pool water starts unticked.
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box("structures"), { checked: true, disabled: true, row: "Weirs and locks none" });
    assert.deepEqual(await box("pool-levels"), { checked: true, disabled: true, row: "Pool levels none" });
    assert.deepEqual(await box("pool-water"), { checked: false, disabled: true, row: "Pool water none" });
    // The example has a DEM and one flat water level: no pools, so still none, and an empty list from the engine.
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the example's DEM on the map", () => (async () => !(await (await tid(b, "terrain-3d")).getAttribute("disabled")))());
    const none = await b.execute(() => window.__owdPlatform.call("map_structures"));
    assert.deepEqual([none.pools, none.weirs], [[], []]);
    assert.equal(none.model.note, "placeholder model, not a design");
    assert.equal(await b.execute(() => window.__owdPlatform.call("pool_water_info")), null);
    assert.equal((await box("structures")).disabled, true);
    assert.deepEqual(await box("pool-water"), { checked: false, disabled: true, row: "Pool water none" });
    assert.equal(await mapAttr("data-structures"), "0");
    assert.equal(await exists(b, "legend-structures"), false);

    // The design with three pools, opened without its DEM: two weirs and three pools on the map.
    await (await tid(b, "layers-button")).click();
    await pick(b, async () => (await openButton(b)).click(), doc);
    await until(b, "the file name", async () => (await text(b, "file-label")).includes("river-pools.owd"));
    await until(b, "the weirs on the map", async () => (await mapAttr("data-structures")) === "2");
    const list = await b.execute(() => window.__owdPlatform.call("map_structures"));
    assert.deepEqual(list.pools.map((p) => [p.number, p.level_m]), [[1, 1], [2, 2.5], [3, 4]]);
    assert.deepEqual(list.weirs.map((w) => [w.number, w.upstream_level_m, w.downstream_level_m, w.lift_m, w.upstream_pool]), [[1, 2.5, 1, 1.5, 2], [2, 4, 2.5, 1.5, 3]]);
    for (const w of list.weirs) {
      assert.ok(w.clipped_to_river && w.weir_length_m > 20 && w.weir_length_m < 80, `the weir is as long as the river is wide: ${w.weir_length_m}`);
      assert.deepEqual([...new Set(w.parts.map((p) => p.kind))].sort(), ["lock_chamber", "lock_gate", "lock_wall", "weir"]);
    }
    const km = (w) => (w.chainage_m / 1000).toFixed(1);
    const labels = await b.execute(() => [...document.querySelectorAll('[data-testid="weir-marker"]')].map((e) => [e.querySelector(".weir-mark").textContent, e.querySelector(".weir-text").textContent, e.title]));
    assert.deepEqual(labels.map((l) => l.slice(0, 2)), [["1", `Weir 1 · km ${km(list.weirs[0])} · pool 2.5 m · lift 1.5 m`], ["2", `Weir 2 · km ${km(list.weirs[1])} · pool 4.0 m · lift 1.5 m`]]);
    assert.match(labels[0][2], /placeholder model, not a design/);
    assert.deepEqual(await b.execute(() => [...document.querySelectorAll('[data-testid="pool-label"]')].map((e) => e.textContent)), ["Pool 1 · 1.0 m", "Pool 2 · 2.5 m", "Pool 3 · 4.0 m"]);
    assert.match(await text(b, "legend-structures"), /weir and lock: placeholder model, not a design/);
    assert.equal(await exists(b, "legend-pool-levels"), true);
    assert.equal(await exists(b, "legend-pool-water"), false);
    // The blocks are map shapes over the design line; in the flat view the 3D blocks are hidden.
    let state = await mapState();
    assert.ok(state.order[4] > state.order[3] && state.order[3] > state.order[2], `the weirs are drawn over the design: ${state.order}`);
    assert.equal(state.blocks, "none");
    assert.equal(await b.execute(() => window.__owdMap.getSource("structures").serialize().data.features.length), list.weirs.reduce((n, w) => n + w.parts.length, 0));
    // With pools and no DEM the pool water says what it waits for.
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box("structures"), { checked: true, disabled: false, row: "Weirs and locks" });
    assert.deepEqual(await box("pool-levels"), { checked: true, disabled: false, row: "Pool levels" });
    assert.deepEqual(await box("pool-water"), { checked: false, disabled: true, row: "Pool water load a DEM" });

    // A weir's card: the levels, the lift and the chamber the placeholder uses.
    await b.execute(() => document.querySelector('[data-testid="weir-marker"][data-weir="1"]').click());
    await until(b, "the weir card", () => exists(b, "weir-card"));
    assert.equal(await text(b, "weir-card-note"), "placeholder model, not a design");
    assert.equal((await text(b, "weir-card-chainage")).replace(/\s+/g, " "), `km ${km(list.weirs[0])} (${Math.round(list.weirs[0].chainage_m)} m)`);
    assert.match((await text(b, "weir-card-upstream")).replace(/\s+/g, " "), /^2\.5 m ?the minimum level this weir holds, pool 2$/);
    assert.match((await text(b, "weir-card-downstream")).replace(/\s+/g, " "), /^1\.0 m ?pool 1$/);
    assert.equal(await text(b, "weir-card-lift"), "1.5 m");
    assert.equal(await text(b, "weir-card-chamber"), `${list.model.chamber_length_m.toFixed(1)} m × ${list.model.chamber_width_m.toFixed(1)} m`);
    assert.equal(list.model.chamber_length_m, 1.2 * list.model.vessel_length_m);
    assert.equal(list.model.chamber_width_m, list.model.vessel_beam_m + 2);
    assert.match(await text(b, "weir-card-rules"), /chamber length = 1\.2 x vessel length; chamber width = vessel beam \+ 2 m; walls, gates and weir 3 m thick; top of walls 1\.5 m above the upstream pool level/);
    assert.match(await text(b, "weir-card-land"), /tick Pool water/);
    await (await tid(b, "weir-card-close")).click();
    await until(b, "the card to close", async () => !(await exists(b, "weir-card")));
    const before = await savedText();

    // The DEM arrives: the pool water can be ticked. Until it is, the map has asked the engine for nothing.
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    await until(b, "the pool water tick to come on", async () => !(await box("pool-water")).disabled);
    const beforeDem = await savedText();
    assert.deepEqual(await box("pool-water"), { checked: false, disabled: false, row: "Pool water" });
    assert.equal(await mapAttr("data-pool-water"), "off");
    assert.deepEqual((await mapState()).sources, []);
    await b.execute(() => {
      window.__mapErrors = [];
      window.__owdMap.on("error", (e) => window.__mapErrors.push(`${e.sourceId ?? ""}: ${e.error?.message ?? e.error}`));
    });
    await (await tid(b, "layer-pool-water")).click();
    await until(b, "the pool water on the map", async () => (await mapAttr("data-pool-water")) === "on", 120000);
    const info = await b.execute(() => window.__owdPlatform.call("pool_water_info"));
    state = await mapState();
    assert.deepEqual(state.sources, ["pool-water", "dem-water"]);
    assert.deepEqual(state.tiles, [`owd-pool://${info.id}/{z}/{x}/{y}`]);
    assert.ok(state.order[1] > state.order[0] && state.order[1] < state.order[2], `the water is over the imagery and under the design: ${state.order}`);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["pool-water"]');
    // Each pool floods ground, and the land outside the river polygon is counted in hectares.
    assert.deepEqual(info.pools.map((p) => [p.number, p.level_m]), [[1, 1], [2, 2.5], [3, 4]]);
    for (const p of info.pools) assert.ok(p.flooded_ha > 1 && p.outside_river_ha >= 0 && p.outside_river_ha < p.flooded_ha, JSON.stringify(p));
    assert.ok(info.pools[2].outside_river_ha > 1, "the highest pool drowns land outside the river");
    assert.equal(info.mask.factor, 1, "a small DEM: a mask cell for each DEM cell");
    await until(b, "the legend", () => exists(b, "legend-pool-water"));
    assert.match(await text(b, "legend-pool-water"), /pool water by depth: 0 m to 10 m and deeper/);
    await (await tid(b, "pool-land-button")).click();
    const rows = await b.execute(() => [...document.querySelectorAll('[data-testid="pool-land-row"]')].map((r) => [...r.children].map((c) => c.textContent)));
    assert.deepEqual(rows, info.pools.map((p) => [String(p.number), p.level_m.toFixed(1), p.flooded_ha.toFixed(1), p.outside_river_ha.toFixed(1)]));
    assert.match(await text(b, "pool-land-table"), /A study figure, not a survey/);
    await (await tid(b, "pool-land-button")).click();
    // The weir card now has the land its pool drowns.
    await b.execute(() => document.querySelector('[data-testid="weir-marker"][data-weir="2"]').click());
    await until(b, "the weir card", () => exists(b, "weir-card"));
    assert.equal((await text(b, "weir-card-land")).replace(/\s+/g, " "), `${info.pools[2].outside_river_ha.toFixed(1)} ha outside the river polygon by pool 3 at 4.0 m`.replace("polygon by", "polygon by"));
    await (await tid(b, "weir-card-close")).click();

    // A water tile is a PNG made in the browser, kept after the first ask; the surface tile too. How long each takes.
    const tile = await b.execute(async (id, mid) => {
      const z = 14;
      const x = Math.floor(((mid[0] + 180) / 360) * 2 ** z);
      const lat = (mid[1] * Math.PI) / 180;
      const y = Math.floor(((1 - Math.log(Math.tan(lat) + 1 / Math.cos(lat)) / Math.PI) / 2) * 2 ** z);
      const time = async (command) => {
        const t0 = performance.now();
        const bytes = await window.__owdPlatform.call(command, { id, z, x, y });
        const first = performance.now() - t0;
        const t1 = performance.now();
        await window.__owdPlatform.call(command, { id, z, x, y });
        return { first, again: performance.now() - t1, length: bytes.length, magic: [...bytes.slice(0, 4)] };
      };
      // A zoom level the map has not asked for, so the first ask makes the tile.
      const water = await time("pool_water_tile");
      const surface = await time("pool_surface_tile");
      const old = await window.__owdPlatform.call("pool_water_tile", { id: id + 7, z, x, y }).then(() => "", (e) => String(e));
      const at = await window.__owdPlatform.call("pool_water_at", { id, lon: mid[0], lat: mid[1] });
      return { water, surface, old, at };
    }, info.id, list.pools[1].mid);
    assert.deepEqual(tile.water.magic, [0x89, 0x50, 0x4e, 0x47]);
    assert.deepEqual(tile.surface.magic, [0x89, 0x50, 0x4e, 0x47]);
    assert.match(tile.old, /out of date/);
    assert.equal(tile.at.pool, 2);
    assert.equal(tile.at.water_level_m, 2.5);
    assert.ok(Math.abs(tile.at.depth_m - (2.5 - tile.at.ground_m)) < 1e-9 && tile.at.depth_m > 0);
    console.log(`pool water in the browser (wasm, SwiftShader run): flood mask ${info.build_ms.toFixed(0)} ms (${info.mask.width} x ${info.mask.height} cells), water tile first ${tile.water.first.toFixed(1)} ms, cached ${tile.water.again.toFixed(1)} ms, surface tile first ${tile.surface.first.toFixed(1)} ms, cached ${tile.surface.again.toFixed(1)} ms`);

    // A click on the water reads it: the level, the ground, the depth and the pool.
    await (await tid(b, "layers-button")).click();
    await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()), 120000);
    const at = await b.execute((p) => {
      const q = window.__owdMap.project(p);
      const r = window.__owdMap.getCanvas().getBoundingClientRect();
      return [Math.round(q.x + r.left), Math.round(q.y + r.top)];
    }, list.pools[1].mid);
    await b.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", x: at[0], y: at[1] }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] }]);
    await until(b, "the read-out", () => exists(b, "pool-water-readout"));
    assert.match(await text(b, "pool-water-readout"), /^water level 2\.5 m, ground -?\d+\.\d m, depth \d+\.\d m, pool 2/);

    // In 3D the map stands on the terrain with the water on it, and the weirs and locks stand up as blocks.
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the water terrain", async () => (await mapAttr("data-water-terrain")) === "on");
    state = await mapState();
    assert.equal(state.terrain, "dem-water");
    assert.equal(state.blocks, "visible");
    await until(b, "the map to settle in 3D", () => b.execute(() => window.__owdMap.loaded() && !window.__owdMap.isMoving()), 180000);
    await until(b, "the water terrain's tiles", () => b.execute(() => window.__owdMap.isSourceLoaded("dem-water")), 120000);
    // The 3D surface at the middle of pool 2 is the pool's level, not the drowned ground under it.
    const surface = await b.execute((p) => window.__owdMap.queryTerrainElevation(p) / window.__owdMap.getTerrain().exaggeration, list.pools[1].mid);
    assert.ok(Math.abs(surface - 2.5) < 0.05, `the water is flat at 2.5 m in 3D: ${surface}`);
    if (SHOTS) await b.saveScreenshot(path.join(SHOTS, "web-pool-water-3d.png"));
    // Without the pool water the map stands on the ground again, and each block's top is at its level: the
    // map measures a block from the drawn terrain at its middle, so top = (terrain there + height) / exaggeration.
    await (await tid(b, "layers-button")).click();
    await (await tid(b, "layer-pool-water")).click();
    await until(b, "the ground terrain", async () => (await mapAttr("data-water-terrain")) === "off");
    assert.equal((await mapState()).terrain, "dem");
    assert.equal(await mapAttr("data-pool-water"), "off");
    assert.deepEqual((await mapState()).sources, []);
    assert.equal(await exists(b, "pool-water-readout"), false);
    await until(b, "the map to settle on the ground", () => b.execute(() => window.__owdMap.loaded() && !window.__owdMap.isMoving()), 180000);
    const tops = () => b.execute(() => {
      const m = window.__owdMap;
      const e = m.getTerrain().exaggeration;
      return m.getSource("structures-3d").serialize().data.features.map((f) => {
        const ring = f.geometry.coordinates[0].slice(0, -1);
        const mid = [ring.reduce((a, p) => a + p[0], 0) / ring.length, ring.reduce((a, p) => a + p[1], 0) / ring.length];
        return { kind: f.properties.kind, top: f.properties.top_m, height: f.properties.height, drawn: (m.queryTerrainElevation(mid) + f.properties.height) / e };
      });
    });
    await until(b, "the blocks to stand at their levels", async () => {
      const t = await tops();
      return t.length > 0 && t.every((x) => Math.abs(x.drawn - x.top) < 0.05);
    }, 120000);
    const blocks = await tops();
    assert.equal(blocks.length, list.weirs.reduce((n, w) => n + w.parts.length, 0));
    assert.deepEqual([...new Set(blocks.filter((x) => x.kind === "weir").map((x) => x.top))], [2.5, 4], "each weir's crest is at its upstream pool level");
    assert.deepEqual([...new Set(blocks.filter((x) => x.kind === "lock_wall").map((x) => x.top))], [4, 5.5], "the walls stand 1.5 m over it");
    assert.deepEqual([...new Set(blocks.filter((x) => x.kind === "lock_chamber").map((x) => x.top))], [1, 2.5], "the chamber is at the lower pool level");
    assert.ok(blocks.every((x) => x.height > 0), "every block stands above the ground");
    assert.deepEqual(await b.execute(() => window.__mapErrors.filter((m) => /^(pool-water|dem-water|dem)/.test(m))), [], "no pool water or terrain tile failed");
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the terrain off", async () => (await mapAttr("data-terrain")) === "off");
    assert.equal((await mapState()).blocks, "none");

    // Unticked, the weirs and the pool labels leave the map and the legend; ticked again they are back.
    await (await tid(b, "layer-structures")).click();
    await (await tid(b, "layer-pool-levels")).click();
    await until(b, "the legend without the weirs", async () => !(await exists(b, "legend-structures")));
    assert.equal(await exists(b, "legend-pool-levels"), false);
    assert.deepEqual(await b.execute(() => [...document.querySelectorAll('[data-testid="weir-marker"], [data-testid="pool-label"]')].map((e) => e.style.display)), ["none", "none", "none", "none", "none"]);
    assert.equal(await b.execute(() => window.__owdMap.getLayoutProperty("structures-weir", "visibility")), "none");
    assert.equal(await mapAttr("data-layers-off"), "structures pool-levels");
    await (await tid(b, "layer-structures")).click();
    await (await tid(b, "layer-pool-levels")).click();
    await until(b, "the weirs in the legend again", () => exists(b, "legend-structures"));
    await (await tid(b, "layer-pool-water")).click();
    await until(b, "the pool water again", async () => (await mapAttr("data-pool-water")) === "on", 120000);

    // Nothing of this is in the design: the saved text is the same, byte for byte.
    const after = await savedText();
    assert.equal(after, beforeDem, "the design file is byte-identical after the weirs, the pool water and the 3D view");
    assert.equal(JSON.stringify(JSON.parse(after).stages), JSON.stringify(JSON.parse(before).stages), "and the stages are as they were before the DEM came");
    assert.doesNotMatch(after, /weir_length|placeholder|pool-water|flooded/);

    // Remembered on this machine. A reload has the design and not the DEM: the pool water tick waits for it.
    await b.refresh();
    await loaded(b);
    await until(b, "the design after a reload", async () => (await text(b, "file-label")).includes("river-pools.owd"));
    await until(b, "the weirs again", async () => (await mapAttr("data-structures")) === "2");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["pool-water"]');
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box("pool-water"), { checked: true, disabled: true, row: "Pool water load a DEM" });
    assert.equal(await mapAttr("data-pool-water"), "off");
    assert.equal(await exists(b, "error"), false);
  });

  it("the channel and its cut (MAP-5): the Channel and Cut extent layers, the cut read-out, Before and After in 3D with shading and contours, all remembered, the design unchanged, and the cross-section drawn to the ground", async () => {
    // The map needs WebGL: headless Chrome gets it from SwiftShader, in software.
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const mapAttr = async (name) => (await tid(b, "plan-map")).getAttribute(name);
    const box = (id) => b.execute((i) => {
      const c = document.querySelector(`[data-testid="layer-${i}"]`);
      return { checked: c.checked, disabled: c.disabled, row: c.closest("label").textContent.replace(/\s+/g, " ").trim() };
    }, id);
    const mapState = () => b.execute(() => {
      const m = window.__owdMap;
      const t = m.getTerrain();
      const sources = m.getStyle().sources;
      return {
        terrain: t ? t.source : null,
        sources: ["dem", "dem-shade", "dem-after", "dem-after-shade", "cut", "contours"].filter((id) => m.getSource(id)),
        afterTiles: sources["dem-after"]?.tiles?.[0] ?? null,
        cutTiles: sources.cut?.tiles?.[0] ?? null,
        contourTiles: sources.contours?.tiles?.[0] ?? null,
        shadeFrom: m.getLayer("terrain-shading")?.source ?? null,
        layers: m.getStyle().layers.map((l) => l.id).filter((id) => /^(channel|cut)/.test(id)),
        channelVisible: m.getLayer("channel-bed") ? m.getLayoutProperty("channel-bed", "visibility") !== "none" : false,
        bed: m.getSource("channel-bed") ? m.getStyle().sources["channel-bed"].data.features.length : 0,
        daylight: m.getSource("cut-daylight") ? m.getStyle().sources["cut-daylight"].data.features.length : 0,
      };
    });
    /** Tick a layer on or off. A floating panel may lie over the list after a station pick: the tick is pressed in
     * the page, with a click event (`pick` below turns `click()` off for inputs). */
    const tick = (id) => b.execute((i) => void document.querySelector(`[data-testid="layer-${i}"]`).dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })), id);
    const savedText = async () => {
      await b.execute(() => window.__owdPlatform.call("snapshot"));
      await new Promise((r) => setTimeout(r, 300));
      return b.execute(() => JSON.parse(localStorage.getItem("owd.lastDesign")).text);
    };
    await until(b, "the map", () => b.execute(() => !!window.__owdMap));

    // No design: both ticks are there, greyed, and say why. The channel starts on and the cut extent off.
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box("channel"), { checked: true, disabled: true, row: "Channel no design line yet" });
    assert.deepEqual(await box("cut"), { checked: false, disabled: true, row: "Cut extent load the DEM first" });
    assert.equal(await exists(b, "terrain-state"), false);
    assert.equal(await exists(b, "legend-channel"), false);

    // The example has a design line, a size and a water level: the channel is drawn, as a band and its edges.
    await (await tid(b, "open-example")).click();
    await until(b, "the example", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the channel on the map", async () => (await mapAttr("data-channel")) !== "");
    assert.deepEqual(await box("channel"), { checked: true, disabled: false, row: "Channel" });
    let m = await mapState();
    assert.ok(m.bed >= 1 && m.channelVisible, JSON.stringify(m));
    assert.deepEqual(m.layers, ["channel-bed", "channel-bed-casing", "channel-bed-edges", "channel-water", "cut-daylight-casing", "cut-daylight"]);
    assert.equal(await exists(b, "legend-channel"), true);
    assert.equal(await exists(b, "legend-channel-water"), true);
    const first = await b.execute(() => window.__owdPlatform.call("channel_geometry"));
    assert.ok(first.geometry.bed.left[0].length === first.geometry.stations && first.geometry.water_surface.polygons.length >= 1);
    // The example's volumes record was saved without its stations: there is no cut to show until the volumes run again.
    assert.equal(first.geometry.daylight, null);
    assert.deepEqual(await box("cut"), { checked: false, disabled: true, row: "Cut extent run the volumes first" });
    assert.equal((await b.execute(() => window.__owdPlatform.call("terrain_info"))).cut, null);

    // Run the volumes: the stations are stored, and the cut is there.
    await (await tid(b, "run-volumes")).click();
    await until(b, "the cut", async () => !(await box("cut")).disabled, 180000);
    const info = await b.execute(() => window.__owdPlatform.call("terrain_info"));
    assert.ok(Number.isInteger(info.cut), JSON.stringify(info));
    const view = await b.execute(() => window.__owdPlatform.call("channel_geometry"));
    assert.ok(view.geometry.from_volumes && view.geometry.daylight.polygons.length >= 1);
    assert.equal(view.legend.length, 6);
    assert.notEqual(view.key, first.key);
    await until(b, "the daylight lines on the map", async () => (await mapState()).daylight >= 2);
    const before = await savedText();
    await b.execute(() => {
      window.__mapErrors = [];
      window.__owdMap.on("error", (e) => window.__mapErrors.push(`${e.sourceId ?? ""}: ${e.error?.message ?? e.error}`));
    });

    // The cut extent: a picture of the depth of cut, made by the engine one tile at a time, with a key.
    assert.equal(await mapAttr("data-cut"), "off");
    assert.equal(await exists(b, "legend-cut"), false);
    await tick("cut");
    await until(b, "the cut-depth layer", async () => (await mapAttr("data-cut")) === "on");
    m = await mapState();
    assert.match(m.cutTiles, /^owd-cut:\/\/\d+\/\d+\/\{z\}\/\{x\}\/\{y\}$/);
    assert.ok(m.layers.includes("cut-depth"));
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["cut"]');
    assert.equal(await exists(b, "legend-cut"), true);
    assert.equal(await exists(b, "legend-cut-daylight"), true);
    assert.equal((await b.$$('[data-testid="legend-cut-dredge"] .swatch')).length, 6);
    assert.equal((await b.$$('[data-testid="legend-cut-dry"] .swatch')).length, 6);

    // A point of the cut, found on the engine's own geometry: between the bed edge and the daylight line.
    const point = await b.execute(async (g) => {
      const day = g.daylight, n = day.left[0].length;
      let best = null;
      for (let i = 2; i < n - 2; i += 1) {
        for (const t of [0.5, 0.25, 0.75, 0.1, 0.9]) {
          const lon = day.left[0][i][0] + (day.right[0][i][0] - day.left[0][i][0]) * t;
          const lat = day.left[0][i][1] + (day.right[0][i][1] - day.left[0][i][1]) * t;
          const c = await window.__owdPlatform.call("cut_at", { lon, lat });
          if (c && (!best || c.depth_m > best.c.depth_m)) best = { lon, lat, c };
        }
      }
      return best;
    }, view.geometry);
    assert.ok(point, "some point between the daylight lines is cut");
    const c = point.c;
    assert.ok(c.depth_m > 0 && Math.abs(c.ground_m - c.after_m - c.depth_m) < 1e-6, JSON.stringify(c));
    assert.ok(c.kind === "dredge" || c.kind === "dry");
    assert.equal(await b.execute(async () => (await window.__owdPlatform.call("cut_at", { lon: 0, lat: 0 }))), null);
    console.log(`the deepest cut found: ${c.depth_m.toFixed(2)} m (${c.kind}) at chainage ${c.chainage_m.toFixed(0)} m, ground ${c.ground_m.toFixed(2)} m, after ${c.after_m.toFixed(2)} m`);

    // The engine's tiles there: the cut-depth picture has colour, and the After ground is the Before ground cut down.
    const tiles = await b.execute(async (lon, lat, zoom) => {
      const k = 2 ** zoom;
      const x = Math.floor(((lon + 180) / 360) * k);
      const sin = Math.sin((lat * Math.PI) / 180);
      const y = Math.floor((0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * k);
      const pixels = async (png) => {
        const bitmap = await createImageBitmap(new Blob([png], { type: "image/png" }), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        ctx.drawImage(bitmap, 0, 0);
        return ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      };
      const heights = (px) => Array.from({ length: px.length / 4 }, (_, i) => px[4 * i] * 256 + px[4 * i + 1] + px[4 * i + 2] / 256 - 32768);
      const t0 = performance.now();
      const afterPng = await window.__owdPlatform.call("terrain_tile", { z: zoom, x, y, state: "after" });
      const afterMs = performance.now() - t0;
      const t1 = performance.now();
      const cutPng = await window.__owdPlatform.call("cut_tile", { z: zoom, x, y });
      const cutMs = performance.now() - t1;
      const was = heights(await pixels(await window.__owdPlatform.call("terrain_tile", { z: zoom, x, y })));
      const now = heights(await pixels(afterPng));
      const cut = await pixels(cutPng);
      let lower = 0, higher = 0, deepest = 0, coloured = 0, clear = 0;
      for (let i = 0; i < was.length; i += 1) {
        if (now[i] < was[i] - 0.01) lower += 1;
        if (now[i] > was[i] + 0.01) higher += 1;
        deepest = Math.max(deepest, was[i] - now[i]);
        if (cut[4 * i + 3] === 255) coloured += 1;
        else if (cut[4 * i + 3] === 0) clear += 1;
      }
      const far = await window.__owdPlatform.call("cut_tile", { z: 3, x: 0, y: 0 });
      let refused = "";
      try {
        await window.__owdPlatform.call("terrain_tile", { z: zoom, x, y, state: "pool" });
      } catch (e) {
        refused = String(e);
      }
      return { lower, higher, deepest, coloured, clear, total: was.length, far: far.length, refused, afterMs, cutMs };
    }, point.lon, point.lat, info.max_zoom);
    console.log(`After terrain tile in the browser (wasm, SwiftShader run): ${tiles.afterMs.toFixed(1)} ms; cut-depth tile: ${tiles.cutMs.toFixed(1)} ms; ${tiles.lower} of ${tiles.total} pixels cut, the deepest ${tiles.deepest.toFixed(2)} m`);
    assert.ok(tiles.lower > 0 && tiles.higher === 0, `the After ground is never above the Before ground: ${JSON.stringify(tiles)}`);
    assert.ok(tiles.coloured > 0 && tiles.coloured + tiles.clear === tiles.total, `the picture is coloured where the cut is and clear elsewhere: ${JSON.stringify(tiles)}`);
    assert.ok(Math.abs(tiles.coloured - tiles.lower) <= tiles.total * 0.02, `the picture covers the cut: ${JSON.stringify(tiles)}`);
    assert.equal(tiles.far, 0, "a tile with no cut has no bytes");
    assert.match(tiles.refused, /no terrain state `pool`/);

    // A click on the cut reads the point out.
    const at = await b.execute(async (lon, lat) => {
      const map = window.__owdMap;
      map.jumpTo({ center: [lon, lat], zoom: 15 });
      await new Promise((r) => map.once("idle", r));
      const p = map.project([lon, lat]);
      const r = map.getCanvas().getBoundingClientRect();
      return { x: Math.round(r.left + p.x), y: Math.round(r.top + p.y) };
    }, point.lon, point.lat);
    await b.action("pointer").move({ x: at.x, y: at.y }).down().up().perform();
    await until(b, "the cut read-out", () => exists(b, "cut-readout"));
    const read = (await text(b, "cut-readout")).replace(/\s+/g, " ");
    for (const word of ["Chainage", "Ground before", "Level after", "Depth of cut", "Kind", "Station", "m²"]) assert.ok(read.includes(word), `${word} in: ${read}`);
    assert.match(read, new RegExp(c.kind === "dredge" ? "dredge \\(in the river\\)" : "dry cut"));
    await (await tid(b, "cut-readout-close")).click();
    assert.equal(await exists(b, "cut-readout"), false);

    // 3D: the state control comes with it. Before is the DEM as it is.
    assert.equal(await exists(b, "terrain-state"), false);
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the terrain on the map", async () => (await mapAttr("data-terrain")) === "on");
    const control = await tid(b, "terrain-state");
    assert.equal(await control.getAttribute("role"), "group");
    assert.equal(await control.getAttribute("data-state"), "before");
    m = await mapState();
    assert.equal(m.terrain, "dem");
    assert.deepEqual(m.sources, ["dem", "dem-shade", "dem-after", "dem-after-shade", "cut"]);
    assert.match(m.afterTiles, /^owd-dem:\/\/\d+\/after\/\d+\/\{z\}\/\{x\}\/\{y\}$/);
    const afterButton = await tid(b, "terrain-state-after");
    const beforeButton = await tid(b, "terrain-state-before");
    assert.equal(await afterButton.getTagName(), "button");
    assert.deepEqual([await beforeButton.getAttribute("aria-pressed"), await afterButton.getAttribute("aria-pressed")], ["true", "false"]);
    // After: the map's terrain is the second source, the ground with the channel cut into it.
    await afterButton.click();
    await until(b, "the After ground", async () => (await mapAttr("data-terrain-state")) === "after");
    assert.equal((await mapState()).terrain, "dem-after");
    assert.deepEqual([await beforeButton.getAttribute("aria-pressed"), await afterButton.getAttribute("aria-pressed")], ["false", "true"]);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-terrain-state")), "after");
    await until(b, "the After tiles", () => b.execute(() => window.__owdMap.isSourceLoaded("dem-after")), 120000);
    // By the keyboard: Before with Enter on its button, and Compare shows the other ground while Space is held.
    await b.execute(() => document.querySelector('[data-testid="terrain-state-before"]').focus());
    await b.keys("Enter");
    await until(b, "Before by the keyboard", async () => (await mapAttr("data-terrain-state")) === "before");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-terrain-state")), null);
    await b.execute(() => document.querySelector('[data-testid="terrain-compare"]').focus());
    await b.action("key").down(" ").perform(true);
    await until(b, "the other ground while Compare is held", async () => (await mapAttr("data-terrain-state")) === "after");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-terrain-state")), null, "a look is not a choice");
    await b.action("key").up(" ").perform();
    await until(b, "back when Compare is let go", async () => (await mapAttr("data-terrain-state")) === "before");
    await afterButton.click();
    await until(b, "After again", async () => (await mapAttr("data-terrain-state")) === "after");

    // The shading and the contours follow the state.
    await tick("terrain-shading");
    await until(b, "the shading of the After ground", async () => (await mapState()).shadeFrom === "dem-after-shade");
    await tick("contours");
    await until(b, "the contours of the After ground", async () => /^owd-contour:\/\/\d+\/after\/\d+\/auto\//.test((await mapState()).contourTiles ?? ""));
    await beforeButton.click();
    await until(b, "the shading of the ground as it is", async () => (await mapState()).shadeFrom === "dem-shade");
    assert.match((await mapState()).contourTiles, /^owd-contour:\/\/\d+\/auto\//);
    await afterButton.click();
    await until(b, "After once more", async () => (await mapState()).terrain === "dem-after");
    await until(b, "the map to settle", () => b.execute(() => window.__owdMap.loaded()), 180000);
    if (SHOTS) {
      fs.mkdirSync(SHOTS, { recursive: true });
      await b.saveScreenshot(path.join(SHOTS, "web-cut-after-3d.png"));
    }
    assert.deepEqual(await b.execute(() => window.__mapErrors.filter((e) => /^(dem|cut|contours)/.test(e))), [], "no terrain, cut or contour tile failed");
    // 3D off: the control stays while the shading or the contours are on (they follow the state), and goes with them.
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the terrain off", async () => (await mapAttr("data-terrain")) === "off");
    assert.equal(await exists(b, "terrain-state"), true);
    await tick("terrain-shading");
    await tick("contours");
    await until(b, "the control to go", async () => !(await exists(b, "terrain-state")));

    // The cross-section is drawn to the ground: the cut slopes stop at the daylight points.
    // The click on the cut may have picked a station too, which opens the panel.
    if (!(await exists(b, "section-panel"))) await (await tid(b, "section-toggle")).click();
    await until(b, "a cross-section", async () => (await exists(b, "cs-chainage")), 120000);
    const drawn = await b.execute(async () => {
      const svg = document.querySelector('[data-testid="cross-section-svg"]');
      const chainage = Number(document.querySelector('[data-testid="cross-section"]').dataset.chainage);
      const option = (await window.__owdPlatform.call("map_geometry")).options[0].id;
      const s = await window.__owdPlatform.call("cross_section", { option, chainage });
      const finite = (v) => v != null && Number.isFinite(v);
      const marks = [...document.querySelectorAll('[data-testid="cs-daylight"]')].map((c) => ({ offset: Number(c.dataset.offset), level: Number(c.dataset.level) }));
      return { lo: Number(svg.dataset.lo), hi: Number(svg.dataset.hi), ground: Math.max(...s.terrain_m.filter(finite)), template: Math.max(...s.cut_face_m.filter(finite)), water: s.water_level_m, marks, face: document.querySelector('[data-testid="cs-face"]').getAttribute("d").length };
    });
    console.log(`cross-section: the plot runs ${drawn.lo} m to ${drawn.hi} m; the ground tops at ${drawn.ground.toFixed(2)} m, the stored template at ${drawn.template.toFixed(2)} m; ${drawn.marks.length} daylight point(s)`);
    assert.ok(drawn.hi <= Math.max(drawn.ground, drawn.water) + 0.011, `the plot is scaled to the ground, not to the template: ${JSON.stringify(drawn)}`);
    assert.ok(drawn.marks.length <= 2 && drawn.face > 0);
    for (const mark of drawn.marks) assert.ok(mark.level <= drawn.ground + 0.011, "a daylight point is on the ground");

    // Nothing of this is in the design: the saved text is the same, byte for byte.
    const after = await savedText();
    assert.equal(after, before, "the design file is byte-identical after the layers, the read-out and Before and After");
    assert.doesNotMatch(after, /owd-cut|owd-map-terrain-state|dem-after|daylight/);

    // Remembered on this machine. A reload has the design and not the DEM: the cut waits for it.
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-terrain-state")), "after");
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["cut"]');
    await b.refresh();
    await loaded(b);
    await until(b, "the example after a reload", async () => (await text(b, "file-label")).includes("river-example.owd"));
    await until(b, "the map again", () => b.execute(() => !!window.__owdMap));
    await until(b, "the channel again", async () => (await mapAttr("data-channel")) !== "");
    await (await tid(b, "layers-button")).click();
    assert.deepEqual(await box("channel"), { checked: true, disabled: false, row: "Channel" });
    assert.deepEqual(await box("cut"), { checked: true, disabled: true, row: "Cut extent load the DEM first" });
    assert.equal(await mapAttr("data-cut"), "off");
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the cut to come back with the DEM", async () => (await mapAttr("data-cut")) === "on", 60000);
    assert.equal(await mapAttr("data-terrain-state"), "after", "the state is remembered");
    // The channel tick off: its layers go, and the legend with them.
    await tick("channel");
    await until(b, "the channel to go", async () => !(await mapState()).channelVisible);
    assert.equal(await exists(b, "legend-channel"), false);
    assert.equal(await b.execute(() => localStorage.getItem("owd-map-layers")), '["channel","cut"]');
    assert.equal(await exists(b, "error"), false);
  });

  it("After with the pool water (MAP-4 with MAP-5): the 3D view stands on the cut ground with the cut channel filled to the pool level", async () => {
    // The synthetic example with three pools, as the weirs test makes it.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-web-cut-pools-"));
    const doc = path.join(dir, "river-pools.owd");
    fs.copyFileSync(path.join(FIXTURES, "river-example.owd"), doc);
    fs.writeFileSync(path.join(dir, "pools.csv"), "chainage_m,level_m\n0,1.0\n1000,2.5\n2000,4.0\n");
    execFileSync(CLI, ["import", doc, "--water-pools", path.join(dir, "pools.csv")], { encoding: "utf8" });
    const b = await start({ args: ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--window-size=1500,950"] });
    await b.url(`${origin}/owd/app`);
    await loaded(b);
    const mapAttr = async (name) => (await tid(b, "plan-map")).getAttribute(name);
    const tick = (id) => b.execute((i) => void document.querySelector(`[data-testid="layer-${i}"]`).dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })), id);
    const mapState = () => b.execute(() => {
      const m = window.__owdMap;
      return { terrain: m.getTerrain()?.source ?? null, water: m.getStyle().sources["dem-water"]?.tiles?.[0] ?? null };
    });
    await until(b, "the map", () => b.execute(() => !!window.__owdMap));
    await pick(b, async () => (await openButton(b)).click(), doc);
    await until(b, "the file name", async () => (await text(b, "file-label")).includes("river-pools.owd"));
    await pick(b, async () => (await tid(b, "load-dem")).click(), path.join(FIXTURES, "river-dem.tif"));
    await until(b, "the DEM", async () => (await text(b, "data-dem")).includes("river-dem.tif"));
    // The pools changed the water level: the volumes run again, and the cut is there.
    await (await tid(b, "run-volumes")).click();
    await until(b, "the cut", async () => Number.isInteger((await b.execute(() => window.__owdPlatform.call("terrain_info")))?.cut), 180000);
    await (await tid(b, "layers-button")).click();
    await until(b, "the pool water tick", async () => !(await b.execute(() => document.querySelector('[data-testid="layer-pool-water"]').disabled)));
    await tick("pool-water");
    await until(b, "the pool water on the map", async () => (await mapAttr("data-pool-water")) === "on", 120000);
    await (await tid(b, "terrain-3d")).click();
    await until(b, "the water terrain", async () => (await mapAttr("data-water-terrain")) === "on");
    // Before: the water on the ground as it is.
    assert.match((await mapState()).water, /^owd-pool-dem:\/\/\d+\/\{z\}\/\{x\}\/\{y\}$/);
    // After: the water on the cut ground, with the cut filled.
    await (await tid(b, "terrain-state-after")).click();
    await until(b, "the water on the After ground", async () => /^owd-pool-dem:\/\/\d+\/after\/\d+\//.test((await mapState()).water ?? ""));
    assert.equal((await mapState()).terrain, "dem-water");
    assert.equal(await mapAttr("data-terrain-state"), "after");
    // The engine's tiles at the deepest cut: the ground as it is; the cut ground; the cut ground with the water on it.
    const tiles = await b.execute(async () => {
      const p = window.__owdPlatform;
      const g = (await p.call("channel_geometry")).geometry;
      const info = await p.call("terrain_info");
      const water = await p.call("pool_water_info");
      // The deepest dry cut on the cut slopes: high ground the channel is cut through. Each bank's daylight line
      // runs only where that bank is cut, so each daylight point is paired with the nearest point of the
      // water's edge on its own bank, and the slope between them is searched.
      const day = g.daylight;
      const edges = [...g.water_surface.left, ...g.water_surface.right].flat();
      const nearest = (q) => edges.reduce((a, e) => ((e[0] - q[0]) ** 2 + (e[1] - q[1]) ** 2 < (a[0] - q[0]) ** 2 + (a[1] - q[1]) ** 2 ? e : a));
      let best = null;
      for (const q of [...day.left, ...day.right].flat()) {
        const w = nearest(q);
        for (const t of [0.1, 0.25, 0.4, 0.6, 0.75, 0.9]) {
          const lon = q[0] + (w[0] - q[0]) * t;
          const lat = q[1] + (w[1] - q[1]) * t;
          const c = await p.call("cut_at", { lon, lat });
          if (c && c.kind === "dry" && (!best || c.depth_m > best.c.depth_m)) best = { lon, lat, c };
        }
      }
      const zoom = info.max_zoom, k = 2 ** zoom;
      const x = Math.floor(((best.lon + 180) / 360) * k);
      const sin = Math.sin((best.lat * Math.PI) / 180);
      const y = Math.floor((0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * k);
      const heights = async (png) => {
        const bitmap = await createImageBitmap(new Blob([png], { type: "image/png" }), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        ctx.drawImage(bitmap, 0, 0);
        const px = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
        return Array.from({ length: px.length / 4 }, (_, i) => px[4 * i] * 256 + px[4 * i + 1] + px[4 * i + 2] / 256 - 32768);
      };
      const ground = await heights(await p.call("terrain_tile", { z: zoom, x, y }));
      const after = await heights(await p.call("terrain_tile", { z: zoom, x, y, state: "after" }));
      const wet = await heights(await p.call("pool_surface_tile", { id: water.id, z: zoom, x, y }));
      const filled = await heights(await p.call("pool_surface_tile", { id: water.id, z: zoom, x, y, state: "after" }));
      const levels = water.pools.map((q) => q.level_m);
      let below = 0, changed = 0, standing = 0, offLevel = 0, aboveOld = 0, deepest = 0;
      for (let i = 0; i < filled.length; i += 1) {
        if (filled[i] < after[i] - 0.01) below += 1;
        // Where nothing is cut, the water is as on the ground as it is.
        if (Math.abs(after[i] - ground[i]) <= 0.01 && Math.abs(filled[i] - wet[i]) > 0.01) changed += 1;
        // Dry ground before (no water on it), cut to below the pool level: water stands in the cut.
        if (Math.abs(wet[i] - ground[i]) <= 0.01 && filled[i] > after[i] + 0.01) {
          standing += 1;
          deepest = Math.max(deepest, filled[i] - after[i]);
          if (!levels.some((l) => Math.abs(l - filled[i]) < 0.01)) offLevel += 1;
          if (filled[i] > ground[i] + 0.01) aboveOld += 1;
        }
      }
      return { c: best.c, levels, below, changed, standing, offLevel, aboveOld, deepest };
    });
    console.log(`After with the pool water, in the tile of the deepest dry cut (chainage ${tiles.c.chainage_m.toFixed(0)} m, ${tiles.c.depth_m.toFixed(1)} m deep): water stands in the cut on ${tiles.standing} pixels of ground that was dry, up to ${tiles.deepest.toFixed(2)} m deep, at the pool levels ${tiles.levels.join(", ")} m`);
    assert.ok(tiles.standing > 0, `the cut channel is filled where it goes through dry ground: ${JSON.stringify(tiles)}`);
    // Between the two stations either side of a weir the level runs from one pool's to the next: a few pixels.
    assert.ok(tiles.offLevel <= tiles.standing * 0.05, `the water in the cut is at a pool's level: ${JSON.stringify(tiles)}`);
    // Ground below the pool level that the flood did not reach, and the cut now joins to the pool, is under water too.
    assert.ok(tiles.aboveOld < tiles.standing, `most of the water stands in ground that was above it: ${JSON.stringify(tiles)}`);
    assert.equal(tiles.below, 0, "the water is never below the cut ground");
    assert.equal(tiles.changed, 0, "where nothing is cut, the water is as on the ground as it is");
    assert.equal(await exists(b, "error"), false);
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
    // The floating panels open with their default place: nothing to remember, nothing breaks.
    await (await tid(b, "section-toggle")).click();
    await until(b, "the section panel", async () => (await exists(b, "section-panel")) && (await b.execute(() => parseFloat(document.querySelector("[data-testid=section-panel]").style.width) > 0)));
    assert.equal(await exists(b, "error"), false);
  });
});
