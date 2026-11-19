// The worker host (#414): a worker that traps, errors or stops must never leave a call pending. The
// host is driven here with a stand-in worker, so the test needs no browser. Run: node --test e2e/host.test.mjs
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { Host, isTrap } from "../src/lib/platform/host.ts";

/** A worker the test controls: `sent` is what the page posted, `reply` is what the worker answers. */
class FakeWorker {
  sent = [];
  ended = false;
  onmessage = null;
  onerror = null;
  onmessageerror = null;
  postMessage(m) {
    this.sent.push(m);
  }
  terminate() {
    this.ended = true;
  }
  reply(data) {
    this.onmessage({ data });
  }
}

function host() {
  const worker = new FakeWorker();
  return { worker, host: new Host(() => worker, () => {}, () => {}) };
}

describe("the worker host", () => {
  it("answers a call with the worker's result", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("ping");
    worker.reply({ id: 1, ok: true, result: true });
    assert.equal(await p, true);
  });

  it("an ordinary error rejects that call only; the host stays usable", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("open_text");
    worker.reply({ id: 1, ok: false, error: "not a design" });
    await assert.rejects(p, (e) => e === "not a design");
    const q = h.rpc("ping");
    worker.reply({ id: 2, ok: true, result: true });
    assert.equal(await q, true);
    assert.equal(worker.ended, false);
  });

  it("a worker that throws outside a call rejects every waiting call, and later calls at once", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("run_stage");
    const q = h.rpc("snapshot");
    worker.onerror({ message: "Uncaught RuntimeError: unreachable" });
    for (const w of [p, q]) await assert.rejects(w, (e) => /the web engine stopped: .*unreachable/.test(e));
    await assert.rejects(h.rpc("snapshot"), (e) => /the web engine stopped/.test(e));
    assert.equal(worker.sent.length, 2, "nothing is posted to a dead worker");
  });

  it("an error with no message still says the engine stopped", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("snapshot");
    worker.onerror({ message: "" });
    await assert.rejects(p, (e) => /the web engine stopped/.test(e));
  });

  it("a trap the worker caught (a fatal reply) ends the worker and rejects the call with its message", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("reach_step");
    const q = h.rpc("snapshot");
    worker.reply({ id: 1, ok: false, fatal: true, error: "RangeError: Maximum call stack size exceeded" });
    await assert.rejects(p, (e) => /the web engine stopped: RangeError: Maximum call stack size exceeded/.test(e));
    await assert.rejects(q, (e) => /the web engine stopped/.test(e));
    assert.equal(worker.ended, true);
    await assert.rejects(h.rpc("ping"), (e) => /the web engine stopped/.test(e));
  });

  it("a message the page cannot read rejects the waiting calls", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("snapshot");
    worker.onmessageerror({});
    await assert.rejects(p, (e) => /the web engine stopped/.test(e));
  });

  it("ending the worker on purpose rejects waiting calls with the reason given, as Cancel does", async () => {
    const { worker, host: h } = host();
    const p = h.rpc("run_stage");
    h.end("cancelled");
    await assert.rejects(p, (e) => e === "cancelled");
    assert.equal(worker.ended, true);
  });

  it("a wasm trap and a stack overflow are fatal; an ordinary error is not", () => {
    assert.equal(isTrap(new WebAssembly.RuntimeError("unreachable")), true);
    assert.equal(isTrap(new RangeError("Maximum call stack size exceeded")), true);
    assert.equal(isTrap("not a design"), false);
    assert.equal(isTrap(new Error("no such session method: x")), false);
  });
});
