// A project folder (PROJ-1) as the web app reaches it: a small interface over one folder, so the app
// reads and writes through it and a test can give a folder of its own. The browser's folder handle
// (the File System Access API in Chrome and Edge, or the Origin Private File System) is one kind; a
// folder in memory is the other. What is in `owd-project.json`, and whether it is valid, is decided
// by the engine (`owd_format::project`), never here.
//
// Written so that Node can run it without a build (type stripping): no parameter properties, no enums.

/** The name of the text file at the top of a project folder. */
export const PROJECT_FILE = "owd-project.json";

/** One folder. Paths are from the folder, with forward slashes. */
export interface ProjectFolder {
  /** The folder's own name (never its place on the disk: the browser does not tell). */
  name: string;
  /** The names of the files in a folder inside it ("" is the top level). None when it is not there. */
  list(dir: string): Promise<string[]>;
  /** One file, or null when it is not there. */
  read(path: string): Promise<Blob | null>;
  /** Write a file, making its folders. */
  write(path: string, data: string | Uint8Array): Promise<void>;
  /** What the browser can keep to open the folder again (a directory handle), if anything. */
  handle?: unknown;
}

/** The parts of the browser's handles this uses. */
interface FileHandleLike {
  kind: "file";
  name: string;
  getFile(): Promise<Blob>;
  createWritable(): Promise<{ write(data: string | Uint8Array): Promise<void>; close(): Promise<void> }>;
}
export interface DirHandleLike {
  kind: "directory";
  name: string;
  getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<DirHandleLike>;
  getFileHandle(name: string, options?: { create?: boolean }): Promise<FileHandleLike>;
  entries(): AsyncIterable<[string, { kind: string }]>;
  queryPermission?(o: { mode: "read" | "readwrite" }): Promise<string>;
  requestPermission?(o: { mode: "read" | "readwrite" }): Promise<string>;
}

const parts = (path: string) => path.split("/").filter((p) => p !== "");
const missing = (e: unknown) => e instanceof DOMException && (e.name === "NotFoundError" || e.name === "TypeMismatchError");

/** A project folder over a directory handle of the browser. */
export function handleFolder(root: DirHandleLike): ProjectFolder {
  async function dir(names: string[], create: boolean): Promise<DirHandleLike | null> {
    let at = root;
    for (const n of names) {
      try {
        at = await at.getDirectoryHandle(n, { create });
      } catch (e) {
        if (missing(e)) return null;
        throw e;
      }
    }
    return at;
  }
  return {
    name: root.name,
    handle: root,
    async list(path) {
      const d = await dir(parts(path), false);
      if (!d) return [];
      const names: string[] = [];
      for await (const [name, entry] of d.entries()) if (entry.kind === "file") names.push(name);
      return names.sort();
    },
    async read(path) {
      const names = parts(path);
      const file = names.pop();
      const d = file ? await dir(names, false) : null;
      if (!d || !file) return null;
      try {
        return await (await d.getFileHandle(file)).getFile();
      } catch (e) {
        if (missing(e)) return null;
        throw e;
      }
    },
    async write(path, data) {
      const names = parts(path);
      const file = names.pop();
      if (!file) throw new Error(`\`${path}\` is not a file name`);
      const d = await dir(names, true);
      const w = await (await d!.getFileHandle(file, { create: true })).createWritable();
      await w.write(data);
      await w.close();
    },
  };
}

/** A project folder in memory: for tests. `files` maps a path to its text or bytes. */
export function memoryFolder(name: string, files: Record<string, string | Uint8Array> = {}): ProjectFolder & { files: Map<string, string | Uint8Array> } {
  const held = new Map(Object.entries(files));
  return {
    name,
    files: held,
    async list(path) {
      const prefix = parts(path).join("/");
      const names: string[] = [];
      for (const p of held.keys()) {
        const at = p.lastIndexOf("/");
        if ((at < 0 ? "" : p.slice(0, at)) === prefix) names.push(p.slice(at + 1));
      }
      return names.sort();
    },
    async read(path) {
      const data = held.get(parts(path).join("/"));
      return data === undefined ? null : new Blob([data as BlobPart]);
    },
    async write(path, data) {
      held.set(parts(path).join("/"), data);
    },
  };
}

/** The files of the standard places, as the engine's search reads them. Dot files are left out. */
export async function listing(folder: ProjectFolder): Promise<{ design: string[]; terrain: string[]; inputs: string[]; top: string[] }> {
  const seen = async (dir: string) => (await folder.list(dir)).filter((n) => !n.startsWith("."));
  const [design, terrain, inputs, top] = await Promise.all([seen("design"), seen("terrain"), seen("inputs"), seen("")]);
  return { design, terrain, inputs, top };
}

