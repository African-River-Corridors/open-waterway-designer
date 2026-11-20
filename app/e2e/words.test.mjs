// Plain-words helpers (no browser). Run: node --test e2e/words.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { bankValue, fileNameOf, plainWarning, roundedFall, sectionReason } from "../src/lib/words.ts";

const stages = (...s) => s.map((x) => ({ name: x.split(":")[0], status: x.split(":")[1] }));

describe("sectionReason", () => {
  const err = "the volumes stage cannot run: volumes is stale. Run volumes first.";
  it("names the stage that is out of date, once", () => {
    const t = sectionReason(err, stages("sizing:fresh", "alignment:stale", "optimise:not_run", "volumes:stale"));
    assert.equal(t, "No sections yet: the alignment is out of date. Run alignment, then volumes.");
  });
  it("says only volumes when only volumes is stale", () => {
    assert.equal(sectionReason(err, stages("sizing:fresh", "alignment:fresh", "volumes:stale")), "No sections yet: the volumes are out of date. Run volumes.");
  });
  it("leaves another message alone", () => {
    assert.equal(sectionReason("something else", stages("alignment:stale")), "something else");
  });
});

describe("DEM words", () => {
  it("takes the file name from a relative path", () => {
    assert.equal(fileNameOf("../../../cases/x/terrain.tif"), "terrain.tif");
    assert.equal(fileNameOf("C:\\data\\t.tif"), "t.tif");
  });
  it("rewrites the missing-DEM warning with the file name and the Load button", () => {
    const t = plainWarning("the DEM `../../a/terrain.tif` is missing: the volumes may no longer hold");
    assert.match(t, /terrain\.tif/);
    assert.match(t, /Load button/);
    assert.doesNotMatch(t, /\.\.\//);
  });
  it("rewrites the refusal of a stage with no DEM", () => {
    const t = plainWarning("the DEM `../x/terrain.tif` is missing: the volumes stage cannot run without it");
    assert.match(t, /terrain\.tif was not found/);
    assert.match(t, /Load button/);
  });
  it("leaves other warnings alone", () => assert.equal(plainWarning("hello"), "hello"));
});

describe("bankValue", () => {
  const line = { coordinates: [[0, 0], [1, 1]] };
  it("counts positions when loaded", () => assert.equal(bankValue(line, false), "2 positions"));
  it("says not needed when a river polygon stands in", () => assert.equal(bankValue(null, true), "not needed (river polygon loaded)"));
  it("says not loaded when neither is there", () => assert.equal(bankValue(null, false), "not loaded"));
});

describe("roundedFall", () => {
  it("rounds to three significant figures", () => assert.equal(roundedFall("-0.0004925721857999721"), "-0.000493"));
  it("leaves empty alone", () => assert.equal(roundedFall(""), ""));
});
