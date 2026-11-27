// The project folder in the web app (PROJ-1): the built web app in headless Chrome. Run by e2e/run-web.sh.
// Headless Chrome cannot show the native folder dialog, so each test makes a real folder in the Origin
// Private File System (real directory handles, no dialog) and has `showDirectoryPicker` answer with it.
// Everything after the dialog is the app's own path: reading owd-project.json, opening the design and the
// DEM in one step, Save back into the folder, exports into exports/, the folder remembered for Reopen.
// The data is the synthetic web example.
import { after, afterEach, before, describe, it } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { serve } from "./web-server.mjs";
import { buildings, places, settlements } from "./synthetic-places.mjs";

const here = import.meta.dirname;
const FIXTURES = path.resolve(here, "../../examples/web");
const DIST = path.resolve(here, "../dist-web-test");
const DESIGN = fs.readFileSync(path.join(FIXTURES, "river-example.owd"), "utf8");
const DEM = fs.readFileSync(path.join(FIXTURES, "river-dem.tif"));
/** A name no design or export may hold: the folder is never written into either. */
const FOLDER = "zz-project-folder-reach-a";
const POOLS = "chainage_m,level_m\n0,31.5\n600,31.25\n";
const STANDARD = {
  owd_project: 1,
  title: "Reach A screening",
  design: "design/reach-a.owd",
  dem: "terrain/reach-a-dem.tif",
  inputs: { centreline: null, river_polygon: null, left_bank: null, right_bank: null, water_pools: "inputs/pools.csv" },
  notes: "",
  client_ref: "kept as it is",
};