/** The bytes of a file, read in pieces so a large one (a DEM) can say how far it is. The memory is one
 * buffer the size of the file, as `arrayBuffer()` takes; `onProgress` gets the bytes read and the size. */
export async function readAll(file: Blob, onProgress?: (done: number, total: number) => void): Promise<Uint8Array> {
  const total = file.size;
  if (typeof file.stream !== "function") return new Uint8Array(await file.arrayBuffer());
  const out = new Uint8Array(total);
  let done = 0;
  const reader = file.stream().getReader();
  for (;;) {
    const { done: end, value } = await reader.read();
    if (end) break;
    // A file that grew while it was read: start again in one piece.
    if (done + value.length > total) return new Uint8Array(await file.arrayBuffer());
    out.set(value, done);
    done += value.length;
    onProgress?.(done, total);
  }
  return done === total ? out : out.slice(0, done);
}

/** The progress line of a DEM read: megabytes, a new line only when the whole number changes. */
export function demProgress(say: (line: string) => void): (done: number, total: number) => void {
  let last = -1;
  const mb = (n: number) => Math.round(n / 1048576);
  return (done, total) => {
    if (mb(done) === last) return;
    last = mb(done);
    say(total >= 1048576 ? `Reading the DEM: ${mb(done)} of ${mb(total)} MB` : "Reading the DEM…");
  };
}

/** How one item of a project stands. */
export type ItemState = "loaded" | "found" | "changed" | "set" | "missing" | "none";

/** The state of one item: `path` is what the text file names, `there` whether the folder has it,
 * `inDesign` whether the open design holds it. `changed`: a DEM that is not the one the design
 * recorded. `set`: a water level is in the design, which may or may not be from this file. */
export function itemState(key: string, path: string | null, there: boolean, inDesign: boolean, demChanged = false): ItemState {
  if (!path) return "none";
  if (!there) return "missing";
  if ((key === "dem" || key === "buildings" || key === "points_of_interest" || key === "settlements") && demChanged) return "changed";
  if (!inDesign) return "found";
  return key === "water_pools" ? "set" : "loaded";
}

/** The words for a state. */
export const STATE_WORDS: Record<ItemState, string> = {
  loaded: "loaded",
  found: "found, not loaded",
  changed: "found, not the file the design recorded",
  set: "found; the design has a water level",
  missing: "missing",
  none: "not named",
};

/** Whether the browser gave leave to read and write the folder. A handle with no permission calls
 * (the Origin Private File System in some browsers) is always allowed. */
export async function allowed(handle: DirHandleLike, ask: boolean): Promise<boolean> {
  const mode = { mode: "readwrite" as const };
  try {
    if (typeof handle.queryPermission !== "function") return true;
    if ((await handle.queryPermission(mode)) === "granted") return true;
    if (!ask || typeof handle.requestPermission !== "function") return false;
    return (await handle.requestPermission(mode)) === "granted";
  } catch {
    return false;
  }
}

// --- the last project folder, remembered on this machine -------------------------------------------
// A folder handle cannot go in localStorage: it is kept in IndexedDB. Every use is guarded; where the
// store is missing or blocked nothing is remembered, and the app works the same.

const DB = "owd-project";
const STORE = "last";
const KEY = "folder";

export interface Remembered {
  handle: DirHandleLike;
  title: string;
  folder: string;
}

function store<T>(mode: "readonly" | "readwrite", work: (s: IDBObjectStore) => IDBRequest<T>): Promise<T | null> {
  return new Promise((resolve) => {
    try {
      const open = indexedDB.open(DB, 1);
      open.onupgradeneeded = () => open.result.createObjectStore(STORE);
      open.onerror = () => resolve(null);
      open.onblocked = () => resolve(null);
      open.onsuccess = () => {
        try {
          const db = open.result;
          const tx = db.transaction(STORE, mode);
          const req = work(tx.objectStore(STORE));
          let value: T | null = null;
          req.onsuccess = () => (value = req.result ?? null);
          tx.oncomplete = () => {
            db.close();
            resolve(value);
          };
          tx.onerror = tx.onabort = () => {
            db.close();
            resolve(null);
          };
        } catch {
          resolve(null);
        }
      };
    } catch {
      resolve(null);
    }
  });
}

/** Remember the folder for "Reopen". Nothing happens when the browser cannot keep it. */
export const remember = (r: Remembered): Promise<unknown> => store("readwrite", (s) => s.put(r, KEY));
/** The folder remembered last, or null. */
export const remembered = (): Promise<Remembered | null> => store<Remembered>("readonly", (s) => s.get(KEY) as IDBRequest<Remembered>);
/** Forget it. */
export const forget = (): Promise<unknown> => store("readwrite", (s) => s.delete(KEY));
