// The design session, in a Web Worker: a stage runs to its end here without freezing the page. The page
// sends {id, method, args}; the worker answers {id, ok, result} or {id, ok: false, error}, and posts
// {progress} while a stage runs. Cancel is the page ending this worker (wasm cannot unwind out of a stage).
import init, { Session } from "../../../wasm-pkg/owd_web.js";
import { isTrap } from "./host";
import wasmUrl from "../../../wasm-pkg/owd_web_bg.wasm?url";

let session: Session | undefined;
// The handler below is set before any await, so no message sent while the module loads is lost.
const ready = init({ module_or_path: wasmUrl }).then(() => {
  session = new Session();
});

type Methods = Record<string, (...args: unknown[]) => unknown>;

/** The network for the reach steps: a synchronous request, which a Worker may make. `start` is -1 for
 * the whole file. A host that ignores the range sends the whole file, and the range is cut from it. */
function syncFetch(url: string, start: number, len: number): Uint8Array {
  const x = new XMLHttpRequest();
  x.open("GET", url, false);
  x.responseType = "arraybuffer";
  if (start >= 0) x.setRequestHeader("Range", `bytes=${start}-${start + len - 1}`);
  x.send();
  if (x.status !== 200 && x.status !== 206) throw new Error(`HTTP ${x.status} from ${url}`);
  const all = new Uint8Array(x.response as ArrayBuffer);
  return start >= 0 && x.status === 200 ? all.slice(start, start + len) : all;
}

/** A DEM file the worker keeps (COG-1): its size and a synchronous reader of a byte range, which a
 * Worker may make with `FileReaderSync`. The engine reads the blocks it needs through it, so a large
 * sectioned DEM is never loaded whole and the page is never blocked. */
// A Worker's synchronous file reader (not in the DOM types the app is checked against).
declare const FileReaderSync: { new (): { readAsArrayBuffer(blob: Blob): ArrayBuffer } };

function fileReader(blob: Blob): [number, (start: number, len: number) => Uint8Array] {
  const reader = new FileReaderSync();
  return [blob.size, (start, len) => new Uint8Array(reader.readAsArrayBuffer(blob.slice(start, start + len)))];
}

/** The methods whose last argument is a DEM file (or nothing): it is given as its size and reader. */
const FILE_METHODS = new Set(["load_dem_file", "open_text_file", "open_project_file"]);

function withFile(args: unknown[]): unknown[] {
  const last = args[args.length - 1];
  const rest = args.slice(0, -1);
  return last instanceof Blob ? [...rest, ...fileReader(last)] : [...rest, undefined, undefined];
}

/** The methods that reach the network take the fetch as their last argument. */
const NETWORK = new Set(["reach_route", "reach_step", "reach_search", "reach_river", "qa_overlay", "reach_restore"]);

// Progress of a reach step (#320), which the session reports through this hook.
(self as unknown as { owdReachProgress: (done: number, total: number, what: string) => void }).owdReachProgress = (done, total, what) =>
  self.postMessage({ reach: { done, total, what } });

self.onmessage = async (e: MessageEvent<{ id: number; method: string; args: unknown[] }>) => {
  const { id, method, args } = e.data;
  try {
    await ready;
    const s = session as unknown as Methods;
    if (method === "ping") return self.postMessage({ id, ok: true, result: true });
    if (typeof s[method] !== "function") throw new Error(`no such session method: ${method}`);
    const given = method === "run_stage" ? [args[0], args[1], (n: number) => self.postMessage({ progress: n })] : NETWORK.has(method) ? [...args, syncFetch] : FILE_METHODS.has(method) ? withFile(args) : args;
    self.postMessage({ id, ok: true, result: s[method].apply(session, given) });
  } catch (err) {
    // A trap or a stack overflow leaves the wasm session unusable: say so, and the page ends this worker.
    self.postMessage({ id, ok: false, error: String(err), ...(isTrap(err) ? { fatal: true } : {}) });
  }
};