const chrome = (args = []) => ({
  browserName: "chrome",
  "wdio:enforceWebDriverClassic": true,
  "goog:chromeOptions": {
    ...(process.env.CHROME_BINARY ? { binary: process.env.CHROME_BINARY } : fs.existsSync("/Applications/Google Chrome.app") ? { binary: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" } : {}),
    args: ["--headless=new", "--no-sandbox", "--disable-gpu", "--window-size=1500,950", ...args],
  },
});

let origin, server;
const sessions = [];
async function start(args) {
  const b = await remote({ logLevel: "warn", strictSelectors: false, capabilities: chrome(args) });
  sessions.push(b);
  await b.url(`${origin}/owd/app`);
  await until(b, "the app", async () => (await exists(b, "file-label")) && (await exists(b, "web-label")));
  return b;
}
const until = (b, what, check, timeout = 60000) => b.waitUntil(check, { timeout, interval: 150, timeoutMsg: `timed out waiting for: ${what}` });
const tid = (b, id) => b.$(`[data-testid="${id}"]`);
const text = async (b, id) => (await tid(b, id)).getText();
const exists = async (b, id) => (await tid(b, id)).isExisting();
const state = async (b, key) => (await tid(b, `project-item-${key}`)).getAttribute("data-state");
const snapshot = (b) => b.execute(async () => await window.__owdPlatform.call("snapshot"));

/** Make the folder in the Origin Private File System, and have the folder dialog answer with it.
 * `files` maps a path to text, or to `{ base64 }` for bytes. Also notes every progress line shown. */
async function folder(b, files, name = FOLDER) {
  await b.execute(
    async (name, files) => {
      const root = await navigator.storage.getDirectory();
      await root.removeEntry(name, { recursive: true }).catch(() => {});
      const dir = await root.getDirectoryHandle(name, { create: true });
      for (const [p, data] of Object.entries(files)) {
        const names = p.split("/");
        const file = names.pop();
        let at = dir;
        for (const n of names) at = await at.getDirectoryHandle(n, { create: true });
        const w = await (await at.getFileHandle(file, { create: true })).createWritable();
        await w.write(typeof data === "string" ? data : Uint8Array.from(atob(data.base64), (c) => c.charCodeAt(0)));
        // A file made larger with zeros after its end (a TIFF reads the same): a large file without
        // sending its bytes through the driver.
        if (typeof data !== "string" && data.padTo) await w.truncate(data.padTo);
        await w.close();
      }
      window.__pickerAsked = [];
      window.showDirectoryPicker = async (options) => {
        window.__pickerAsked.push(options);
        return dir;
      };
      window.__progress = [];
      new MutationObserver(() => {
        const line = document.querySelector("[data-testid=project-progress]")?.textContent;
        if (line && window.__progress.at(-1) !== line) window.__progress.push(line);
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    },
    name,
    files,
  );
}
/** A file of the folder as text, or null when it is not there. */
const fileText = (b, p, name = FOLDER) =>
  b.execute(
    async (name, p) => {
      try {
        let at = await (await navigator.storage.getDirectory()).getDirectoryHandle(name);
        const names = p.split("/");
        const file = names.pop();
        for (const n of names) at = await at.getDirectoryHandle(n);
        return await (await (await at.getFileHandle(file)).getFile()).text();
      } catch {
        return null;
      }
    },
    name,
    p,
  );
const dem = (bytes = DEM) => ({ base64: Buffer.from(bytes).toString("base64") });
const standardFiles = (extra = {}) => ({
  "owd-project.json": JSON.stringify(STANDARD, null, 2),
  "design/reach-a.owd": DESIGN,
  "terrain/reach-a-dem.tif": dem(),
  "inputs/pools.csv": POOLS,
  ...extra,
});
const openProject = async (b) => (await tid(b, "open-project")).click();
const opened = (b) => until(b, "the project's design", async () => (await text(b, "file-label")).includes("reach-a.owd") && (await exists(b, "project-panel")));

before(async () => {
  ({ server, origin } = await serve(DIST));
});
// Close each test's browsers when the test ends, so a file never holds one driver and one Chrome
// per test run so far (29 at once in web-smoke on 27 Nov), and a killed file leaves none behind.
afterEach(async () => {
  while (sessions.length) await sessions.pop().deleteSession().catch(() => {});
});
after(async () => {
  for (const b of sessions) await b.deleteSession().catch(() => {});
  server?.close();
});

describe("OWD web: the project folder", () => {
  it("Open project loads the design and its DEM in one step; Save writes back into the folder; exports land in exports/", async () => {
    const b = await start();
    assert.equal(await text(b, "open-project"), "Open project");
    await folder(b, standardFiles());
    await openProject(b);
    await opened(b);
    assert.deepEqual(await b.execute(() => window.__pickerAsked), [{ mode: "readwrite", id: "owd-project" }], "the folder is asked for once, to read and write");

    // One step: the design is open and its DEM is in memory, with no Load… and no warning.
    const snap = await snapshot(b);
    assert.equal(snap.file_name, "reach-a.owd");
    assert.equal(snap.document.project.title, JSON.parse(DESIGN).project.title);
    assert.deepEqual(snap.warnings, [], "the DEM is the one the design recorded");
    assert.equal(snap.stages.map((s) => `${s.name}:${s.status}`).join(" "), "sizing:fresh alignment:fresh optimise:not_run volumes:fresh");
    assert.notEqual(await b.execute(async () => await window.__owdPlatform.call("terrain_info")), null, "the DEM is in memory");
    const progress = await b.execute(() => window.__progress);
    // The DEM is not read whole first (COG-1): the worker keeps the file and reads what it needs.
    assert.ok(progress.some((l) => /Loading the design and the DEM/.test(l)), `a progress line for the load: ${JSON.stringify(progress)}`);
    assert.ok(!progress.some((l) => /Reading the DEM: .* of .* MB/.test(l)), `no read-the-whole-file progress: ${JSON.stringify(progress)}`);
    assert.equal(await exists(b, "project-progress"), false, "the progress line goes when the project is open");
    assert.equal(await exists(b, "error"), false);

    // The Project panel: the title, and each listed item with its state.
    assert.equal(await text(b, "project-title"), "Reach A screening");
    assert.match(await text(b, "project-folder"), new RegExp(FOLDER));
    assert.equal(await state(b, "design"), "loaded");
    assert.equal(await state(b, "dem"), "loaded");
    assert.equal(await state(b, "centreline"), "none");
    assert.match(await text(b, "project-state-dem"), /^loaded$/);

    // The design names its DEM from its own folder, as the command line reads it: it said `river-dem.tif`
    // (beside itself), so it is pointed at the project's and is unsaved until Save.
    assert.equal(snap.document.inputs.dem.path, "../terrain/reach-a-dem.tif");
    assert.match(await text(b, "file-label"), /unsaved/);
    assert.match(await text(b, "notice"), /named its DEM as `river-dem\.tif`.*`\.\.\/terrain\/reach-a-dem\.tif`.*Save/);

    // Save writes back to the design file in the folder, with no dialog.
    await (await tid(b, "save")).click();
    await until(b, "the save notice", async () => (await exists(b, "notice")) && /Saved design\/reach-a\.owd in the project folder/.test(await text(b, "notice")));
    assert.doesNotMatch(await text(b, "file-label"), /unsaved/);
    const saved = await fileText(b, "design/reach-a.owd");
    assert.equal(JSON.parse(saved).inputs.dem.path, "../terrain/reach-a-dem.tif");
    assert.equal(JSON.parse(saved).inputs.dem.sha256, JSON.parse(DESIGN).inputs.dem.sha256);
    // Nothing about the folder goes into the design file.
    assert.equal(saved.includes(FOLDER), false, "the design does not name the project folder");
    assert.equal(saved.includes("owd-project"), false);
    // The text file is the user's: opening and saving do not rewrite it.
    assert.equal(await fileText(b, "owd-project.json"), JSON.stringify(STANDARD, null, 2));

    // An export lands in exports/ (made here: the folder had none), and one notice says where.
    assert.equal(await fileText(b, "exports/reach-a.csv"), null);
    await (await b.$("button=Export CSV")).click();
    await until(b, "the CSV notice", async () => (await exists(b, "notice")) && /Exported to exports\/reach-a\.csv in the project folder/.test(await text(b, "notice")));
    const csv = await fileText(b, "exports/reach-a.csv");
    assert.ok(csv && csv.split("\n").length > 2, "the CSV is in exports/");
    await (await b.$("button=Export GeoJSON")).click();
    await until(b, "the GeoJSON notice", async () => /Exported to exports\/reach-a\.geojson in the project folder/.test(await text(b, "notice")));
    const geojson = await fileText(b, "exports/reach-a.geojson");
    assert.equal(JSON.parse(geojson).type, "FeatureCollection");
    for (const out of [csv, geojson]) assert.equal(out.includes(FOLDER), false, "an export does not name the project folder");
    // The sheets go there too.
    await (await tid(b, "sheets-toggle")).click();
    await until(b, "the sheets", async () => (await exists(b, "export-sheets")) && (await tid(b, "export-sheets")).isEnabled());
    await (await tid(b, "export-sheets")).click();
    await until(b, "the sheets notice", async () => /Sheets exported as PDF: \d+ PDFs? in exports\/ in the project folder/.test(await text(b, "notice")), 240000);
    const pdfs = await b.execute(async (name) => {
      const dir = await (await (await navigator.storage.getDirectory()).getDirectoryHandle(name)).getDirectoryHandle("exports");
      const names = [];
      for await (const [n] of dir.entries()) names.push(n);
      return names.filter((n) => n.endsWith(".pdf"));
    }, FOLDER);
    assert.ok(pdfs.length >= 1, `the sheet PDFs are in exports/: ${pdfs}`);
    assert.equal(await exists(b, "error"), false);
  });

  it("a folder with no text file shows what it found and offers to write owd-project.json", async () => {
    const b = await start();
    const files = standardFiles();
    delete files["owd-project.json"];
    await folder(b, files);
    await openProject(b);
    await until(b, "the dialog", async () => exists(b, "project-dialog"));
    assert.equal(await text(b, "project-found-design"), "design/reach-a.owd");
    assert.equal(await text(b, "project-found-dem"), "terrain/reach-a-dem.tif");
    assert.equal(await text(b, "project-found-water_pools"), "inputs/pools.csv");
    assert.equal(await text(b, "project-adopt"), "Use these and write owd-project.json");
    assert.equal(await fileText(b, "owd-project.json"), null, "nothing is written before the user says so");
    await (await tid(b, "project-adopt")).click();
    await opened(b);
    const written = JSON.parse(await fileText(b, "owd-project.json"));
    assert.deepEqual(written, {
      owd_project: 1,
      title: FOLDER,
      design: "design/reach-a.owd",
      dem: "terrain/reach-a-dem.tif",
      inputs: { centreline: null, river_polygon: null, left_bank: null, right_bank: null, water_pools: "inputs/pools.csv" },
      notes: "",
    });
    assert.equal(await state(b, "dem"), "loaded");
    assert.match(await text(b, "notice"), /Wrote owd-project\.json/);
    assert.deepEqual((await snapshot(b)).warnings, []);
    assert.equal(await exists(b, "error"), false);

    // A listed input that is in the folder loads through the panel, by the same call as the import.
    assert.equal(await state(b, "water_pools"), "set");
    await (await tid(b, "project-load-water_pools")).click();
    await until(b, "the pool levels", async () => JSON.stringify((await snapshot(b)).document.inputs.water_level?.points?.[0]) === "[0,31.5]");
  });

  it("more than one design: the user picks; nothing found: a plain line says what a project folder holds", async () => {
    const b = await start();
    await folder(b, { "design/reach-a.owd": DESIGN, "design/reach-b.owd": DESIGN, "terrain/reach-a-dem.tif": dem() });
    await openProject(b);
    await until(b, "the dialog", async () => exists(b, "project-dialog"));
    assert.equal(await (await tid(b, "project-adopt")).isEnabled(), false, "nothing to use until a design is picked");
    await (await tid(b, "project-pick-design")).selectByVisibleText("design/reach-b.owd");
    await until(b, "the button", async () => (await tid(b, "project-adopt")).isEnabled());
    await (await tid(b, "project-adopt")).click();
    await until(b, "the picked design", async () => (await text(b, "file-label")).includes("reach-b.owd") && (await exists(b, "project-panel")));
    assert.equal(JSON.parse(await fileText(b, "owd-project.json")).design, "design/reach-b.owd");
    // Its DEM was pointed at the project's, so it is unsaved: save it, or the next Open project asks first.
    await (await tid(b, "save")).click();
    await until(b, "saved", async () => !/unsaved/.test(await text(b, "file-label")));
    assert.equal(JSON.parse(await fileText(b, "design/reach-b.owd")).inputs.dem.path, "../terrain/reach-a-dem.tif");

    // A folder with nothing OWD knows: a plain line, and nothing is written or opened.
    await folder(b, { "readme.txt": "hello" }, "zz-empty");
    await openProject(b);
    await until(b, "the dialog", async () => exists(b, "project-nothing"));
    assert.match(await text(b, "project-nothing"), /no design file \(\.owd\) and no DEM \(\.tif\).*owd-project\.json.*design\/.*terrain\/.*inputs\/.*exports\//s);
    assert.equal(await exists(b, "project-adopt"), false);
    await (await tid(b, "project-dialog-close")).click();
    assert.equal(await fileText(b, "owd-project.json", "zz-empty"), null);
    assert.match(await text(b, "file-label"), /reach-b\.owd/, "the open design stays");
  });


  it("buildings, places and settlements the project names are loaded by Open project, said in one calm line, and checked against their hash after a save (BLD-1)", async () => {
    const b = await start();
    const project = { ...STANDARD, inputs: { ...STANDARD.inputs, buildings: "inputs/buildings.geojson", points_of_interest: "inputs/points-of-interest.geojson", settlements: "inputs/settlements.geojson" } };
    const files = standardFiles({
      "owd-project.json": JSON.stringify(project, null, 2),
      "inputs/buildings.geojson": JSON.stringify(buildings(50, { reach: 300 })),
      "inputs/points-of-interest.geojson": JSON.stringify(places()),
      "inputs/settlements.geojson": JSON.stringify(settlements()),
    });
    await folder(b, files);
    await openProject(b);
    await opened(b);
    // One calm line however many inputs, with no warning mark; the design is unsaved.
    await until(b, "the added line", () => exists(b, "project-added"));
    assert.equal(await text(b, "project-added").then((t) => t.replace(/Dismiss$/, "").trim()), "The project added 3 inputs to the design (buildings, places, settlements). Save to keep them.");
    assert.equal(await b.execute(() => document.querySelector('[data-testid="project-added"]').className.includes("warn")), false);
    assert.match(await text(b, "file-label"), /unsaved/);
    for (const k of ["buildings", "points_of_interest", "settlements"]) assert.equal(await state(b, k), "loaded", k);
    const snap = await snapshot(b);
    assert.equal(snap.document.inputs.buildings.path, "../inputs/buildings.geojson");
    assert.deepEqual(snap.warnings, []);
    // Saved, then opened again: the design names them, so they are attached, and no line is said.
    await (await tid(b, "save")).click();
    await until(b, "the save", async () => !/unsaved/.test(await text(b, "file-label")));
    assert.equal(await exists(b, "project-added"), false);
    await openProject(b);
    await opened(b);
    await until(b, "the inputs loaded again", async () => (await state(b, "buildings")) === "loaded");
    assert.equal(await exists(b, "project-added"), false);
    assert.doesNotMatch(await text(b, "file-label"), /unsaved/);
  });

  it("a missing DEM shows as missing; another DEM than the design recorded shows the warning and can be taken", async () => {
    const b = await start();
    const files = standardFiles();
    delete files["terrain/reach-a-dem.tif"];
    await folder(b, files);
    await openProject(b);
    await opened(b);
    assert.equal(await state(b, "dem"), "missing");
    assert.equal(await text(b, "project-state-dem"), "missing");
    assert.equal(await exists(b, "project-load-dem"), false, "nothing to load");
    assert.equal(await state(b, "design"), "loaded");
    assert.ok((await snapshot(b)).warnings.some((w) => /DEM .* is missing/.test(w)), "the design's own warning shows too");
    assert.equal(await b.execute(async () => await window.__owdPlatform.call("terrain_info")), null);

    // The same name, another file: the existing warning, the item says so, and the design is not changed.
    const other = Buffer.from(DEM);
    other[other.length - 1] ^= 0xff;
    await folder(b, standardFiles({ "terrain/reach-a-dem.tif": dem(other) }));
    await openProject(b);
    await until(b, "the changed DEM", async () => (await exists(b, "project-panel")) && (await state(b, "dem")) === "changed");
    const snap = await snapshot(b);
    assert.ok(snap.warnings.some((w) => /has changed since its hash was recorded/.test(w)), JSON.stringify(snap.warnings));
    assert.match(await text(b, "warning"), /DEM/);
    assert.equal(snap.document.inputs.dem.path, "river-dem.tif", "the design is left as it was");
    assert.equal(snap.dirty, false);
    // "Use this DEM" records it, as Load… does.
    await (await tid(b, "project-load-dem")).click();
    await until(b, "the DEM taken", async () => (await state(b, "dem")) === "loaded");
    const taken = await snapshot(b);
    assert.equal(taken.document.inputs.dem.path, "../terrain/reach-a-dem.tif");
    assert.notEqual(taken.document.inputs.dem.sha256, JSON.parse(DESIGN).inputs.dem.sha256);
    assert.deepEqual(taken.warnings, []);
  });

  it("a text file OWD cannot read is refused in plain words, and the open design stays", async () => {
    const b = await start();
    await folder(b, standardFiles({ "owd-project.json": JSON.stringify({ ...STANDARD, dem: "../elsewhere/dem.tif" }) }));
    await openProject(b);
    await until(b, "the refusal", async () => exists(b, "error"));
    assert.match(await text(b, "error"), /`dem` is `\.\.\/elsewhere\/dem\.tif`: a path may not leave the project folder/);
    assert.equal(await exists(b, "project-panel"), false);
    assert.match(await text(b, "file-label"), /Untitled/);
  });

  it("remembers the folder and offers Reopen on the next start", async () => {
    const b = await start();
    assert.equal(await exists(b, "reopen-banner"), false, "nothing to offer the first time");
    await folder(b, standardFiles());
    await openProject(b);
    await opened(b);
    await (await tid(b, "save")).click();
    await until(b, "saved", async () => !/unsaved/.test(await text(b, "file-label")));
    // A new start: the folder is offered by its title, and one press opens the design with its DEM again.
    await b.url(`${origin}/owd/app`);
    await until(b, "the offer", async () => exists(b, "reopen-project"));
    assert.equal(await text(b, "reopen-project"), "Reopen Reach A screening");
    assert.equal(await exists(b, "project-panel"), false, "the folder is offered, not opened by itself");
    // Nothing about the folder is in the browser's plain storage: the handle is in IndexedDB.
    assert.equal(await b.execute((name) => JSON.stringify({ ...localStorage }).includes(name), FOLDER), false);
    await (await tid(b, "reopen-project")).click();
    await opened(b);
    assert.equal(await state(b, "dem"), "loaded");
    assert.deepEqual((await snapshot(b)).warnings, []);
    assert.doesNotMatch(await text(b, "file-label"), /unsaved/, "the saved design names its DEM as the project does");
    assert.equal(await exists(b, "reopen-banner"), false);
    // Open (a single file) leaves the project: the panel goes.
    await b.execute(async () => await window.__owdPlatform.call("new_file"));
    await b.url(`${origin}/owd/app`);
    await until(b, "the offer again", async () => exists(b, "reopen-project"));
    await (await tid(b, "reopen-forget")).click();
    // The offer goes when the folder is forgotten (the store has answered).
    await until(b, "the folder forgotten", async () => !(await exists(b, "reopen-banner")));
    await b.url(`${origin}/owd/app`);
    await until(b, "the app", async () => exists(b, "file-label"));
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(await exists(b, "reopen-banner"), false, "a forgotten folder is not offered");
  });

  it("where the browser has no folder dialog the button says so, and Open and Load… stay", async () => {
    // Chrome without the File System Access dialogs is what Firefox and Safari give the page.
    const b = await start(["--disable-blink-features=FileSystemAccessLocal"]);
    assert.equal(await b.execute(() => typeof window.showDirectoryPicker), "undefined");
    assert.equal(await text(b, "open-project"), "Open project (Chrome or Edge only)");
    await (await tid(b, "open-project")).click();
    await until(b, "the plain notice", async () => exists(b, "notice"));
    assert.match(await text(b, "notice"), /This browser cannot open a folder: Chrome and Edge can\. Use Open for the design and Load… for the DEM\./);
    assert.equal(await (await b.$("button=Open")).isExisting(), true);
    assert.equal(await exists(b, "load-dem"), true);
    assert.equal(await exists(b, "error"), false);
  });
});

describe("OWD web: a large sectioned DEM (COG-1)", () => {
  /** The example DEM made sectioned by `owd dem prepare`, then made 80 MiB with zeros after its end, so
   * the app reads it in sections (a tiled file of 64 MiB or more). Same cells as the example DEM. */
  const sectioned = () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "owd-cog1-"));
    const out = path.join(dir, "river-dem-sectioned.tif");
    execFileSync(process.env.OWD_CLI, ["dem", "prepare", path.join(FIXTURES, "river-dem.tif"), "-o", out, "--json"]);
    const bytes = fs.readFileSync(out);
    fs.rmSync(dir, { recursive: true, force: true });
    return { base64: bytes.toString("base64"), padTo: 80 * 1024 * 1024 };
  };
  const call = (b, command, args = {}) => b.execute(async (c, a) => await window.__owdPlatform.call(c, a), command, args);

  it("opens at once, reads in blocks, and gives the volumes, sections, map and contours of the same DEM read whole", async () => {
    const b = await start();
    // The design on its own DEM, read whole: the volumes and a cross-section to compare with.
    await folder(b, standardFiles());
    await openProject(b);
    await opened(b);
    await call(b, "run_stage", { stage: "volumes" });
    const whole = await snapshot(b);
    const option = (await call(b, "map_geometry")).options[0].id;
    const sectionWhole = await call(b, "cross_section", { option, chainage: 300 });

    // The same project with the sectioned DEM: opened in one step, with no read of the whole file.
    await folder(b, standardFiles({ "terrain/reach-a-dem.tif": sectioned() }), `${FOLDER}-cog`);
    // The first design is unsaved (its DEM path was pointed at the project's): discard it.
    await b.execute(() => {
      window.confirm = () => true;
    });
    const t0 = Date.now();
    await openProject(b);
    await until(b, "the sectioned DEM's project", async () => (await snapshot(b)).warnings.some((w) => /has changed since its hash was recorded/.test(w)));
    const openMs = Date.now() - t0;
    assert.ok(openMs < 15000, `opened in ${openMs} ms`);
    const progress = await b.execute(() => window.__progress);
    assert.ok(!progress.some((l) => /Reading the DEM: .* of .* MB/.test(l)), JSON.stringify(progress));
    // Another file than the design recorded: Use this DEM records it, by its quick check.
    let snap = await snapshot(b);
    assert.ok(snap.warnings.some((w) => /has changed since its hash was recorded/.test(w)), JSON.stringify(snap.warnings));
    await call(b, "project_load", { key: "dem" });
    snap = await snapshot(b);
    assert.deepEqual(snap.warnings, []);
    assert.equal(snap.document.inputs.dem.quick_check.method, "owd-quick-1");
    assert.equal(snap.document.inputs.dem.quick_check.length, 80 * 1024 * 1024);
    assert.equal(snap.document.inputs.dem.sha256, undefined);

    // Volumes and a section: the numbers of the DEM read whole (both read full resolution).
    await call(b, "run_stage", { stage: "volumes" });
    snap = await snapshot(b);
    assert.deepEqual(snap.document.stages.volumes.results, whole.document.stages.volumes.results);
    assert.deepEqual(await call(b, "cross_section", { option, chainage: 300 }), sectionWhole);

    // The map: terrain info, a terrain tile, a contour tile and the hillshade, from the blocks.
    const info = await call(b, "terrain_info");
    assert.notEqual(info, null);
    const [w, s, e, n] = info.bounds;
    const z = Math.min(info.max_zoom, 14);
    const lon = (w + e) / 2;
    const lat = (s + n) / 2;
    const x = Math.floor(((lon + 180) / 360) * 2 ** z);
    const y = Math.floor(((1 - Math.log(Math.tan((lat * Math.PI) / 180) + 1 / Math.cos((lat * Math.PI) / 180)) / Math.PI) / 2) * 2 ** z);
    const tile = await call(b, "terrain_tile", { z, x, y });
    assert.ok(Object.keys(tile).length > 100, "a terrain tile");
    const contours = await call(b, "contour_tile", { z, x, y, interval: 1 });
    assert.ok(Object.keys(contours ?? {}).length > 0, "contour lines");
    assert.notEqual(await call(b, "hillshade"), null);
    assert.equal(await exists(b, "error"), false);
  });

  it("a DEM in strips over the terrain limit is refused plainly, naming owd dem prepare", async () => {
    const b = await start();
    await folder(b, standardFiles(), `${FOLDER}-limit`);
    // The limit, set very low, stands for a browser's memory: 0.0001 GiB is about 107 kB, under the
    // 268,800 bytes the example's 33,600 cells need read whole.
    await call(b, "set_terrain_limit", { gib: "0.0001" });
    await openProject(b);
    await opened(b);
    const said = await b.execute(async () => {
      try {
        await window.__owdPlatform.call("project_load", { key: "dem" });
        return "loaded";
      } catch (e) {
        return String(e);
      }
    });
    assert.match(said, /too large/);
    assert.match(said, /strips, not tiles/);
    assert.match(said, /owd dem prepare/);
  });
});
