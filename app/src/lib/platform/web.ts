// The browser adapter: the owd-web WebAssembly session, in a Web Worker on this page. No server sees a
// design or a DEM. Where the desktop opens a dialog or writes a file, the browser picks a file or
// downloads one. A run that is cancelled ends the worker, and a new worker starts from the design as
// it was before the run. Errors thrown by the session are strings; they become rejected promises.
import { Host as WorkerHost } from "./host";
import type { FileDropEvent, Platform, ReachProgress } from "./types";
import { PROJECT_FILE, allowed, demProgress, forget, handleFolder, itemState, listing, readAll, remember, remembered, type DirHandleLike, type ProjectFolder } from "../project";

/** The design worker, and the requests waiting on it (host.ts). */
const newHost = (onProgress: (n: number) => void, onReach: (p: ReachProgress) => void) => new WorkerHost(() => new Worker(new URL("./worker.ts", import.meta.url), { type: "module" }), onProgress, onReach);
type Host = WorkerHost;

const LAST_DESIGN = "owd.lastDesign";
const DEFAULT_NAME = "waterway-design.owd";
const EXAMPLE = "river-example.owd";
const EXAMPLE_DEM = "river-dem.tif";
const DISCARD = "The channel design has unsaved changes. Discard them?";

/** Storage may be missing or throw (private window, blocked site data): every use is guarded. */
function readKey(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}
function writeKey(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* not remembered */
  }
}

const CANCELLED = "cancelled";

function download(name: string, data: BlobPart, type: string) {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.style.display = "none";
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

/** Ask for a file with the browser's picker. null when the user cancels. */
function pickFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.dataset.testid = "file-picker";
    // Present but out of sight, so a test can fill it in.
    input.style.cssText = "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;";
    const done = (file: File | null) => {
      input.remove();
      resolve(file);
    };
    input.addEventListener("change", () => done(input.files?.[0] ?? null));
    input.addEventListener("cancel", () => done(null));
    document.body.append(input);
    input.click();
  });
}

/** The part of a FileSystemFileHandle that saving uses. */
type SaveHandle = { name: string; createWritable: () => Promise<{ write: (t: string) => Promise<void>; close: () => Promise<void> }> };

const bytesOf = async (file: File) => new Uint8Array(await file.arrayBuffer());

/** What `owd-project.json` says, as the engine read it (`project_read`). */
interface ProjectInfo {
  title: string | null;
  design: string | null;
  dem: string | null;
  items: { key: string; label: string; path: string | null }[];
}
/** The project folder that is open: the folder, and its text file as read. */
interface OpenProject {
  folder: ProjectFolder;
  info: ProjectInfo;
}
const NO_FOLDER_ACCESS = "The browser did not let OWD into the folder. Use Open project, choose the folder again and allow editing.";
/** A write into the folder; a refusal by the browser is said in plain words. */
async function writeIn(folder: ProjectFolder, path: string, data: string | Uint8Array) {
  try {
    await folder.write(path, data);
  } catch (e) {
    if (e instanceof DOMException && (e.name === "NotAllowedError" || e.name === "SecurityError")) throw NO_FOLDER_ACCESS;
    throw `cannot write ${path} into the project folder "${folder.name}": ${e instanceof Error ? e.message : String(e)}`;
  }
}

