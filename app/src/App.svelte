<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import * as api from "./lib/api";
  import type { ChannelView } from "./lib/channel";
  import type { Choices, CrossSection as CrossSectionData, CustomFields, FitsSelection, FloodSetting, Hillshade, InputRole, LongSection as LongSectionData, MapGeometry, OptionTable, ProjectLook, ProjectStatus, RunMethod, Snapshot, StageName, StretchCollection, TerrainInfo } from "./lib/api";
  import ProjectDialog from "./lib/ProjectDialog.svelte";
  import CrossSection from "./lib/CrossSection.svelte";
  import LongSection from "./lib/LongSection.svelte";
  import FloatPanel from "./lib/FloatPanel.svelte";
  import StretchesDrawer from "./lib/StretchesDrawer.svelte";
  import { currentTheme, optionVar, setTheme, type ThemeChoice } from "./lib/theme";
  import { BASEMAP_TILES } from "./lib/basemap";
  import MapPanel from "./lib/MapPanel.svelte";
  import ImageryControl from "./lib/ImageryControl.svelte";
  import { layerFor, DEFAULT_IMAGERY, type ImageryLayer } from "./lib/imagery";
  import BankEditor from "./lib/BankEditor.svelte";
  import StartFromRiver from "./lib/StartFromRiver.svelte";
  import { lineOnScreen, plainNote, plainWarning, sectionReason } from "./lib/words";
  import SheetsView from "./lib/SheetsView.svelte";
  import type * as maplibregl from "maplibre-gl";
  import LeftPanel from "./lib/LeftPanel.svelte";
  import { platform } from "./lib/platform";
  import markLight from "./assets/owd-mark.svg";
  import markDark from "./assets/owd-mark-reversed-clear.svg";

  // Display state only: the channel design, its stage status and its results all come from Rust.
  let snap = $state<Snapshot | null>(null);
  let choices = $state<Choices | null>(null);
  let error = $state<string | null>(null);
  let geometry = $state<MapGeometry | null>(null);
  let selectedId = $state<string | null>(null);
  let table = $state<OptionTable | null>(null);
  let longSection = $state<LongSectionData | null>(null);
  let crossSection = $state<CrossSectionData | null>(null);
  let sectionNote = $state<string | null>(null);
  /** The station: a chainage on the selected option's line, set by the slider, a plot click or a map click. */
  let station = $state<number | null>(null);
  let offline = $state(false);
  let hillshade = $state<Hillshade | null>(null);
  let terrain = $state<TerrainInfo | null>(null);
  let channel = $state<ChannelView | null>(null);
  let tiles = $state<string | null>(null);
  let imagery = $state<ImageryLayer>(layerFor(DEFAULT_IMAGERY));
  let imageryNote = $state<string | null>(null);
  let imageryControl = $state<ImageryControl | undefined>();
  let shadeFor: string | null = null;
  let map = $state<maplibregl.Map | undefined>();
  let editingBanks = $state(false);
  /** "Start from a river" (spec 0012) is showing in the left panel. */
  let reachView = $state(false);
  /** The Sheets view over the map and sections (spec 0004): the drawing sheets of the active option. */
  let sheetsView = $state(false);
  /** Set while a map click picks a point of the reach. */
  let reachPicker = $state<((lon: number, lat: number) => void) | undefined>();

  // The map redraws from the session's geometry whenever the design changes. A design with
  // nothing to draw yet (no CRS, no inputs) just shows the empty map.
  $effect(() => {
    void snap?.document;
    api.mapGeometry().then(
      (g) => (geometry = g),
      () => (geometry = null),
    );
  });

  // The constraint stretches, as the session exports them in WGS84, for the map and the table.
  let stretches = $state<StretchCollection | null>(null);
  /** The stretch picked in the table: its option and its place in that option's list. */
  let selectedStretch = $state<{ option: string; index: number } | null>(null);
  $effect(() => {
    void snap?.document;
    api.stretchesGeojson().then(
      (fc) => (stretches = fc),
      () => (stretches = null),
    );
  });
  // A stretch belongs to its option: another option, or a new design, drops the pick.
  $effect(() => {
    if (selectedStretch && selectedStretch.option !== selectedId) selectedStretch = null;
  });

  // A stage running on a worker: which, and how many DEM points it has read so far.
  let busy = $state<StageName | null>(null);
  let samples = $state(0);
  let notice = $state<string | null>(null);
  let dropTarget = $state<InputRole | null>(null);

  async function run(stage: StageName, method?: RunMethod): Promise<boolean> {
    await flush();
    busy = stage;
    samples = 0;
    notice = null;
    const next = await guard(() => api.runStage(stage, method));
    busy = null;
    if (!next) return false;
    snap = next;
    if (next.run.outcome === "cancelled") {
      notice = `The ${stage} run was cancelled. The design is as it was.`;
      return false;
    }
    return true;
  }

  async function cancel() {
    await guard(() => api.cancelRun());
  }

  /** Run the stages that are due, each on the worker, so it shows progress and can be cancelled.
   * The shell decides what is due (`snap.due`, the session's one run loop); this only steps
   * through its answer, asking again after each run. Stop at the first that does not finish, or
   * if a stage that just finished is still the first due (it would never end). */
  async function runStale() {
    let last: StageName | null = null;
    for (let next = snap?.due[0]; next && next !== last; next = snap?.due[0]) {
      last = next;
      if (!(await run(next))) return;
    }
  }

  const isWeb = platform().kind === "web";

  async function load(role: InputRole, path?: string | File) {
    await fileAction(() => api.loadInput(role, path));
  }

  /** The rolling circle's water-aware mode (#494): a single pick, sent at once after any queued edit. */
  async function useWater(choice: api.UseWater) {
    await fileAction(() => api.setUseWater(choice));
  }

  async function water(level: string, fall: string) {
    await fileAction(() => api.setWater(level, fall));
  }

  async function terrainLimit(gib: string) {
    await fileAction(() => api.setTerrainLimit(gib));
  }

  async function riverMouth(mouth: string) {
    await fileAction(() => api.setMouth(mouth));
  }
  async function runInputs(objective: string, limits: string, level: string) {
    await fileAction(() => api.setRunInputs(objective, limits, level));
  }
  async function optimiseLength(weight: string) {
    const x = Number(weight);
    if (weight.trim() === "" || !Number.isFinite(x) || x < 0) {
      error = "The cost per metre of line must be a number, 0 or more.";
      return;
    }
    await fileAction(() => api.setOptimiseSettings({ length_weight: x }));
  }
  async function optimiseLand(weight: string) {
    const x = Number(weight);
    if (weight.trim() === "" || !Number.isFinite(x) || x < 0) {
      error = "The cost per m² of land must be a number, 0 or more.";
      return;
    }
    await fileAction(() => api.setOptimiseSettings({ land_weight: x }));
  }
  async function corridorLimit(on: boolean) {
    await fileAction(() => api.setOptimiseSettings({ corridor_limit: on }));
  }
  /** Set a flood setting of the weir search: the cost per hectare of flooded land, or the limit above bank-full (empty: no limit). */
  async function optimiseFlood(setting: FloodSetting, value: string) {
    const text = value.trim();
    if (setting === "max_above_bank_m" && text === "") {
      await fileAction(() => api.setOptimiseSettings({ max_above_bank_m: null }));
      return;
    }
    const x = Number(text);
    if (text === "" || !Number.isFinite(x) || x < 0) {
      error =
        setting === "flood_cost_per_ha"
          ? "The cost per hectare of flooded land must be a number, 0 or more."
          : setting === "building_cost"
          ? "The cost per building flooded must be a number, 0 or more."
          : "The limit above bank-full must be a number of metres, 0 or more, or empty for no limit.";
      return;
    }
    await fileAction(() => api.setOptimiseSettings({ [setting]: x }));
  }
  /** A join setting of the pool search (OPT-3): a tick box, or a length in metres. An empty straight through a weir site means two vessel lengths. */
  async function joinSetting(key: api.JoinSetting, value: boolean | string) {
    if (typeof value === "boolean") {
      await fileAction(() => api.setOptimiseSettings({ [key]: value }));
      return;
    }
    if (key === "weir_straight_m" && value.trim() === "") {
      await fileAction(() => api.setOptimiseSettings({ weir_straight_m: null }));
      return;
    }
    const x = Number(value);
    if (value.trim() === "" || !Number.isFinite(x) || x < 0) {
      error = "The length must be a number of metres, 0 or more.";
      return;
    }
    await fileAction(() => api.setOptimiseSettings({ [key]: x }));
  }
  /** The design line the screen draws, and the stage that made it. */
  let lineName = $derived(lineOnScreen(snap?.document, snap?.stages, table?.rows[0]?.name));
  async function downstreamPoint(point: string) {
    await fileAction(() => api.setDownstreamPoint(point));
  }

  async function exportAs(kind: "geojson" | "csv") {
    await flush();
    notice = null;
    const path = await guard(() => api.exportFile(kind));
    if (path) notice = `Exported to ${path}. The file says "Screening — not for construction".`;
  }

  async function fetchDem() {
    await fileAction(api.fetchDem);
  }

  /** The row under a window position given in CSS pixels, for drag and drop. */
  function rowAt(x: number, y: number): InputRole | null {
    const el = document.elementFromPoint(x, y);
    return (el?.closest("[data-drop-role]")?.getAttribute("data-drop-role") as InputRole | null) ?? null;
  }

  let leftWidth = $state(340);

  // The map fills the space beside the left panel. The profile (the long section) and the cross-section
  // float over it, and the constraint stretches open in a drawer: all closed until asked for.
  let profileOpen = $state(false);
  let sectionOpen = $state(false);
  let stretchesOpen = $state(false);
  let profileToggle = $state<HTMLButtonElement | undefined>();
  let sectionToggle = $state<HTMLButtonElement | undefined>();
  let profilePanel = $state<FloatPanel | undefined>();
  let sectionPanel = $state<FloatPanel | undefined>();
  /** A toolbar toggle: show or hide a floating panel. An opened panel takes the focus, so Escape closes it. */
  async function togglePanel(which: "profile" | "section") {
    if (which === "profile") profileOpen = !profileOpen;
    else sectionOpen = !sectionOpen;
    await tick();
    (which === "profile" ? profilePanel : sectionPanel)?.focus();
  }
  async function toggleStretches() {
    stretchesOpen = !stretchesOpen;
    await tick();
    if (stretchesOpen) document.getElementById("stretches-drawer")?.focus();
  }

  // Sizing recomputes live, after a pause in typing (about 150 ms); nothing else reruns by itself.
  const PAUSE_MS = 150;
  let pending: ReturnType<typeof setTimeout> | undefined;
  let queued: (() => Promise<Snapshot>) | undefined;

  async function guard<T>(work: () => Promise<T>): Promise<T | undefined> {
    try {
      error = null;
      return await work();
    } catch (e) {
      error = String(e);
      return undefined;
    }
  }

  /** Send the queued sizing edit now. A refusal keeps the last good result and shows the reason. */
  async function flush() {
    clearTimeout(pending);
    pending = undefined;
    const send = queued;
    queued = undefined;
    if (!send) return;
    // A refused choice keeps the last good channel shape, shows the reason, and leaves the form as typed.
    const next = await guard(send);
    if (next) snap = next;
  }

  function onVessel(vessel: string, draught: string) {
    queued = () => api.setVessel(vessel, draught);
    clearTimeout(pending);
    pending = setTimeout(() => flush(), PAUSE_MS);
  }

  /** "Size" on the custom vessel's fields: a single press, sent at once after any queued edit. */
  async function onCustomVessel(fields: CustomFields) {
    await fileAction(() => api.setCustomVessel(fields));
  }

  /** A Rule Set or set input is a single pick, not typing: send it at once, after any queued edit. */
  async function onRuleSets(ids: string[], inputs: Record<string, string>) {
    await fileAction(() => api.setRuleSets(ids, inputs));
  }

  /** A file action is running (Save of a large design takes a moment). A second press, or Enter on a
   * button that still has focus, must not start another: it would ask for the name again. */
  let fileBusy = $state(false);
  let fileRunning = false;

  async function fileAction(action: () => Promise<Snapshot | null>) {
    if (fileRunning) return;
    fileRunning = true;
    fileBusy = true;
    try {
      await flush();
      const next = await guard(action);
      if (next) snap = next;
      return next;
    } finally {
      fileRunning = false;
      fileBusy = false;
      canPick = detectPick();
    }
  }

  /** What the web version's Save does, said where it is used. */
  const detectPick = () => isWeb && typeof (window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker === "function";
  /** Whether the browser has a save dialog; read again after each file action. */
  let canPick = $state(detectPick());
  let SAVE_TITLE = $derived(!isWeb ? "Save the design" : canPick ? "Save the design to a file you choose, with your browser's save dialog" : "Your browser downloads the design to its downloads folder; it does not save in place");
  let SAVE_AS_TITLE = $derived(!isWeb ? "Save the design under another name" : canPick ? "Save the design under a name and place you choose, with your browser's save dialog" : "Asks for a name, then your browser downloads the file to its downloads folder");

  /** A header Save or Save as: let go of the button first (Enter would press it again), then run. */
  async function saveButton(e: MouseEvent | null, action: () => Promise<Snapshot | null>) {
    (e?.currentTarget as HTMLElement | null | undefined)?.blur();
    const inProject = action === api.save ? project : null;
    const saved = await fileAction(action);
    if (saved && isWeb) notice = inProject?.design ? `Saved ${inProject.design} in the project folder "${inProject.folder}".` : canPick ? `Saved ${saved.file_name}.` : `Downloaded ${saved.file_name}: look in your browser's downloads folder.`;
  }

  // --- the project folder (PROJ-1): the design and its DEM from one folder, in one step ---------------
  /** The open project, as the Project panel shows it; null: none. Asked again after every change. */
  let project = $state<ProjectStatus | null>(null);
  /** A folder the user chose that has no owd-project.json yet: the dialog shows what was found. */
  let projectLook = $state<ProjectLook | null>(null);
  /** What opening the project, or loading one of its items, is doing now. */
  let projectProgress = $state<string | null>(null);
  /** The project folder remembered from last time, offered once at the start. */
  let lastProject = $state<{ title: string; folder: string } | null>(null);
  /** Chrome and Edge can open a folder; Firefox and Safari cannot. */
  const canProject = isWeb && typeof (window as unknown as { showDirectoryPicker?: unknown }).showDirectoryPicker === "function";
  const NO_PROJECT = "This browser cannot open a folder: Chrome and Edge can. Use Open for the design and Load… for the DEM.";
  $effect(() => {
    void snap;
    if (!isWeb || !snap) return;
    api.projectStatus().then(
      (p) => (project = p),
      () => (project = null),
    );
  });

  /** Open project, or Reopen: look into the folder, then open it, or show what was found. */
  async function chooseProject(look: () => Promise<ProjectLook | null>) {
    if (fileRunning) return;
    await flush();
    notice = null;
    const chosen = await guard(look);
    if (!chosen) return;
    lastProject = null;
    if (chosen.found) projectLook = chosen;
    else await loadProject();
  }
  /** Open the chosen project: its design and its DEM in one step, with a line of progress. */
  async function loadProject() {
    projectLook = null;
    await openDoc(async () => {
      projectProgress = "Opening the project…";
      try {
        const next = await api.projectOpen((line) => (projectProgress = line));
        if (next.project_note) notice = next.project_note;
        return next;
      } finally {
        projectProgress = null;
      }
    });
  }
  /** "Use these and write owd-project.json". */
  async function adoptProject(design: string | null, dem: string | null) {
    const written = await guard(async () => {
      await api.projectAdopt(design, dem);
      return true;
    });
    if (!written) {
      projectLook = null;
      return;
    }
    await loadProject();
    if (!error) notice = [`Wrote owd-project.json into the project folder.`, notice].filter(Boolean).join(" ");
  }
  /** A Load button of the Project panel. */
  async function loadProjectItem(key: string) {
    await fileAction(async () => {
      try {
        return await api.projectLoad(key, (line) => (projectProgress = line));
      } finally {
        projectProgress = null;
      }
    });
  }

  /** The warnings the user dismissed, and whether the older-format row is dismissed or open. A
   * different design starts with all of them shown again. */
  let dismissedWarnings = $state<string[]>([]);
  let openInfoDismissed = $state(false);
  let openInfoDetails = $state(false);
  let shownWarnings = $derived((snap?.warnings ?? []).filter((w) => !dismissedWarnings.includes(w)));
  let openInfo = $derived((snap?.open_notes ?? []).filter((n) => n.kind === "info"));
  /** True when opening also left something to do: then the row does not say "Nothing was lost". */
  let openHasActions = $derived((snap?.open_notes ?? []).some((n) => n.kind === "action"));
  function freshNotes() {
    dismissedWarnings = [];
    openInfoDismissed = false;
    openInfoDetails = false;
  }

  /** A snapshot from "Start from a river": a new study keeps no selection from the old one. */
  function openDocDone(next: Snapshot) {
    if (next.document.project.title !== snap?.document.project.title) {
      selectedId = null;
      station = null;
      freshNotes();
    }
    snap = next;
  }

  /** New and Open start a different study: its first option (a failing one, if any) is selected, not the old selection. */
  async function openDoc(action: () => Promise<Snapshot | null>) {
    if (fileRunning) return;
    fileRunning = true;
    fileBusy = true;
    try {
      await flush();
      const next = await guard(action);
      if (!next) return;
      selectedId = null;
      station = null;
      freshNotes();
      snap = next;
    } finally {
      fileRunning = false;
      fileBusy = false;
      canPick = detectPick();
    }
  }

  function onKey(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey)) return;
    const k = e.key.toLowerCase();
    if (k === "s") {
      e.preventDefault();
      saveButton(null, e.shiftKey ? api.saveAs : api.save);
    } else if (k === "o") {
      e.preventDefault();
      openDoc(api.openFile);
    } else if (k === "n") {
      e.preventDefault();
      openDoc(api.newFile);
    }
  }

  // The option table and checks list come from the session too, and refresh with every change.
  $effect(() => {
    void snap?.document;
    api.optionTable().then(
      (t) => {
        table = t;
        // Keep the selection while its option exists; otherwise select the first row (a failure, if any).
        if (!t.rows.some((r) => r.id === selectedId)) selectedId = t.rows[0]?.id ?? null;
      },
      () => (table = null),
    );
  });

  // The long section of the selected option, drawn from the station walk the volumes summed. It is
  // asked for again whenever the design or the selection changes; a refusal says why instead.
  let sectionRequest = 0;
  $effect(() => {
    const id = selectedId;
    void snap?.document;
    const mine = ++sectionRequest;
    if (!id) {
      longSection = null;
      crossSection = null;
      return;
    }
    api.longSection(id).then(
      (ls) => {
        if (mine !== sectionRequest) return;
        longSection = ls;
        sectionNote = null;
        if (station == null || station > ls.chainage_m[ls.chainage_m.length - 1]) station = ls.chainage_m[0];
      },
      (e) => {
        if (mine !== sectionRequest) return;
        longSection = null;
        crossSection = null;
        sectionNote = sectionReason(String(e), snap?.stages);
      },
    );
  });

  // The cross-section at the station. It is asked for when the slider settles, not at every step
  // of its travel: one section can take a while (a station with no stored profile is cut again on
  // the DEM). The panel says it is working meanwhile, and a refusal says why. Latest request wins.
  const CROSS_SETTLE_MS = 200;
  let crossRequest = 0;
  let crossBusy = $state(false);
  let crossNote = $state<string | null>(null);
  $effect(() => {
    const id = selectedId;
    const at = station;
    void longSection;
    // With both panels hidden nothing shows a section: none is asked for.
    if (!id || at == null || !longSection || !(profileOpen || sectionOpen)) {
      crossBusy = false;
      return;
    }
    const mine = ++crossRequest;
    crossBusy = true;
    const settle = setTimeout(() => {
      api.crossSection(id, at).then(
        (cs) => {
          if (mine !== crossRequest) return;
          crossSection = cs;
          crossNote = null;
          crossBusy = false;
        },
        (e) => {
          if (mine !== crossRequest) return;
          crossSection = null;
          crossNote = String(e).replace(/^Error:\s*/, "");
          crossBusy = false;
        },
      );
    }, CROSS_SETTLE_MS);
    return () => clearTimeout(settle);
  });

  /** The slider moves between the stations the session drew. */
  let stationIndex = $derived(longSection && station != null ? longSection.chainage_m.findIndex((c) => c >= station! - 1e-9) : 0);
  function slide(i: number) {
    if (longSection) station = longSection.chainage_m[i];
  }
  function pickChainage(c: number) {
    if (longSection) station = longSection.chainage_m.reduce((best, x) => (Math.abs(x - c) < Math.abs(best - c) ? x : best));
  }
  /** How near the design line a map click must be to count as a click on it, in screen pixels. */
  const LINE_REACH_PX = 10;
  /** True when a map point is on the design line as the map draws it. */
  function onDesignLine(lon: number, lat: number): boolean {
    if (!map) return false;
    try {
      const p = map.project([lon, lat]);
      const layers = ["options", "options-casing", "ticks"].filter((id) => map!.getLayer(id));
      const box: [maplibregl.PointLike, maplibregl.PointLike] = [[p.x - LINE_REACH_PX, p.y - LINE_REACH_PX], [p.x + LINE_REACH_PX, p.y + LINE_REACH_PX]];
      return layers.length > 0 && map.queryRenderedFeatures(box, { layers }).length > 0;
    } catch {
      return false;
    }
  }
  /** A map click. On the design line: go to the nearest station and show both panels. Elsewhere it
   * moves the station only while a panel shows it; with both hidden the map is left alone. */
  async function pickPoint(lon: number, lat: number) {
    if (!selectedId || !longSection) return;
    const onLine = onDesignLine(lon, lat);
    if (!onLine && !(profileOpen || sectionOpen)) return;
    const c = await guard(() => api.stationNear(selectedId!, lon, lat));
    if (c == null) return;
    station = c;
    if (onLine) {
      profileOpen = true;
      sectionOpen = true;
    }
  }

  /** An option's colour (a CSS reference to its token): its place in the design's list, which is the order the map draws. */
  const indexOf = (id: string) => Math.max(geometry?.options.findIndex((o) => o.id === id) ?? 0, 0);
  const colourOf = (id: string) => optionVar(indexOf(id));

  // The hillshade is drawn once by the Rust side, and only when the imagery is offline. A new DEM
  // reference clears it.
  $effect(() => {
    const key = snap?.document.inputs.dem ? `${snap.document.inputs.dem.path}:${snap.document.inputs.dem.sha256}` : null;
    if (key !== shadeFor) {
      shadeFor = null;
      hillshade = null;
    }
    if (offline && shadeFor === null) {
      shadeFor = key ?? "";
      api.hillshade().then(
        (h) => (hillshade = h),
        () => (hillshade = null),
      );
    }
  });

  // The DEM as terrain for the 3D view (MAP-2). The engine knows which DEM is in memory: it is asked
  // after every snapshot (the answer is kept there, so asking is cheap), and the map is given a new
  // answer only when it changes. The desktop app has no tile path yet: it says so on the button.
  const TERRAIN_NOTE = isWeb ? null : "3D: in the web app only for now";
  $effect(() => {
    void snap;
    if (!isWeb || !snap) return;
    api.terrainInfo().then(
      (t) => {
        if (JSON.stringify(t) !== JSON.stringify(untrack(() => terrain))) terrain = t;
      },
      () => {},
    );
  });

  // The channel and its cut for the map (MAP-5): derived by the engine from the design, asked for after
  // every snapshot (the engine keeps its answer until the design changes), and given to the map only
  // when it changes. The desktop app has no path for it yet: its ticks say so.
  $effect(() => {
    void snap;
    if (!isWeb || !snap) return;
    api.channelGeometry().then(
      (c) => {
        if (c?.key !== untrack(() => channel)?.key) channel = c;
      },
      () => {},
    );
  });

  onMount(() => {
    const undo: (() => void)[] = [];
    // Outside the Tauri window (a browser test of the desktop build) these have nothing to listen to.
    platform()
      .onProgress((n) => (samples = n))
      .then((un) => undo.push(un))
      .catch(() => {});
    platform()
      .onFileDrop((e) => {
        if (e.type === "leave") dropTarget = null;
        else if (e.type === "over") dropTarget = rowAt(e.x, e.y);
        else {
          const role = rowAt(e.x, e.y);
          dropTarget = null;
          // A GeoTIFF is the DEM wherever it lands; vector data must land on its own row.
          if (/\.tiff?$/i.test(e.name)) load("dem", e.file);
          else if (role) load(role, e.file);
          else error = "Drop the file on the row it belongs to: left bank, right bank, river polygon, centreline, buildings, places or settlements.";
        }
      })
      .then((un) => undo.push(un))
      .catch(() => {});
    guard(async () => {
      tiles = (await api.basemapTiles()) ?? BASEMAP_TILES;
      choices = await api.sizingChoices();
      snap = await api.snapshot();
    });
    // The project folder of last time, if this browser kept it: offered, never opened by itself.
    if (canProject) api.projectRemembered().then((last) => (lastProject = last)).catch(() => {});
    return () => undo.forEach((u) => u());
  });

  /** Drag the rule between the left panel and the map. */
  function dragLeft(down: PointerEvent) {
    const target = down.currentTarget as HTMLElement;
    target.setPointerCapture(down.pointerId);
    const move = (e: PointerEvent) => (leftWidth = Math.min(Math.max(e.clientX, 220), window.innerWidth - 300));
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  }

  /** The key of the custom vessel's row in the "what fits" answer, which has no class id. */
  const CUSTOM_ROW = "custom";

  /** "Use this vessel": choose the answer's Rule Set and size the channel for the class, then back to today's flow (alignment, then volumes). */
  async function useVessel(choice: string, selection: FitsSelection | null, vesselStandardId: string) {
    await flush();
    if (selection) {
      const inputs = Object.fromEntries(selection.set_inputs.map((i) => [i.name, i.value]));
      const chosen = await guard(() => api.setRuleSets(selection.rule_sets, inputs));
      if (!chosen) return;
      snap = chosen;
    }
    // The custom vessel's row (spec 0015) sizes the design's own custom vessel again.
    const stored = snap?.document.stages.sizing.settings?.custom_vessel;
    const custom = vesselStandardId === CUSTOM_ROW && stored;
    const next = await guard(() => (custom ? api.setCustomVessel(api.customFields(custom)) : api.setVessel(vesselStandardId, "")));
    if (!next) return;
    snap = next;
    reachView = false;
    const named = custom ? (next.document.stages.sizing.results?.vessel_label ?? "the custom vessel") : vesselStandardId;
    notice = `Vessel set: ${named} under ${choice}. Next: run the alignment, then the volumes.`;
  }

  let theme = $state<ThemeChoice>(currentTheme());

  let anyStale = $derived((snap?.due.length ?? 0) > 0);
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <header>
    <span class="brand"><img class="mark-light" src={markLight} alt="OWD" /><img class="mark-dark" src={markDark} alt="OWD" /></span>
    <button onclick={() => openDoc(api.newFile)}>New</button>
    {#if isWeb}<button onclick={() => openDoc(api.openExample)} data-testid="open-example" title="A short synthetic river with its DEM">Open the example</button>{/if}
    <button onclick={() => openDoc(api.openFile)}>Open</button>
    {#if isWeb}<button data-testid="open-project" disabled={fileBusy} aria-disabled={!canProject} title={canProject ? "Open a project folder: OWD reads its owd-project.json, opens the design and loads the DEM in one step. Nothing is uploaded." : NO_PROJECT} onclick={() => (canProject ? chooseProject(api.projectPick) : (notice = NO_PROJECT))}>{canProject ? "Open project" : "Open project (Chrome or Edge only)"}</button>{/if}
    <button class:on={reachView} data-testid="start-from-river" aria-pressed={reachView} onclick={() => (reachView = !reachView)} title="Make a study's inputs from open data for a river reach">Start from a river</button>
    <button disabled={fileBusy} data-testid="save" title={SAVE_TITLE} onclick={(e) => saveButton(e, api.save)}>Save</button>
    <button disabled={fileBusy} data-testid="save-as" title={SAVE_AS_TITLE} onclick={(e) => saveButton(e, api.saveAs)}>Save as</button>
    <button class:on={sheetsView} data-testid="sheets-toggle" aria-pressed={sheetsView} disabled={!(table?.rows.length)} onclick={() => (sheetsView = !sheetsView)} title="The drawing sheets of the active option, as owd draw makes them">Sheets</button>
    <button bind:this={profileToggle} class:on={profileOpen} data-testid="profile-toggle" aria-pressed={profileOpen} onclick={() => togglePanel("profile")} title="Show or hide the profile (the long section) over the map">Profile</button>
    <button bind:this={sectionToggle} class:on={sectionOpen} data-testid="section-toggle" aria-pressed={sectionOpen} onclick={() => togglePanel("section")} title="Show or hide the cross-section at the station over the map">Section</button>
    <button disabled={!(table?.rows.length)} onclick={() => exportAs("geojson")} title="Design line options as GeoJSON (WGS84), with their checks">Export GeoJSON</button>
    <button disabled={!(table?.rows.length)} onclick={() => exportAs("csv")} title="The quantity table as CSV">Export CSV</button>
    <button class="primary" disabled={!anyStale || busy !== null} onclick={runStale} title="Run every stage that is due (stale, or set up and not run yet), in order">Run stale</button>
    <span class="grow"></span>
    {#if tiles}<ImageryControl bind:this={imageryControl} openTiles={tiles} bind:layer={imagery} bind:note={imageryNote} />{/if}
    <label class="theme">
      Theme
      <select aria-label="Theme" bind:value={theme} onchange={() => setTheme(theme)}>
        <option value="auto">Auto</option>
        <option value="light">Light</option>
        <option value="dark">Dark</option>
      </select>
    </label>
    {#if isWeb}
      <span class="meta" data-testid="web-label">Web version: runs in this browser, nothing is uploaded. <a href="/owd/docs#install">Get the desktop app</a></span>
    {/if}
    {#if snap}
      <span class="meta" data-testid="file-label">{snap.file_name}{snap.dirty ? " (unsaved)" : ""}</span>
      <span class="meta" data-testid="study-title">{snap.document.project.title}</span>
      <span class="meta" data-testid="engine-version">Engine {snap.engine_version}</span>
    {/if}
  </header>

  {#if error}
    <div class="banner error" role="alert" data-testid="error">
      {plainWarning(error)}
      <button onclick={() => (error = null)}>Dismiss</button>
    </div>
  {/if}
  {#if notice}
    <div class="banner warn" role="status" data-testid="notice">
      {notice}
      <button onclick={() => (notice = null)}>Dismiss</button>
    </div>
  {/if}
  {#if projectProgress}
    <div class="banner info" role="status" data-testid="project-progress">{projectProgress}</div>
  {/if}
  {#if lastProject && !project}
    <div class="banner info" role="status" data-testid="reopen-banner">
      <span>Your last project folder: {lastProject.folder}/. Your browser will ask you to allow it again.</span>
      <button data-testid="reopen-project" disabled={fileBusy} onclick={() => chooseProject(api.projectReopen)}>Reopen {lastProject.title}</button>
      <button data-testid="reopen-dismiss" onclick={() => (lastProject = null)}>Not now</button>
      <button data-testid="reopen-forget" onclick={() => api.projectForget().catch(() => {}).finally(() => (lastProject = null))} title="OWD will not offer this folder again">Forget it</button>
    </div>
  {/if}
  {#if imageryNote}
    <div class="banner warn" role="status" data-testid="imagery-note">
      {imageryNote}
      <button onclick={() => (imageryNote = null)}>Dismiss</button>
    </div>
  {/if}
  {#each shownWarnings as w (w)}
    <div class="banner warn" role="status" data-testid="warning">
      <span>{plainWarning(w)}</span>
      <button data-testid="warning-dismiss" onclick={() => (dismissedWarnings = [...dismissedWarnings, w])}>Dismiss</button>
    </div>
  {/each}
  {#if openInfo.length && !openInfoDismissed}
    <div class="banner info" role="status" data-testid="open-info">
      <span>Opened from an older format.{openHasActions ? "" : " Nothing was lost."}</span>
      <button data-testid="open-info-details" aria-expanded={openInfoDetails} onclick={() => (openInfoDetails = !openInfoDetails)}>{openInfoDetails ? "Hide details" : "Details"}</button>
      <button data-testid="open-info-dismiss" onclick={() => (openInfoDismissed = true)}>Dismiss</button>
      {#if openInfoDetails}
        <ul data-testid="open-info-list">
          {#each openInfo as n}<li data-testid="open-info-note">{plainNote(n.text)}</li>{/each}
        </ul>
      {/if}
    </div>
  {/if}

  <div class="body" style:grid-template-columns="{leftWidth}px 6px 1fr">
    <section class="left">
      {#if reachView}
        <StartFromRiver {map} {snap} onSnap={(next) => openDocDone(next)} onError={(m) => (error = m)} onUseVessel={useVessel} bind:picker={reachPicker} />
      {:else}
        <LeftPanel {snap} {choices} {onRuleSets} {onVessel} {onCustomVessel} {busy} {samples} {dropTarget} onRun={run} onCancel={cancel} onUseWater={useWater} onLoad={(r) => load(r)} onFetch={fetchDem} onWater={water} onTerrainLimit={terrainLimit} onDownstream={downstreamPoint} onMouth={riverMouth} onRunInputs={runInputs} onOptimiseLength={optimiseLength} onOptimiseFlood={optimiseFlood} onOptimiseLand={optimiseLand} onCorridorLimit={corridorLimit} onJoinSetting={joinSetting} {lineName} {table} {indexOf} {selectedId} onSelect={(id) => (selectedId = id)} {stretchesOpen} onStretches={toggleStretches} {project} onProjectLoad={loadProjectItem} onFeatureAction={(a) => fileAction(a)} />
      {/if}
    </section>
    <div class="handle v" role="separator" aria-orientation="vertical" onpointerdown={dragLeft}></div>
    <section class="main">
      {#if sheetsView}<SheetsView option={selectedId} onNotice={(m) => (notice = m)} onError={(m) => (error = m)} />{/if}
      <div class="maparea" data-testid="map-area">
      <div class="mapbox">
        {#if tiles}<MapPanel {geometry} {selectedId} {stretches} {selectedStretch} {hillshade} {terrain} {channel} terrainNote={TERRAIN_NOTE} {imagery} onImageryFail={(src) => imageryControl?.fail(src)} stationPosition={profileOpen || sectionOpen ? (crossSection?.position ?? null) : null} onPick={editingBanks ? undefined : reachView && reachPicker ? reachPicker : pickPoint} onReady={(m) => (map = m)} bind:offline features={snap?.feature_inputs} />{/if}
        <BankEditor {map} {geometry} {snap} onSnap={(next) => (snap = next)} bind:active={editingBanks} />
        {#if snap?.notices?.length}
          <ul class="notices" data-testid="map-notices" aria-label="Data notices">
            {#each snap.notices as n}<li>{n}</li>{/each}
          </ul>
        {/if}
      </div>
      {#if profileOpen}
        <FloatPanel bind:this={profilePanel} id="profile" title="Profile" place={(w, h) => ({ x: 12, y: h - 260 - 56, w: Math.min(560, w * 0.5 - 18), h: 260 })} onClose={() => (profileOpen = false)} returnFocus={profileToggle}>
          <div class="longbox">
            {#if longSection}
              <LongSection data={longSection} {station} colour={selectedId ? colourOf(selectedId) : "var(--ink)"} onPick={pickChainage} />
            {:else}
              <p class="slot" data-testid="section-note">{sectionNote ?? "Run volumes to see the long section."}</p>
            {/if}
          </div>
        </FloatPanel>
      {/if}
      {#if sectionOpen}
        <FloatPanel bind:this={sectionPanel} id="section" title="Cross-section" place={(w, h) => { const pw = Math.min(520, w * 0.5 - 18); return { x: w - pw - 12, y: h - 300 - 56, w: pw, h: 300 }; }} onClose={() => (sectionOpen = false)} returnFocus={sectionToggle}>
          <label class="slider">
            Station
            <input
              type="range"
              data-testid="station-slider"
              aria-label="Station"
              min="0"
              max={Math.max((longSection?.chainage_m.length ?? 1) - 1, 0)}
              step="1"
              disabled={!longSection}
              value={stationIndex < 0 ? 0 : stationIndex}
              oninput={(e) => slide(Number(e.currentTarget.value))}
            />
          </label>
          <div class="csbody">
            {#if !longSection && sectionNote}
              <p class="slot" data-testid="cross-note">{sectionNote}</p>
            {:else}
              <CrossSection section={crossSection} busy={crossBusy} note={crossNote} colour={selectedId ? colourOf(selectedId) : "var(--ink)"} />
            {/if}
          </div>
        </FloatPanel>
      {/if}
      </div>
      {#if stretchesOpen}
        <StretchesDrawer {table} {snap} {stretches} {selectedId} {selectedStretch} onSelectStretch={(s) => (selectedStretch = s)} onClose={() => (stretchesOpen = false)} returnFocus={document.querySelector<HTMLElement>("[data-testid=stretches-toggle]")} />
      {/if}
    </section>
  </div>

  {#if projectLook}<ProjectDialog look={projectLook} busy={fileBusy} onUse={adoptProject} onClose={() => (projectLook = null)} />{/if}

  <footer data-testid="status-line">Screening — not for construction</footer>
</div>

<style>
  .shell { display: flex; flex-direction: column; height: 100vh; }
  header { display: flex; gap: var(--sp-2); align-items: center; padding: var(--sp-2) var(--sp-3); border-bottom: var(--lw-thin) solid var(--ink); background: var(--paper); }
  .brand { display: flex; align-items: center; margin-right: var(--sp-3); }
  .brand img { height: 26px; }
  .mark-dark { display: none; }
  .grow { flex: 1; }
  .theme { display: flex; gap: var(--sp-1); align-items: center; font-size: var(--fs-12); color: var(--ink-3); }
  .theme select { padding: 2px 6px; font-size: var(--fs-12); }
  .meta { color: var(--ink-3); font-size: var(--fs-12); margin-left: var(--sp-3); font-variant-numeric: tabular-nums; }
  .banner { display: flex; gap: var(--sp-2); align-items: center; padding: 6px var(--sp-3); font-size: var(--fs-13); border-bottom: var(--lw-thin) solid var(--rule); }
  .banner.error { background: var(--fail-soft); color: var(--fail); }
  .banner.error::before { content: "✕"; font-weight: 700; }
  .banner.warn { background: var(--warn-soft); color: var(--warn); }
  .banner.warn::before { content: "!"; font-weight: 700; }
  .banner.info { flex-wrap: wrap; padding: 3px var(--sp-3); background: var(--surface); color: var(--ink-2); font-size: var(--fs-12); }
  .banner.info ul { flex-basis: 100%; margin: 0 0 var(--sp-1); padding-left: var(--sp-4, 16px); }
  .banner button { padding: 1px 8px; font-size: var(--fs-12); }
  .body { flex: 1; display: grid; grid-template-rows: minmax(0, 1fr); min-height: 0; }
  .body > section { overflow: auto; min-width: 0; min-height: 0; }
  .left { grid-column: 1; background: var(--surface); }
  .handle.v { grid-column: 2; cursor: col-resize; background: var(--ink); }
  /* The map fills the space beside the left panel; the panels float over it. */
  .body > .main { grid-column: 3; overflow: hidden; position: relative; display: flex; flex-direction: column; }
  .maparea { flex: 1; min-height: 120px; position: relative; overflow: hidden; }
  .mapbox { position: absolute; inset: 0; }
  .longbox { flex: 1; min-width: 0; min-height: 0; overflow: hidden; }
  .csbody { flex: 1; min-height: 0; min-width: 0; }
  .slider { display: flex; gap: var(--sp-2); align-items: center; padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); color: var(--ink-3); }
  .slider input { flex: 1; accent-color: var(--ink); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 10px; font-size: var(--fs-13); }
  .handle:hover { background: var(--ink-2); }
  .notices { position: absolute; right: var(--sp-2); bottom: 28px; z-index: 2; margin: 0; padding: 2px var(--sp-2); max-width: 60%; list-style: none; background: var(--surface); color: var(--ink-2); border: var(--lw-thin) solid var(--rule); font-size: var(--fs-11); }
  header button.on { background: var(--accent); color: var(--accent-ink); }
  footer { padding: var(--sp-1) var(--sp-3); background: var(--accent); color: var(--accent-ink); font: 600 var(--fs-12) var(--font-ui); letter-spacing: 0.04em; }

  /* The reversed mark shows on the dark theme: chosen by the OS, or by data-theme. */
  :global(:root[data-theme="dark"]) .mark-light { display: none; }
  :global(:root[data-theme="dark"]) .mark-dark { display: block; }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .mark-light { display: none; }
    :global(:root:not([data-theme="light"])) .mark-dark { display: block; }
  }
</style>
