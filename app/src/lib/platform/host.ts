// The page's side of the design worker (#414): the requests waiting on it, and what happens when it
// stops. A worker that throws, traps (a wasm stack overflow is a trap), cannot be read or ends must
// never leave a call pending: every waiting call is rejected with "the web engine stopped: …", and a
// call made afterwards is rejected at once. The design is not lost: the page saves it as it changes,
// and a reload brings it back.
//
// Written so that Node can run it without a build (type stripping): no parameter properties, no enums.

/** The part of a Worker the host uses; a test gives it a stand-in. */
export interface WorkerLike {
  onmessage: ((e: any) => void) | null;
  onerror: ((e: any) => void) | null;
  onmessageerror: ((e: any) => void) | null;
  postMessage(message: unknown): void;
  terminate(): void;
}

/** Whether a thrown value means the wasm session can no longer be trusted: a trap (unreachable, out of
 * bounds, a Rust panic) or a stack overflow. An ordinary error from the session is a string or an Error. */
export function isTrap(err: unknown): boolean {
  return err instanceof WebAssembly.RuntimeError || err instanceof RangeError;
}

export const STOPPED = "the web engine stopped";

export class Host {
  private worker: WorkerLike;
  private next = 1;
  private dead: string | null = null;
  private waiting = new Map<number, { ok: (v: unknown) => void; fail: (e: unknown) => void }>();
  private onProgress: (n: number) => void;
  private onReach: (p: any) => void;
  constructor(makeWorker: () => WorkerLike, onProgress: (n: number) => void, onReach: (p: any) => void) {
    this.onProgress = onProgress;
    this.onReach = onReach;
    this.worker = makeWorker();
    this.worker.onmessage = (e) => {
      if ("progress" in e.data) return this.onProgress(e.data.progress as number);
      if ("reach" in e.data) return this.onReach(e.data.reach);
      if (e.data.fatal) return this.stop(`${STOPPED}: ${e.data.error}`);
      const w = this.waiting.get(e.data.id);
      this.waiting.delete(e.data.id);
      if (!w) return;
      if (e.data.ok) w.ok(e.data.result);
      else w.fail(e.data.error);
    };
    this.worker.onerror = (e) => this.stop(`${STOPPED}: ${e.message || "no reason was given"}`);
    this.worker.onmessageerror = () => this.stop(`${STOPPED}: it sent a message the page cannot read`);
  }
  rpc<T>(method: string, ...args: unknown[]): Promise<T> {
    if (this.dead) return Promise.reject(this.dead);
    return new Promise<T>((ok, fail) => {
      const id = this.next++;
      this.waiting.set(id, { ok: ok as (v: unknown) => void, fail });
      this.worker.postMessage({ id, method, args });
    });
  }
  /** The worker has failed: end it, and reject every waiting call and every later one. */
  private stop(reason: string) {
    const why = `${reason}. Reload the page to start it again; your design is kept.`;
    this.dead = why;
    this.worker.terminate();
    this.failAll(why);
  }
  private failAll(why: string) {
    for (const w of this.waiting.values()) w.fail(why);
    this.waiting.clear();
  }
  /** End the worker on purpose (Cancel). Every waiting request fails with `why`. */
  end(why: string) {
    this.worker.terminate();
    this.failAll(why);
  }
}