export async function createWebPlatform(): Promise<Platform> {
  const progressHandlers = new Set<(n: number) => void>();
  const onProgress = (n: number) => progressHandlers.forEach((h) => h(n));
  const reachHandlers = new Set<(p: ReachProgress) => void>();
  const onReach = (p: ReachProgress) => reachHandlers.forEach((h) => h(p));
  let host = newHost(onProgress, onReach);
  await host.rpc("ping");

  /** The DEM the session holds, kept so a restart can give it back. */
  let dem: Uint8Array | null = null;
  /** The Rule Set files loaded in this session, kept so a restart loads them again (#499). */
  const ruleSetTexts: string[] = [];
  /** Whether the design has unsaved changes, as the last snapshot said (or as it was before a restart). */
  let dirty = false;
  let carriedDirty = false;
  let running: { stage: string; cancelled: boolean } | null = null;

  const json = async <T>(method: string, ...args: unknown[]): Promise<T> => JSON.parse(await host.rpc<string>(method, ...args)) as T;

  /** The project folder that is open (PROJ-1), and the one chosen but not opened yet. */
  let project: OpenProject | null = null;
  let chosen: { folder: ProjectFolder; info: ProjectInfo | null } | null = null;
  /** The name the design is saved under: in a project, its path from the project folder, so the
   * design names its DEM from its own folder (`../terrain/x.tif`). */
  const docName = (snap: { file_name: string }) => project?.info.design ?? snap.file_name;

  /** A snapshot: note its title and unsaved flag, and remember the design for a reload. */
  function follow<T extends { dirty: boolean; title: string; file_name: string }>(snap: T): T {
    if (carriedDirty) snap.dirty = true;
    dirty = snap.dirty;
    try {
      document.title = snap.title;
    } catch {
      /* no document title to set */
    }
    const name = docName(snap);
    host
      .rpc<string>("save_text", name)
      .then((text) => writeKey(LAST_DESIGN, JSON.stringify({ name, text })))
      .catch(() => {});
    return snap;
  }
  const change = async <T extends { dirty: boolean; title: string; file_name: string }>(method: string, ...args: unknown[]) => follow(await json<T>(method, ...args));

  const fileName = (snap: { file_name: string }) => (/\.owd$/.test(snap.file_name) ? snap.file_name : DEFAULT_NAME);
  const confirmDiscard = () => !dirty || window.confirm(DISCARD);

  /** The file the browser's save dialog gave us, so Save writes to it again without asking (File System Access). */
  let handle: SaveHandle | null = null;
  const canPick = () => typeof (window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker === "function";

  async function writeTo(h: SaveHandle, text: string) {
    const w = await h.createWritable();
    await w.write(text);
    await w.close();
  }

  /** Save with the browser's save dialog. null when the user cancels it. */
  async function pickSave(suggested: string): Promise<SaveHandle | null> {
    try {
      return await (window as unknown as { showSaveFilePicker: (o: unknown) => Promise<SaveHandle> }).showSaveFilePicker({
        suggestedName: suggested,
        types: [{ description: "Channel design", accept: { "application/json": [".owd"] } }],
      });
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return null;
      throw e;
    }
  }

  async function saveNamed(name: string, to?: SaveHandle) {
    const text = await host.rpc<string>("save_text", name);
    if (to) {
      await writeTo(to, text);
      handle = to;
    } else {
      handle = null;
      download(name, text, "application/json");
    }
    carriedDirty = false;
    return change("saved", name);
  }

  /** Save as: the browser's save dialog where there is one (it asks for the name), else a name prompt and a download. */
  async function saveAsChosen(current: string) {
    if (canPick()) {
      const h = await pickSave(current);
      if (!h) return null;
      project = null; // a copy somewhere else: it is no longer the project's design
      return saveNamed(/\.owd$/.test(h.name) ? h.name : `${h.name}.owd`, h);
    }
    const name = window.prompt("File name (the browser will download it)", current);
    if (name === null || name.trim() === "") return null;
    const clean = name.trim();
    project = null;
    return saveNamed(/\.owd$/.test(clean) ? clean : `${clean}.owd`);
  }

  /** Save the project's design back into the project folder. */
  async function saveInProject(p: OpenProject) {
    const path = p.info.design!;
    await writeIn(p.folder, path, await host.rpc<string>("save_text", path));
    handle = null;
    carriedDirty = false;
    return change("saved", path);
  }

  /** Give the user the files OWD made. In a project they go into `exports/` of the folder; else
   * the browser downloads them. Returns where they went, in words. */
  async function deliver(files: [string, BlobPart, string][], count?: string): Promise<string | null> {
    if (!files.length) return null;
    const p = project;
    if (!p) {
      for (const [name, data, type] of files) download(name, data, type);
      return count ?? files.map(([name]) => name).join(", ");
    }
    const paths: string[] = [];
    for (const [name, data] of files) {
      const path = await host.rpc<string>("project_export_path", name);
      await writeIn(p.folder, path, data as string | Uint8Array);
      paths.push(path);
    }
    return `${count ? `${count} in exports/` : paths.join(", ")} in the project folder "${p.folder.name}"`;
  }

  const canPickFolder = () => typeof (window as unknown as { showDirectoryPicker?: unknown }).showDirectoryPicker === "function";

  /** Look into a folder the user chose: its text file as the engine reads it, or, with no text
   * file, what the standard places hold. The folder is kept until it is opened. */
  async function look(folder: ProjectFolder) {
    const file = await folder.read(PROJECT_FILE);
    if (file) {
      const info = await json<ProjectInfo>("project_read", await file.text());
      chosen = { folder, info };
      return { folder: folder.name, title: info.title, found: null };
    }
    chosen = { folder, info: null };
    return { folder: folder.name, title: null, found: await json("project_find", JSON.stringify(await listing(folder))) };
  }

  /** Ask for a folder with the browser's folder dialog. null when the user cancels it. */
  async function pickProject() {
    if (!canPickFolder()) throw "This browser cannot open a folder: Chrome and Edge can. Use Open for the design and Load… for the DEM.";
    if (!confirmDiscard()) return null;
    let dir: DirHandleLike;
    try {
      dir = await (window as unknown as { showDirectoryPicker: (o: unknown) => Promise<DirHandleLike> }).showDirectoryPicker({ mode: "readwrite", id: "owd-project" });
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return null;
      throw e;
    }
    return look(handleFolder(dir));
  }

  /** The folder remembered from last time; the browser asks the user for leave again. */
  async function reopenProject() {
    const last = await remembered();
    if (!last) throw "No project folder is remembered in this browser. Use Open project.";
    if (!confirmDiscard()) return null;
    if (!(await allowed(last.handle, true))) throw NO_FOLDER_ACCESS;
    return look(handleFolder(last.handle));
  }

  /** Write `owd-project.json` into the chosen folder, which has none: the design and the DEM the
   * user took, and the inputs found under their standard names. */
  async function adoptProject(design: string | null, dem: string | null) {
    if (!chosen) throw "Choose a project folder first.";
    const { folder } = chosen;
    if (await folder.read(PROJECT_FILE)) throw `${PROJECT_FILE} is already in the folder: OWD does not overwrite it.`;
    const text = await host.rpc<string>("project_adopt", JSON.stringify(await listing(folder)), folder.name, design ?? undefined, dem ?? undefined);
    await writeIn(folder, PROJECT_FILE, text);
    chosen = { folder, info: await json<ProjectInfo>("project_read", text) };
  }

  /** Open the chosen project: its design, and the DEM it names, in one step. */
  async function openProject(say: (line: string) => void) {
    if (!chosen?.info) throw "Choose a project folder first.";
    const { folder, info } = chosen;
    if (!info.design) throw `${PROJECT_FILE} names no design: add "design": "design/<name>.owd" to it.`;
    const designFile = await folder.read(info.design);
    if (!designFile) throw `The project names the design ${info.design}, but that file is not in the folder "${folder.name}".`;
    const text = await designFile.text();
    let bytes: Uint8Array | null = null;
    const demFile = info.dem ? await folder.read(info.dem) : null;
    if (demFile) bytes = await readAll(demFile, demProgress(say));
    say(bytes ? "Loading the design and the DEM…" : "Loading the design…");
    const before = project;
    project = { folder, info };
    try {
      const snap = await change<{ dirty: boolean; title: string; file_name: string }>("open_project", info.design, text, bytes ? info.dem : undefined, bytes ?? undefined);
      dem = bytes;
      handle = null;
      carriedDirty = false;
      chosen = null;
      if (folder.handle) await remember({ handle: folder.handle as DirHandleLike, title: info.title ?? folder.name, folder: folder.name });
      return snap;
    } catch (e) {
      project = before;
      throw e;
    }
  }

  /** The open project for the panel: each item the text file names, and how it stands. */
  async function projectStatus() {
    const p = project;
    if (!p) return null;
    const snap = await json<{ warnings: string[]; document: { inputs: Record<string, unknown> } }>("snapshot");
    const inputs = snap.document.inputs;
    const demChanged = snap.warnings.some((w) => w.includes("has changed since its hash was recorded"));
    const inDesign = (key: string) => (key === "design" ? true : key === "dem" ? !!inputs.dem && dem !== null : key === "water_pools" ? !!inputs.water_level : !!inputs[key]);
    const items = [];
    for (const item of p.info.items) {
      const there = item.path ? (await p.folder.read(item.path)) !== null : false;
      items.push({ ...item, state: itemState(item.key, item.path, there, inDesign(item.key), demChanged) });
    }
    return { title: p.info.title ?? p.folder.name, folder: p.folder.name, design: p.info.design, items };
  }

  /** Load one item of the open project into the design, through the same calls as Load…. */
  async function projectLoad(key: string, say: (line: string) => void) {
    const p = project;
    const path = p?.info.items.find((i) => i.key === key)?.path;
    if (!p || !path) throw "The project does not name that file.";
    const file = await p.folder.read(path);
    if (!file) throw `${path} is not in the project folder "${p.folder.name}".`;
    if (key === "dem") {
      const bytes = await readAll(file, demProgress(say));
      say("Loading the DEM…");
      const snap = await change("load_dem", path, bytes);
      dem = bytes;
      return snap;
    }
    if (key === "water_pools") return change("set_water_pools", await file.text());
    return change("load_input", key, path.split("/").pop(), await readAll(file), undefined);
  }

  /** Bring back the design as it stood before a run, in a new worker. */
  async function restart(before: { name: string; text: string; dem: Uint8Array | null; dirty: boolean }) {
    host = newHost(onProgress, onReach);
    await host.rpc("ping");
    dem = before.dem;
    for (const text of ruleSetTexts) await host.rpc("load_rule_set_text", text);
    await host.rpc("open_text", before.name, before.text, before.dem ?? undefined);
    carriedDirty = before.dirty;
  }

  async function runStage(stage: string, method: string | null) {
    const snap = await json<{ file_name: string; dirty: boolean }>("snapshot");
    const before = { name: docName(snap), text: await host.rpc<string>("save_text", docName(snap)), dem, dirty: snap.dirty || carriedDirty };
    running = { stage, cancelled: false };
    const mine = running;
    try {
      const result = await json<{ dirty: boolean; title: string; file_name: string }>("run_stage", stage, method ?? undefined);
      carriedDirty = false;
      return follow(result);
    } catch (e) {
      // Cancel ended the worker; so can a stage that stopped unexpectedly. Either way the design is as it was.
      let alive = true;
      if (!mine.cancelled) await host.rpc("ping").catch(() => (alive = false));
      if (mine.cancelled || !alive) {
        await restart(before);
        const back = await change<{ dirty: boolean; title: string; file_name: string }>("snapshot");
        if (mine.cancelled) return { ...back, run: { stage, outcome: "cancelled" } };
        throw "the stage stopped unexpectedly: the design is as it was";
      }
      throw e;
    } finally {
      if (running === mine) running = null;
    }
  }

  /** Where the reach sources are: empty is the public services; a test points them at recordings.
   * `?photon=URL` and `?nominatim=URL` on the page name another river search server, and
   * `?overpass=URL,URL` the Overpass servers, asked in that order (#319). */
  let reachAt = (() => {
    try {
      const q = new URLSearchParams(window.location.search);
      const at: Record<string, string> = {};
      for (const k of ["photon", "nominatim", "overpass"]) if (q.get(k)) at[k] = q.get(k)!;
      return Object.keys(at).length ? JSON.stringify(at) : "";
    } catch {
      return "";
    }
  })();

  /** One reach step in the worker. Cancel ends the worker, as for a stage, and the design comes back as it was. */
  /** Network work in the worker (a route, a reach step). Cancel ends the worker, as for a stage, and
   * the design comes back as it was. */
  async function network(method: string, ...args: unknown[]) {
    const snap = await json<{ file_name: string; dirty: boolean }>("snapshot");
    const before = { name: docName(snap), text: await host.rpc<string>("save_text", docName(snap)), dem, dirty: snap.dirty || carriedDirty };
    running = { stage: method, cancelled: false };
    const mine = running;
    try {
      const result = await change<{ dirty: boolean; title: string; file_name: string }>(method, ...args);
      carriedDirty = false;
      return result;
    } catch (e) {
      if (mine.cancelled) {
        await restart(before);
        throw "cancelled: the design is as it was";
      }
      throw e;
    } finally {
      if (running === mine) running = null;
    }
  }

  async function loadInput(role: string, given: unknown) {
    const file = given instanceof File ? given : await pickFile(role === "dem" ? ".tif,.tiff" : ".geojson,.json,.gpkg");
    if (!file) return null;
    const bytes = await bytesOf(file);
    if (role === "dem") {
      const snap = await change("load_dem", file.name, bytes);
      dem = bytes;
      return snap;
    }
    return change("load_input", role, file.name, bytes, undefined);
  }

  async function openExample() {
    if (!confirmDiscard()) return null;
    const base = import.meta.env.BASE_URL;
    const [text, tif] = await Promise.all([fetch(`${base}examples/${EXAMPLE}`).then((r) => (r.ok ? r.text() : Promise.reject(`cannot load the example (${r.status})`))), fetch(`${base}examples/${EXAMPLE_DEM}`).then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(`cannot load the example DEM (${r.status})`)))]);
    dem = new Uint8Array(tif);
    handle = null;
    project = null;
    carriedDirty = false;
    return change("open_text", EXAMPLE, text, dem);
  }

  const commands: Record<string, (args: Record<string, unknown>) => Promise<unknown>> = {
    snapshot: () => change("snapshot"),
    sizing_choices: () => json("sizing_choices"),
    load_rule_set_file: async () => {
      const file = await pickFile(".json");
      if (!file) return null;
      const text = await file.text();
      const loaded = await host.rpc<string>("load_rule_set_text", text);
      if (!ruleSetTexts.includes(text)) ruleSetTexts.push(text);
      return { loaded };
    },
    new_file: async () => {
      if (!confirmDiscard()) return null;
      dem = null;
      handle = null;
      project = null;
      carriedDirty = false;
      return change("new_design");
    },
    open_file: async () => {
      if (!confirmDiscard()) return null;
      const file = await pickFile(".owd");
      if (!file) return null;
      const text = await file.text();
      dem = null;
      handle = null;
      project = null;
      carriedDirty = false;
      return change("open_text", file.name, text, undefined);
    },
    save: async () => {
      if (project?.info.design) return saveInProject(project);
      const name = fileName(await json("snapshot"));
      if (handle) return saveNamed(handle.name, handle);
      return canPick() ? saveAsChosen(name) : saveNamed(name);
    },
    save_as: async () => saveAsChosen(fileName(await json("snapshot"))),
    set_rule_sets: (a) => change("set_rule_sets", JSON.stringify(a.ids), JSON.stringify(a.inputs)),
    set_vessel: (a) => change("set_vessel", a.vesselStandardId, a.draught),
    set_custom_vessel: (a) => change("set_custom_vessel", JSON.stringify(a.fields)),
    check_custom_vessel: (a) => json("check_custom_vessel", JSON.stringify(a.fields)),
    stretches_geojson: () => json("stretches_geojson"),
    run_stale: () => change("run_stale"),
    map_geometry: () => json("map_geometry"),
    option_table: () => json("option_table"),
    hillshade: () => json("hillshade"),
    // The 3D view (MAP-2): the DEM's terrain tiles are made in the worker, from the DEM in memory.
    terrain_info: () => json("terrain_info"),
    terrain_tile: (a) => host.rpc("terrain_tile", a.z, a.x, a.y),
    // Contours (MAP-3): traced in the worker from the same DEM, a vector tile for each map tile.
    contour_tile: (a) => host.rpc("contour_tile", a.z, a.x, a.y, a.interval),
    // Weirs, locks and pools (MAP-4): the list is made by the session; the pool water is worked out in the
    // worker from the DEM in memory, and its tiles are made there too.
    map_structures: () => json("map_structures"),
    pool_water_info: () => json("pool_water_info"),
    pool_water_tile: (a) => host.rpc("pool_water_tile", a.id, a.z, a.x, a.y),
    pool_surface_tile: (a) => host.rpc("pool_surface_tile", a.id, a.z, a.x, a.y),
    pool_water_at: (a) => json("pool_water_at", a.id, a.lon, a.lat),
    long_section: (a) => json("long_section", a.option),
    sheets: (a) => json("sheets", a.option),
    export_sheets: async (a) => {
      // each sheet's PDF, as `owd draw` names it; the browser saves them as downloads
      const pdfs = await host.rpc<[string, Uint8Array][]>("sheet_pdfs", a.option);
      return deliver(pdfs.map(([name, bytes]) => [name, bytes as BlobPart, "application/pdf"]), `${pdfs.length} PDF${pdfs.length === 1 ? "" : "s"}`);
    },
    export_report: async (a) => {
      // each report file, as `owd report` names it; the browser saves them as downloads
      const files = await host.rpc<[string, Uint8Array][]>("report_files", a.option ?? undefined, JSON.stringify(a.ask));
      const types: Record<string, string> = {
        pdf: "application/pdf",
        html: "text/html",
        docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      };
      return deliver(files.map(([name, bytes]) => [name, bytes as BlobPart, types[name.split(".").pop() ?? ""] ?? "application/octet-stream"]));
    },
    export_qa_overlay: async (a) => {
      // the QA overlay's PNG and KMZ (spec 0013), drawn in the worker from the scenes; saved as downloads
      const files = await host.rpc<[string, Uint8Array][]>("qa_overlay", a.fromKm ?? undefined, a.toKm ?? undefined, reachAt);
      return deliver(files.map(([name, bytes]) => [name, bytes as BlobPart, name.endsWith(".kmz") ? "application/vnd.google-earth.kmz" : "image/png"]));
    },
    cross_section: (a) => json("cross_section", a.option, a.chainage),
    station_near: (a) => host.rpc("station_near", a.option, a.lon, a.lat),
    load_input: (a) => loadInput(a.role as string, a.path),
    set_water: (a) => change("set_water", a.level, a.fall),
    set_alignment_settings: (a) => change("set_alignment_settings", a.method, JSON.stringify(a.settings)),
    set_optimise_settings: (a) => change("set_optimise_settings", JSON.stringify(a.settings)),
    set_terrain_limit: (a) => change("set_terrain_limit", (a.gib as string) ?? ""),
    set_downstream_point: (a) => change("set_downstream_point", (a.point as string) ?? ""),
    set_mouth: (a) => change("set_mouth", (a.mouth as string) ?? ""),
    set_run_inputs: (a) => change("set_run_inputs", (a.objective as string) ?? "", (a.limits as string) ?? "", (a.targetLevel as string) ?? ""),
    edit_bank: (a) => change("edit_bank", a.side, JSON.stringify(a.edit)),
    undo_bank_edit: () => change("undo_bank_edit"),
    run_stage: (a) => runStage(a.stage as string, (a.method as string | null) ?? null),
    cancel_run: async () => {
      if (!running) return false;
      running.cancelled = true;
      host.end(CANCELLED);
      return true;
    },
    fetch_dem: () => Promise.reject("Fetching a DEM is not offered in the browser: load a GeoTIFF instead."),
    basemap_tiles: async () => null,
    reach_route: async (a) => {
      if (!confirmDiscard()) return null;
      carriedDirty = false;
      project = null;
      return network("reach_route", a.river ?? "", JSON.stringify({ start: a.start, end: a.end, via: a.via ?? [] }), reachAt);
    },
    reach_search: (a) => json("reach_search", a.text ?? "", a.country ?? "", a.once === true, reachAt),
    reach_river: async (a) => {
      if (!confirmDiscard()) return null;
      carriedDirty = false;
      project = null;
      return network("reach_river", a.osmId, JSON.stringify(a.also ?? []), (a.search as string) ?? "", reachAt);
    },
    reach_cut: async (a) => {
      if (!(await host.rpc<boolean>("reach_route_only")) && !confirmDiscard()) return null;
      carriedDirty = false;
      return change("reach_cut", JSON.stringify({ start: a.start, end: a.end, via: a.via ?? [] }));
    },
    reach_restore: () => network("reach_restore", reachAt),
    reach_add_tile: async (a) => {
      const file = a.file instanceof File ? a.file : await pickFile(".tif,.tiff");
      if (!file) return null;
      return change("reach_add_tile", file.name, await bytesOf(file));
    },
    reach_step: (a) => network("reach_step", a.step as string, (a.percentile as number | null) ?? undefined, reachAt),
    reach_cancel: async () => {
      if (!running) return;
      running.cancelled = true;
      host.end(CANCELLED);
    },
    set_bed: (a) => change("set_bed", (a.depth as string) ?? "", (a.table as string) ?? "", (a.season as string) ?? ""),
    run_fits: () => change("run_fits"),
    run_envelope: () => change("run_envelope"),
    open_example: openExample,
    export_file: async (a) => {
      const kind = a.kind as string;
      if (kind !== "geojson" && kind !== "csv") throw `\`${kind}\` is not an export: use geojson or csv`;
      const snap = await json<{ file_name: string }>("snapshot");
      const base = fileName(snap).replace(/\.owd$/, "");
      const text = await host.rpc<string>(kind === "geojson" ? "export_geojson" : "export_csv");
      return deliver([[`${base}.${kind}`, text, kind === "geojson" ? "application/geo+json" : "text/csv"]]);
    },
    // The project folder (PROJ-1): the folder is read and written here; its text file is the engine's.
    project_supported: async () => canPickFolder(),
    project_pick: pickProject,
    project_remembered: async () => {
      const last = await remembered();
      return last ? { title: last.title, folder: last.folder } : null;
    },
    project_reopen: reopenProject,
    project_forget: async () => {
      await forget();
    },
    project_adopt: (a) => adoptProject((a.design as string | null) ?? null, (a.dem as string | null) ?? null),
    project_open: (a) => openProject(typeof a.onProgress === "function" ? (a.onProgress as (line: string) => void) : () => {}),
    project_status: projectStatus,
    project_load: (a) => projectLoad(a.key as string, typeof a.onProgress === "function" ? (a.onProgress as (line: string) => void) : () => {}),
  };

  // Bring back the last design, if there is one. Any failure leaves the empty design.
  try {
    const raw = readKey(LAST_DESIGN);
    if (raw) {
      const last = JSON.parse(raw) as { name: string; text: string };
      await host.rpc("open_text", last.name, last.text, undefined);
    }
  } catch {
    /* ignore: the empty design stays */
  }

  window.addEventListener("beforeunload", (e) => {
    if (dirty) {
      e.preventDefault();
      e.returnValue = "";
    }
  });

  const platform: Platform = {
    kind: "web",
    call: (command, args) => {
      const run = commands[command];
      return (run ? run(args ?? {}) : Promise.reject(`\`${command}\` is not available in the browser`)) as Promise<never>;
    },
    onReachProgress: async (handler) => {
      reachHandlers.add(handler);
      return () => reachHandlers.delete(handler);
    },
    onProgress: async (handler) => {
      progressHandlers.add(handler);
      return () => progressHandlers.delete(handler);
    },
    onFileDrop: async (handler: (e: FileDropEvent) => void) => {
      const over = (e: DragEvent) => {
        if (!e.dataTransfer?.types.includes("Files")) return;
        e.preventDefault();
        handler({ type: "over", x: e.clientX, y: e.clientY });
      };
      const leave = (e: DragEvent) => {
        if (e.relatedTarget === null) handler({ type: "leave" });
      };
      const drop = (e: DragEvent) => {
        const file = e.dataTransfer?.files[0];
        if (!file) return;
        e.preventDefault();
        handler({ type: "drop", x: e.clientX, y: e.clientY, name: file.name, file });
      };
      window.addEventListener("dragover", over);
      window.addEventListener("dragleave", leave);
      window.addEventListener("drop", drop);
      return () => {
        window.removeEventListener("dragover", over);
        window.removeEventListener("dragleave", leave);
        window.removeEventListener("drop", drop);
      };
    },
  };
  // The smoke test reads results through this, and points the reach sources at recorded answers;
  // only a build in test mode has it.
  if (import.meta.env.MODE === "test") {
    commands.set_reach_endpoints = async (a) => {
      reachAt = JSON.stringify(a.endpoints ?? {});
    };
    (window as unknown as { __owdPlatform: Platform }).__owdPlatform = platform;
  }
  return platform;
}
