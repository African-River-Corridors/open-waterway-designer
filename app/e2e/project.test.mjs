// The project folder's small interface (PROJ-1), no browser. Run: node --test e2e/project.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { PROJECT_FILE, STATE_WORDS, allowed, demProgress, forget, handleFolder, itemState, listing, memoryFolder, readAll, remember, remembered } from "../src/lib/project.ts";

/** A stand-in for the browser's directory handle, over nested Maps. */
function fakeDir(name, tree = new Map()) {
  const gone = () => new DOMException("not there", "NotFoundError");
  return {
    kind: "directory",
    name,
    tree,
    async getDirectoryHandle(n, o) {
      if (!tree.has(n)) {
        if (!o?.create) throw gone();
        tree.set(n, new Map());
      }
      if (!(tree.get(n) instanceof Map)) throw new DOMException("a file", "TypeMismatchError");
      return fakeDir(n, tree.get(n));
    },
    async getFileHandle(n, o) {
      if (!tree.has(n)) {
        if (!o?.create) throw gone();
        tree.set(n, "");
      }
      if (tree.get(n) instanceof Map) throw new DOMException("a folder", "TypeMismatchError");
      return {
        kind: "file",
        name: n,
        getFile: async () => new Blob([tree.get(n)]),
        createWritable: async () => {
          let data = "";
          return { write: async (d) => void (data = d), close: async () => void tree.set(n, data) };
        },
      };
    },
    async *entries() {
      for (const [n, v] of tree) yield [n, { kind: v instanceof Map ? "directory" : "file" }];
    },
  };
}

describe("a project folder", () => {
  it("in memory lists, reads and writes by path", async () => {
    const f = memoryFolder("reach-a", { "owd-project.json": "{}", "design/a.owd": "A", "design/b.owd": "B", "terrain/d.tif": new Uint8Array([1, 2, 3]), ".DS_Store": "x" });
    assert.equal(PROJECT_FILE, "owd-project.json");
    assert.deepEqual(await f.list("design"), ["a.owd", "b.owd"]);
    assert.deepEqual(await f.list(""), [".DS_Store", "owd-project.json"]);
    assert.deepEqual(await f.list("exports"), []);
    assert.equal(await (await f.read("design/a.owd")).text(), "A");
    assert.equal(await f.read("design/none.owd"), null);
    await f.write("exports/a.csv", "x,y\n");
    assert.deepEqual(await f.list("exports"), ["a.csv"]);
    assert.deepEqual(await listing(f), { design: ["a.owd", "b.owd"], terrain: ["d.tif"], inputs: [], top: ["owd-project.json"] });
  });

  it("over a browser handle reads what is there, says null for what is not, and makes exports/ on a write", async () => {
    const root = fakeDir("reach-a", new Map([["owd-project.json", "{}"], ["design", new Map([["a.owd", "A"]])]]));
    const f = handleFolder(root);
    assert.equal(f.name, "reach-a");
    assert.equal(f.handle, root);
    assert.deepEqual(await f.list(""), ["owd-project.json"], "folders are not listed as files");
    assert.deepEqual(await f.list("design"), ["a.owd"]);
    assert.deepEqual(await f.list("terrain"), []);
    assert.equal(await (await f.read("design/a.owd")).text(), "A");
    assert.equal(await f.read("terrain/d.tif"), null);
    assert.equal(await f.read("design"), null, "a folder is not a file");
    assert.equal(root.tree.has("exports"), false);
    await f.write("exports/a.csv", "x,y\n");
    assert.equal(root.tree.get("exports").get("a.csv"), "x,y\n");
    await f.write("design/a.owd", "A2");
    assert.equal(await (await f.read("design/a.owd")).text(), "A2");
    // A failure that is not "missing" is not hidden.
    const broken = handleFolder({ ...root, getDirectoryHandle: async () => { throw new DOMException("no", "NotAllowedError"); } });
    await assert.rejects(broken.read("design/a.owd"), { name: "NotAllowedError" });
  });

  it("reads a large file in pieces, with the progress, into one buffer of its size", async () => {
    const bytes = new Uint8Array(3 * 1048576 + 17).map((_, i) => i % 251);
    const seen = [];
    const out = await readAll(new Blob([bytes]), (done, total) => seen.push([done, total]));
    assert.equal(out.length, bytes.length);
    assert.deepEqual(Buffer.from(out), Buffer.from(bytes));
    assert.ok(seen.length >= 1);
    assert.deepEqual(seen.at(-1), [bytes.length, bytes.length]);
    assert.ok(seen.every(([d], i) => i === 0 || d > seen[i - 1][0]), "the progress only grows");
    const lines = [];
    const say = demProgress((l) => lines.push(l));
    for (const mb of [0.2, 0.4, 1, 1.2, 2, 340]) say(mb * 1048576, 340 * 1048576);
    assert.deepEqual(lines, ["Reading the DEM: 0 of 340 MB", "Reading the DEM: 1 of 340 MB", "Reading the DEM: 2 of 340 MB", "Reading the DEM: 340 of 340 MB"]);
    const small = [];
    demProgress((l) => small.push(l))(100, 200);
    assert.deepEqual(small, ["Reading the DEM…"]);
  });

  it("gives each item its state: loaded, found and not loaded, or missing", () => {
    assert.equal(itemState("dem", null, false, false), "none");
    assert.equal(itemState("dem", "terrain/d.tif", false, true), "missing");
    assert.equal(itemState("dem", "terrain/d.tif", true, false), "found");
    assert.equal(itemState("dem", "terrain/d.tif", true, true), "loaded");
    assert.equal(itemState("dem", "terrain/d.tif", true, true, true), "changed");
    assert.equal(itemState("centreline", "inputs/c.geojson", true, false), "found");
    assert.equal(itemState("centreline", "inputs/c.geojson", true, true, true), "loaded", "only the DEM has a hash to differ");
    assert.equal(itemState("water_pools", "inputs/pools.csv", true, true), "set");
    assert.equal(STATE_WORDS.loaded, "loaded");
    assert.equal(STATE_WORDS.found, "found, not loaded");
    assert.equal(STATE_WORDS.missing, "missing");
  });

  it("asks the browser for leave only when told to, and a blocked store remembers nothing without an error", async () => {
    const calls = [];
    const h = (state, after) => ({ queryPermission: async () => state, requestPermission: async () => (calls.push("ask"), after) });
    assert.equal(await allowed(h("granted", "granted"), false), true);
    assert.equal(await allowed(h("prompt", "granted"), false), false);
    assert.deepEqual(calls, []);
    assert.equal(await allowed(h("prompt", "granted"), true), true);
    assert.equal(await allowed(h("prompt", "denied"), true), false);
    assert.equal(await allowed({}, true), true, "a handle with no permission calls is always allowed");
    assert.equal(await allowed({ queryPermission: async () => { throw new Error("x"); } }, true), false);
    // No IndexedDB here (as in a browser that blocks it): nothing is remembered, nothing throws.
    assert.equal(typeof globalThis.indexedDB, "undefined");
    await remember({ handle: {}, title: "T", folder: "f" });
    assert.equal(await remembered(), null);
    await forget();
  });
});
