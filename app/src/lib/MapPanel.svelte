<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?url";
  import type * as GeoJSON from "geojson";
  import { contourTile, cutAt, cutTile, mapStructures, poolSurfaceTile, poolWaterAt, poolWaterInfo, poolWaterTile, terrainTile, type Hillshade, type LonLat, type MapGeometry, type MapStructures, type PoolWaterInfo, type StretchCollection, type TerrainInfo, type WaterAt } from "./api";
  import { CUT_PROTOCOL, TERRAIN_STATES, classLabel, cutTemplate, loadTerrainState, otherState, parseCutUrl, readout, saveTerrainState, type ChannelView, type CutAt, type TerrainState } from "./channel";
  import { DEFAULT_IMAGERY, layerFor, type ImageryLayer, type ImagerySource } from "./imagery";
  import OptionSwatch from "./OptionSwatch.svelte";
  import { optionColour, token } from "./theme";
  import { MAP_LAYERS, STARTS_OFF, STYLE_LAYERS, loadLayerTicks, saveLayerTicks, type LayerTicks, type MapLayerId } from "./mapLayers";
  import { CONTOUR_DEEP_ZOOM, CONTOUR_MIN_ZOOM, CONTOUR_PROTOCOL, INTERVALS, LABEL_PREFIX, contourTemplate, intervalAt, loadContourChoice, parseContourUrl, saveContourChoice, type ContourChoice } from "./contours";
  import StructureCard from "./StructureCard.svelte";
  import { PLACEHOLDER_NOTE, POOL_PROTOCOL, POOL_SURFACE_PROTOCOL, extrusionFeatures, parsePoolUrl, partFeatures, poolFeatures, poolLabel, poolSurfaceTemplate, poolTemplate, stopColour, waterReadout, weirLabel } from "./structures";
  import { EXAGGERATIONS, PITCH_3D, TERRAIN_PROTOCOL, loadTerrainPrefs, parseTileUrl, saveTerrainPrefs, stateTileTemplate, type TerrainPrefs } from "./terrain3d";

  let {
    geometry,
    selectedId = null,
    stretches = null,
    selectedStretch = null,
    hillshade = null,
    terrain = null,
    terrainNote = null,
    channel = null,
    imagery = layerFor(DEFAULT_IMAGERY),
    onImageryFail,
    stationPosition = null,
    onPick,
    onReady,
    offline = $bindable(false),
  }: {
    geometry: MapGeometry | null;
    selectedId?: string | null;
    /** The constraint stretches as the session exports them (WGS84 lines). */
    stretches?: StretchCollection | null;
    /** The stretch picked in the table: drawn heavier, and the map moves to it. */
    selectedStretch?: { option: string; index: number } | null;
    /** The DEM hillshade, shown in place of the imagery while the tiles cannot load. */
    hillshade?: Hillshade | null;
    /** The DEM in memory as terrain tiles, for the 3D view and the terrain shading. Null with no DEM. */
    terrain?: TerrainInfo | null;
    /** Why this app cannot show the DEM in 3D at all (the desktop app, for now). Null where it can. */
    terrainNote?: string | null;
    /** The channel of the design line as the engine derives it (MAP-5): its edges, or why there are none. Null where the app cannot derive it. */
    channel?: ChannelView | null;
    /** The imagery to draw under the design, and its attribution. */
    imagery?: ImageryLayer;
    /** Called when imagery that is not the open default cannot load (a refused key, or tiles that keep failing). */
    onImageryFail?: (source: ImagerySource) => void;
    /** Where the section station is, as `[lon, lat]`: a marker follows it. */
    stationPosition?: [number, number] | null;
    /** A click on the map: the point under it. */
    onPick?: (lon: number, lat: number) => void;
    /** Called once with the map, when it has loaded: an editor adds its own layers and handlers. */
    onReady?: (map: maplibregl.Map) => void;
    /** True while the basemap tiles are failing. The parent reads it to fetch the hillshade. */
    offline?: boolean;
  } = $props();

  // The worker is served with the app (never from a CDN), so the map starts offline too.
  maplibregl.setWorkerUrl(workerUrl);

  // The terrain tiles (MAP-2) are made by the engine in this browser, from the DEM in memory: the map asks
  // the page for them under a scheme of its own, and no request leaves the page. The engine may be busy
  // (a stage is running) or starting again (a run was cancelled): a tile that fails is asked for again.
  const wait = (ms: number, signal: AbortSignal) =>
    new Promise<void>((done, stop) => {
      const t = setTimeout(done, ms);
      signal.addEventListener("abort", () => (clearTimeout(t), stop(new Error("the tile is no longer wanted"))), { once: true });
    });
  /** A tile from the engine, as the map wants it. A tile that fails is asked for again, five times in all. */
  async function engineTile(make: () => Promise<Uint8Array>, abort: AbortController): Promise<{ data: ArrayBuffer }> {
    for (let tries = 0; ; tries += 1) {
      try {
        const bytes = await make();
        return { data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer };
      } catch (e) {
        if (tries >= 4 || abort.signal.aborted) throw new Error(String(e));
        await wait(1000, abort.signal);
      }
    }
  }
  maplibregl.addProtocol(TERRAIN_PROTOCOL, (params, abort) => {
    const tile = parseTileUrl(params.url);
    if (!tile) throw new Error(`not a terrain tile address: ${params.url}`);
    return engineTile(() => terrainTile(tile.z, tile.x, tile.y, tile.state), abort);
  });
  // The contour tiles (MAP-3) come the same way: the engine traces the lines of one map tile from the
  // DEM, off this thread, and answers with a vector tile. The interval is the one for the tile's zoom level.
  maplibregl.addProtocol(CONTOUR_PROTOCOL, (params, abort) => {
    const tile = parseContourUrl(params.url);
    if (!tile) throw new Error(`not a contour tile address: ${params.url}`);
    return engineTile(() => contourTile(tile.z, tile.x, tile.y, intervalAt(tile.choice, tile.z), tile.state ?? "before"), abort);
  });
  // The cut-depth picture (MAP-5) comes the same way: a PNG for each map tile, coloured by the engine from
  // the cut surface and the DEM. A tile with no cut in it has no bytes: the map is given a clear picture.
  maplibregl.addProtocol(CUT_PROTOCOL, (params, abort) => {
    const tile = parseCutUrl(params.url);
    if (!tile) throw new Error(`not a cut-depth tile address: ${params.url}`);
    return engineTile(async () => {
      const bytes = await cutTile(tile.z, tile.x, tile.y);
      return bytes.length ? bytes : Uint8Array.from(atob(BLANK.slice(BLANK.indexOf(",") + 1)), (c) => c.charCodeAt(0));
    }, abort);
  });

  /** The colour of the water-surface edges and of the daylight lines: fixed, since they lie on imagery in both themes. */
  const WATER_EDGE = "#9ecae1";
  const DAYLIGHT = "#fdae6b";
  // The pool water (MAP-4) comes the same way, under two schemes: the picture of the water, coloured by
  // depth, and the terrain with the water on it, which the 3D view stands on while the water is shown.
  for (const scheme of [POOL_PROTOCOL, POOL_SURFACE_PROTOCOL]) {
    maplibregl.addProtocol(scheme, (params, abort) => {
      const tile = parsePoolUrl(params.url);
      if (!tile) throw new Error(`not a pool water tile address: ${params.url}`);
      return engineTile(() => (tile.surface ? poolSurfaceTile(tile.id, tile.z, tile.x, tile.y, tile.state ?? "before") : poolWaterTile(tile.id, tile.z, tile.x, tile.y)), abort);
    });
  }

  /** A 1 x 1 transparent PNG. */
  const BLANK = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

  let host: HTMLDivElement;
  let map: maplibregl.Map | undefined;
  let ready = $state(false);
  let failure = $state<string | null>(null);
  let labels: maplibregl.Marker[] = [];
  let bendMarkers: maplibregl.Marker[] = [];
  let stationMarker: maplibregl.Marker | undefined;
  /** The option the map has drawn as selected: what a test reads to know the map followed the table. */
  let drawn = $state<string | null>(null);
  /** The stretch the map has drawn as picked, as `option:index`, for the same reason. */
  let drawnStretch = $state("");

  /** The layers ticked on in the Layers list: remembered on this machine, never in the design. */
  let ticks = $state<LayerTicks>(loadLayerTicks());
  let layersOpen = $state(false);
  /** The weirs, the locks and the pools of the design (MAP-4): asked of the session whenever the design
   * changes. It is made on demand and never stored; a design with no pools has empty lists. */
  let structures = $state<MapStructures | null>(null);
  let structuresText = "";
  let structuresAsk = 0;
  $effect(() => {
    void geometry;
    const ask = ++structuresAsk;
    const take = (next: MapStructures | null) => {
      const text = JSON.stringify(next);
      if (ask !== structuresAsk || text === structuresText) return;
      structuresText = text;
      structures = next;
    };
    mapStructures().then(take, () => take(null));
  });
  /** The layers this design has something to draw for. Imagery is always there. */
  const present = $derived<Record<MapLayerId, boolean>>({
    "river-polygon": !!geometry?.river_polygon,
    centreline: !!geometry?.centreline,
    "bank-lines": !!geometry?.banks.length,
    "design-line": !!geometry?.options.length,
    channel: !!channel?.geometry,
    cut: !!channel?.geometry?.daylight && terrain?.cut != null,
    "constraint-stretches": !!stretches?.features.length,
    "station-marks": !!geometry?.options.some((o) => o.ticks.length),
    structures: !!structures?.weirs.length,
    "pool-levels": !!structures?.pools.length,
    "pool-water": !!structures?.pools.length && !!terrain,
    "terrain-shading": !!terrain,
    contours: !!terrain,
    imagery: true,
  });

  /** 3D on or off and the vertical exaggeration: remembered on this machine, never in the design. */
  let prefs = $state<TerrainPrefs>(loadTerrainPrefs());
  /** The map is in 3D: the user asked for it and there is a DEM to show. */
  const terrainOn = $derived(prefs.on && !!terrain);
  function setPrefs(next: TerrainPrefs) {
    prefs = next;
    saveTerrainPrefs(next);
  }
  /** The ground the map shows (MAP-5): as it is, or with the channel cut into it. Remembered on this machine, never in the design. */
  let terrainState = $state<TerrainState>(loadTerrainState());
  /** True while Compare is held: the map shows the other state until it is let go. */
  let peek = $state(false);
  /** There is a cut to show: the volumes are fresh on the DEM in memory. */
  const hasCut = $derived(terrain?.cut != null);
  /** The state the map shows now: After only while there is a cut to show. */
  const shownState = $derived<TerrainState>(!hasCut ? "before" : peek ? otherState(terrainState) : terrainState);
  function setTerrainState(next: TerrainState) {
    terrainState = next;
    saveTerrainState(next);
  }
  /** The DEM and the cut whose tiles the map has now, by their numbers; "" for none. */
  let drawnTerrain = "";
  /** The shading layers the map has now, by the tiles and the state they are drawn from; "" for none. */
  let drawnShade = "";
  /** Whether the map is tilted for 3D now, so the camera moves only when 3D goes on or off. */
  let tilted = false;
  /** What the map itself has done, for a test to read: "on" once the terrain is set. */
  let terrainDrawn = $state("off");
  /** The state of the ground the map itself draws from now, for a test to read. */
  let stateDrawn = $state<TerrainState>("before");
  /** Give the map the DEM's tiles (or take them away), then set 3D, the state, the exaggeration and the shading. */
  function applyTerrain() {
    if (!map || !ready) return;
    const m = map;
    const key = terrain ? `${terrain.id}/${terrain.cut ?? ""}` : "";
    const unshade = () => {
      for (const id of ["terrain-tint", "terrain-shading"]) if (m.getLayer(id)) m.removeLayer(id);
      drawnShade = "";
    };
    if (key !== drawnTerrain) {
      // Another DEM or another cut, or none: everything drawn from the old tiles goes.
      if (m.getTerrain()) m.setTerrain(null);
      dropPoolWater();
      unshade();
      for (const id of ["dem", "dem-shade", "dem-after", "dem-after-shade"]) if (m.getSource(id)) m.removeSource(id);
      if (terrain) {
        // Two sources of the same tiles: the map's own advice, so the 3D surface and the shading
        // each keep their own tiles. The engine makes each tile once and answers both.
        const t = terrain;
        const source = (state: TerrainState): maplibregl.RasterDEMSourceSpecification => ({ type: "raster-dem", tiles: [stateTileTemplate(t.id, state, t.cut ?? 0)], tileSize: t.tile_size, maxzoom: t.max_zoom, encoding: t.encoding });
        m.addSource("dem", source("before"));
        // The shading is drawn only where the DEM is: outside its box there is no terrain to shade.
        m.addSource("dem-shade", { ...source("before"), bounds: t.bounds });
        // The ground with the channel cut into it (MAP-5): a second pair of sources, so Before and After
        // each keep their tiles and a change of state asks the engine for nothing twice.
        if (t.cut != null) {
          m.addSource("dem-after", source("after"));
          m.addSource("dem-after-shade", { ...source("after"), bounds: t.bounds });
        }
      }
      drawnTerrain = key;
    }
    const after = shownState === "after" && !!m.getSource("dem-after");
    const shadeKey = terrain ? `${key}/${after ? "after" : "before"}` : "";
    if (shadeKey !== drawnShade) {
      unshade();
      if (terrain) {
        // Over the imagery, under the design. The tint colours the ground by its height, low ground dark
        // and high ground pale, between the DEM's lowest and highest height: flat river country has little
        // slope to shade, so without the imagery the tint is what shows the DEM. A layer cannot change its
        // source, so the two are made again for the other state.
        const [low, mid, high] = [token("plate-ink"), token("hillshade"), token("cream")];
        const span = Math.max(terrain.max_m - terrain.min_m, 0.01);
        const from = after ? "dem-after-shade" : "dem-shade";
        // Under the pool water and the contours when they are on the map.
        const over = ["pool-water", "contours-casing", "polygon"].find((id) => m.getLayer(id));
        m.addLayer({ id: "terrain-tint", type: "color-relief", source: from, layout: { visibility: "none" }, paint: { "color-relief-color": ["interpolate", ["linear"], ["elevation"], terrain.min_m, low, terrain.min_m + span / 2, mid, terrain.max_m, high] } }, over);
        m.addLayer({ id: "terrain-shading", type: "hillshade", source: from, layout: { visibility: "none" }, paint: { "hillshade-exaggeration": 0.6, "hillshade-shadow-color": low, "hillshade-highlight-color": high } }, over);
      }
      drawnShade = shadeKey;
    }
    if (terrainOn) {
      // While the pool water is shown the 3D view stands on the terrain with the water on it: the pool's
      // level where there is water, the ground of the state shown elsewhere. So each pool reads as a flat surface.
      const surface = ticks["pool-water"] && m.getSource("dem-water") ? "dem-water" : after ? "dem-after" : "dem";
      m.setTerrain({ source: surface, exaggeration: prefs.exaggeration });
      if (!tilted) m.easeTo({ pitch: PITCH_3D, duration: 600 });
    } else {
      if (m.getTerrain()) m.setTerrain(null);
      // Back to the flat view, north up.
      if (tilted) m.easeTo({ pitch: 0, bearing: 0, duration: 600 });
    }
    tilted = terrainOn;
    terrainDrawn = m.getTerrain() ? "on" : "off";
    stateDrawn = after ? "after" : "before";
    waterTerrain = map.getTerrain()?.source === "dem-water" ? "on" : "off";
    // The blocks of the weirs and locks stand up only in the 3D view.
    if (map.getLayer("structures-3d")) map.setLayoutProperty("structures-3d", "visibility", ticks.structures && terrainOn ? "visible" : "none");
    raiseBlocks();
    // With the imagery: the shading alone, laid over it. Without: the tint under the shading, the DEM itself.
    const shaded = ticks["terrain-shading"];
    if (m.getLayer("terrain-shading")) m.setLayoutProperty("terrain-shading", "visibility", shaded ? "visible" : "none");
    if (m.getLayer("terrain-tint")) m.setLayoutProperty("terrain-tint", "visibility", shaded && !ticks.imagery ? "visible" : "none");
    applyContours();
    applyCut();
  }

  /** The cut-depth picture the map has now, as the DEM's and the cut's numbers; "" for none. */
  let drawnCut = "";
  /** What the map itself has done, for a test to read: "on" once the cut-depth layer is there. */
  let cutDrawn = $state("off");
  /** Put the cut-depth picture on the map, or take it off (MAP-5). It is a raster layer, so in 3D the map
   * drapes it on whichever ground is shown. It is made only while the tick is on. */
  function applyCut() {
    if (!map || !ready) return;
    const key = terrain && terrain.cut != null && ticks.cut && channel?.geometry?.daylight ? `${terrain.id}/${terrain.cut}` : "";
    if (key !== drawnCut) {
      if (map.getLayer("cut-depth")) map.removeLayer("cut-depth");
      if (map.getSource("cut")) map.removeSource("cut");
      if (terrain && terrain.cut != null && key) {
        // Further out than zoom 8 a channel is thinner than a pixel: nothing is asked for there.
        map.addSource("cut", { type: "raster", tiles: [cutTemplate(terrain.id, terrain.cut)], tileSize: 256, minzoom: 8, maxzoom: Math.max(terrain.max_zoom, 9), bounds: terrain.bounds });
        // Over the imagery, the shading, the contours and the river polygon; under the lines of the design.
        map.addLayer({ id: "cut-depth", type: "raster", source: "cut", paint: { "raster-opacity": 0.85, "raster-resampling": "nearest", "raster-fade-duration": 0 } }, "banks");
      }
      drawnCut = key;
      cutDrawn = key ? "on" : "off";
    }
    if (!key) cutRead = null;
  }
  /** What the last click on the cut read out; null for none. */
  let cutRead = $state<CutAt | null>(null);
  /** A click on the map while the cut extent is shown: read the cut out at the point. */
  function readCut(lon: number, lat: number) {
    if (!drawnCut) return;
    cutAt(lon, lat).then(
      (c) => (cutRead = c),
      () => (cutRead = null),
    );
  }

  /** Draw the channel's edges and the daylight lines the engine derived (MAP-5). */
  function drawChannel() {
    if (!map || !ready) return;
    const g = channel?.geometry;
    const rings = (e: { polygons: GeoJSON.Position[][] } | null | undefined) => (e?.polygons ?? []).map((ring) => feature({ type: "Polygon", coordinates: [ring] }));
    const lines = (e: { left: GeoJSON.Position[][]; right: GeoJSON.Position[][] } | null | undefined, kind: string) => [...(e?.left ?? []), ...(e?.right ?? [])].map((l) => feature({ type: "LineString", coordinates: l }, { kind }));
    (map.getSource("channel-bed") as maplibregl.GeoJSONSource).setData(fc(rings(g?.bed)));
    (map.getSource("channel-lines") as maplibregl.GeoJSONSource).setData(fc([...lines(g?.bed, "bed"), ...lines(g?.water_surface, "water")]));
    (map.getSource("cut-daylight") as maplibregl.GeoJSONSource).setData(fc(lines(g?.daylight, "daylight")));
    channelDrawn = g ? `${g.bed.polygons.length}` : "";
    untrack(applyCut);
  }
  /** The parts of the channel bed the map has drawn, for a test to read; "" for none. */
  let channelDrawn = $state("");

  /** The pool water (MAP-4): the ground each pool floods. The engine works it out in this browser, from the
   * DEM in memory, only while the tick is on; null until then, and with no DEM or no pools. */
  let poolWater = $state<PoolWaterInfo | null>(null);
  /** What the pool water is waiting for, or why it could not be made. */
  let poolWaterNote = $state<string | null>(null);
  let poolAsk = 0;
  const poolWaterWanted = $derived(ticks["pool-water"] && !!terrain && !!structures?.pools.length);
  $effect(() => {
    void structures;
    void terrain?.id;
    const ask = ++poolAsk;
    // Another DEM or other pools: the water on the map is out of date.
    poolWater = null;
    waterRead = null;
    if (!poolWaterWanted) {
      poolWaterNote = null;
      return;
    }
    poolWaterNote = "Working out the pool water…";
    poolWaterInfo().then(
      (info) => {
        if (ask !== poolAsk) return;
        poolWater = info;
        poolWaterNote = null;
      },
      (e) => {
        if (ask !== poolAsk) return;
        poolWaterNote = `The pool water could not be worked out: ${e}`;
      },
    );
  });
  /** The pool water the map has now, as the DEM's number and the flood mask's; "" for none. */
  let drawnPool = "";
  /** What the map itself has done, for a test to read: "on" once the pool water layer is there. */
  let poolWaterDrawn = $state("off");
  /** "on" while the 3D view stands on the terrain with the water on it. */
  let waterTerrain = $state("off");
  /** Take the pool water off the map. */
  function dropPoolWater() {
    if (!map) return;
    if (map.getTerrain()?.source === "dem-water") map.setTerrain(null);
    if (map.getLayer("pool-water")) map.removeLayer("pool-water");
    for (const id of ["pool-water", "dem-water"]) if (map.getSource(id)) map.removeSource(id);
    drawnPool = "";
    poolWaterDrawn = "off";
  }
  /** Put the pool water on the map, or take it off: the picture of the water over the ground, and the
   * terrain with the water on it for the 3D view. */
  function applyPoolWater() {
    if (!map || !ready) return;
    // The terrain with the water on it is of the ground shown: with After, the cut channel filled to the pool level.
    const state = shownState === "after" && terrain?.cut != null ? "after" : "before";
    const key = poolWater && terrain ? `${terrain.id}/${poolWater.id}/${state}/${terrain.cut ?? ""}` : "";
    if (key !== drawnPool) {
      dropPoolWater();
      if (poolWater && terrain) {
        map.addSource("pool-water", { type: "raster", tiles: [poolTemplate(poolWater.id)], tileSize: terrain.tile_size, maxzoom: terrain.max_zoom, bounds: terrain.bounds });
        map.addSource("dem-water", { type: "raster-dem", tiles: [poolSurfaceTemplate(poolWater.id, state, terrain.cut ?? 0)], tileSize: terrain.tile_size, maxzoom: terrain.max_zoom, encoding: terrain.encoding });
        // Over the imagery and the terrain shading, under the contours and the design.
        map.addLayer({ id: "pool-water", type: "raster", source: "pool-water", paint: { "raster-fade-duration": 0 } }, ["contours-casing", "polygon"].find((id) => map!.getLayer(id)));
        drawnPool = key;
        poolWaterDrawn = "on";
      }
    }
    applyTerrain();
  }
  /** What the last click on the pool water read: the water level, the ground, the depth and the pool. */
  let waterRead = $state<WaterAt | null>(null);
  let readAsk = 0;
  function readWater(lon: number, lat: number) {
    const ask = ++readAsk;
    if (!poolWater || !ticks["pool-water"]) {
      waterRead = null;
      return;
    }
    poolWaterAt(poolWater.id, lon, lat).then(
      (w) => ask === readAsk && (waterRead = w),
      () => ask === readAsk && (waterRead = null),
    );
  }
  /** The flooded land table is open. */
  let landOpen = $state(false);

  /** The weir whose card is open, by its number. */
  let selectedWeir = $state<number | null>(null);
  const weirShown = $derived(ticks.structures ? (structures?.weirs.find((w) => w.number === selectedWeir) ?? null) : null);
  let weirMarkers: maplibregl.Marker[] = [];
  let poolMarkers: maplibregl.Marker[] = [];
  /** The weirs the map has drawn, for a test to read. */
  let structuresDrawn = $state(0);
  /** Draw the weirs, the locks and the pools: the blocks as map shapes, the labels as HTML markers (the map
   * has no glyph server, so it has no text of its own). */
  function drawStructures() {
    if (!map || !ready) return;
    (map.getSource("structures") as maplibregl.GeoJSONSource).setData(fc(partFeatures(structures)));
    (map.getSource("pool-reaches") as maplibregl.GeoJSONSource).setData(fc(poolFeatures(structures)));
    weirMarkers.forEach((m) => m.remove());
    poolMarkers.forEach((m) => m.remove());
    weirMarkers = [];
    poolMarkers = [];
    for (const w of structures?.weirs ?? []) {
      // A square with the weir's number: a clear mark at any zoom. Its words show from WEIR_TEXT_ZOOM in.
      const el = document.createElement("button");
      el.type = "button";
      el.className = "weir-marker";
      el.dataset.testid = "weir-marker";
      el.dataset.weir = String(w.number);
      el.title = `${weirLabel(w)} (${PLACEHOLDER_NOTE})`;
      const mark = document.createElement("span");
      mark.className = "weir-mark";
      mark.textContent = String(w.number);
      const words = document.createElement("span");
      words.className = "weir-text";
      words.textContent = weirLabel(w);
      el.append(mark, words);
      // The click opens the card and goes no further: it does not move the section station.
      el.addEventListener("click", (e) => {
        e.stopPropagation();
        selectedWeir = w.number;
      });
      weirMarkers.push(new maplibregl.Marker({ element: el, anchor: "left", offset: [-10, 0] }).setLngLat(w.position).addTo(map));
    }
    for (const p of structures?.pools ?? []) {
      const el = document.createElement("span");
      el.className = "pool-label";
      el.dataset.testid = "pool-label";
      el.textContent = poolLabel(p);
      poolMarkers.push(new maplibregl.Marker({ element: el, offset: [0, 16] }).setLngLat(p.mid).addTo(map));
    }
    structuresDrawn = weirMarkers.length;
    if (!structures?.weirs.some((w) => w.number === selectedWeir)) selectedWeir = null;
    untrack(applyLayers);
  }
  /** The heights of the 3D blocks the map has now. */
  let blocksKey = "";
  /** Stand the blocks of the weirs and locks up in the 3D view, each top at its level. The map measures a
   * block's height from the terrain at its middle (see `extrusionHeight`), so the heights are worked out
   * from the terrain the map has drawn, and again whenever it settles (a tile arrived, the exaggeration
   * changed, the pool water went on or off). */
  function raiseBlocks() {
    if (!map || !ready) return;
    const source = map.getSource("structures-3d") as maplibregl.GeoJSONSource | undefined;
    if (!source) return;
    const m = map;
    const on = terrainOn && ticks.structures && !!m.getTerrain();
    const blocks = on ? extrusionFeatures(structures, prefs.exaggeration, (at: LonLat) => m.queryTerrainElevation(at) ?? 0) : [];
    const key = `${structuresText}|${blocks.map((b) => b.properties?.height).join(",")}`;
    if (key === blocksKey) return;
    blocksKey = key;
    source.setData(fc(blocks));
  }

  /** The contour interval the user chose: remembered on this machine, never in the design. */
  let contourChoice = $state<ContourChoice>(loadContourChoice());
  function setContourChoice(next: ContourChoice) {
    contourChoice = next;
    saveContourChoice(next);
  }
  /** The contours the map has now, as the DEM's number and the interval choice; "" for none. */
  let drawnContours = "";
  /** What the map itself has done, for a test to read: "on" once the contour layers are there. */
  let contoursDrawn = $state("off");
  /** Put the contours of the DEM on the map, or take them off. They are made only while the tick is on:
   * with the tick off the map has no contour source, and asks the engine for nothing. */
  function applyContours() {
    if (!map || !ready) return;
    const state = shownState === "after" && terrain?.cut != null ? "after" : "before";
    const key = terrain && ticks.contours ? `${terrain.id}/${contourChoice}/${state}/${terrain.cut ?? ""}` : "";
    if (key === drawnContours) return;
    for (const id of ["contours-labels", "contours", "contours-casing"]) if (map.getLayer(id)) map.removeLayer(id);
    if (map.getSource("contours")) map.removeSource("contours");
    if (terrain && key) {
      // The lines are made only inside the DEM's box. Tiles go deeper than the DEM's own cells when it
      // is coarse, so a 1 m interval can still be drawn when zoomed in.
      map.addSource("contours", { type: "vector", tiles: [contourTemplate(terrain.id, contourChoice, state, terrain.cut ?? 0)], minzoom: CONTOUR_MIN_ZOOM, maxzoom: Math.max(terrain.max_zoom, CONTOUR_DEEP_ZOOM), bounds: terrain.bounds });
      // A thin pale line on a soft dark casing: it reads on bright imagery, on dark imagery, on the
      // shaded DEM and on the bare page in either theme. Over the imagery and the shading, under the design.
      const index: maplibregl.ExpressionSpecification = ["==", ["get", "index"], true];
      const line = { type: "line", source: "contours", "source-layer": "contours", layout: { "line-join": "round", "line-cap": "round" } } as const;
      map.addLayer({ ...line, id: "contours-casing", paint: { "line-color": token("plate-ink"), "line-opacity": 0.5, "line-width": ["case", index, 3.2, 2] } }, "polygon");
      map.addLayer({ ...line, id: "contours", paint: { "line-color": token("cream"), "line-opacity": ["case", index, 1, 0.85], "line-width": ["case", index, 1.6, 0.7] } }, "polygon");
      // The level along each index contour is a small picture the page draws (see `levelPicture`): the map
      // has no glyph server, so it has no text of its own, and this works offline too.
      map.addLayer({ id: "contours-labels", type: "symbol", source: "contours", "source-layer": "contours", filter: index, layout: { "symbol-placement": "line", "symbol-spacing": 320, "icon-image": ["concat", LABEL_PREFIX, ["to-string", ["get", "level"]]], "icon-rotation-alignment": "map", "icon-keep-upright": true, "icon-padding": 6 } }, "polygon");
    }
    drawnContours = key;
    contoursDrawn = key ? "on" : "off";
  }
  /** Draw the picture of a level the map asks for: the number in metres, pale on a dark edge like the lines. */
  function levelPicture(id: string) {
    if (!map || !id.startsWith(LABEL_PREFIX) || map.hasImage(id)) return;
    const text = id.slice(LABEL_PREFIX.length);
    const scale = 2;
    const font = `600 ${11 * scale}px system-ui, -apple-system, "Segoe UI", sans-serif`;
    const canvas = document.createElement("canvas");
    const measure = canvas.getContext("2d");
    if (!measure) return;
    measure.font = font;
    canvas.width = Math.ceil(measure.measureText(text).width) + 8 * scale;
    canvas.height = 16 * scale;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    ctx.font = font;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.lineJoin = "round";
    ctx.lineWidth = 3.5 * scale;
    ctx.strokeStyle = token("plate-ink");
    ctx.fillStyle = token("cream");
    ctx.strokeText(text, canvas.width / 2, canvas.height / 2);
    ctx.fillText(text, canvas.width / 2, canvas.height / 2);
    map.addImage(id, ctx.getImageData(0, 0, canvas.width, canvas.height), { pixelRatio: scale });
  }
  /** A layer the legend may list: the design has it and it is ticked on. */
  const shows = (id: MapLayerId) => present[id] && ticks[id];
  function setTick(id: MapLayerId, on: boolean) {
    ticks = { ...ticks, [id]: on };
    saveLayerTicks(ticks);
  }
  /** Show or hide each map layer by its tick. The imagery layers are set in `shade`. */
  function applyLayers() {
    if (!map || !ready) return;
    for (const l of MAP_LAYERS) {
      if (l.id === "imagery" || l.id === "terrain-shading" || l.id === "contours") continue;
      // The cut-depth picture is put on and taken off in `applyCut`; its daylight lines are set here.
      for (const id of STYLE_LAYERS[l.id]) if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", ticks[l.id] ? "visible" : "none");
    }
    // The chainage labels and the tight-bend marks are HTML markers, not map layers.
    for (const m of labels) m.getElement().style.display = ticks["station-marks"] ? "" : "none";
    for (const m of bendMarkers) m.getElement().style.display = ticks["design-line"] ? "" : "none";
    for (const m of weirMarkers) m.getElement().style.display = ticks.structures ? "" : "none";
    for (const m of poolMarkers) m.getElement().style.display = ticks["pool-levels"] ? "" : "none";
    shade();
    applyTerrain();
  }

  /** Why a layer has nothing to draw: said beside its greyed tick. */
  function noneNote(id: MapLayerId): string {
    if ((id === "contours" || id === "pool-water" || id === "channel" || id === "cut") && terrainNote) return "web app only for now";
    if (id === "channel") return geometry?.options.length ? "needs the sizing and a water level" : "no design line yet";
    if (id === "cut") return !terrain ? "load the DEM first" : "run the volumes first";
    if (id === "structures" || id === "pool-levels" || (id === "pool-water" && !structures?.pools.length)) return "none";
    if (id === "pool-water") return "load a DEM";
    return "none loaded";
  }

  const empty = { type: "FeatureCollection", features: [] } as GeoJSON.FeatureCollection;
  const feature = (geometry: GeoJSON.Geometry, properties: GeoJSON.GeoJsonProperties = {}): GeoJSON.Feature => ({
    type: "Feature",
    geometry,
    properties,
  });

  function sources(g: MapGeometry | null, selected: string | null) {
    const bank = [...(g?.banks ?? [])].map((b) => feature(b.line as GeoJSON.Geometry, { side: b.side }));
    const centre = g?.centreline ? [feature(g.centreline as GeoJSON.Geometry)] : [];
    const polygon = g?.river_polygon ? [feature(g.river_polygon as GeoJSON.Geometry)] : [];
    const options = (g?.options ?? []).map((o, i) =>
      feature(o.line as GeoJSON.Geometry, {
        id: o.id,
        colour: optionColour(i, "map"),
        selected: o.id === selected,
        failed: !o.radius_ok,
        dim: selected != null && o.id !== selected,
      }),
    );
    const bends = (g?.options ?? []).flatMap((o) =>
      o.tight_bends.map((b) =>
        feature({ type: "Point", coordinates: b.position }, { option: o.id, radius: Math.round(b.radius_m) }),
      ),
    );
    const ticks = (g?.options ?? []).flatMap((o) =>
      o.ticks.map((t) => feature({ type: "Point", coordinates: t.position }, { option: o.id, label: t.label })),
    );
    return { bank, centre, polygon, options, bends, ticks };
  }

  /** The stretches of the option shown, each numbered in its option's list (the table's order). */
  function stretchFeatures(shown: string | null): GeoJSON.Feature[] {
    const seen = new Map<string, number>();
    const out: GeoJSON.Feature[] = [];
    for (const f of stretches?.features ?? []) {
      const option = f.properties.option_id;
      const index = seen.get(option) ?? 0;
      seen.set(option, index + 1);
      if (shown != null && option !== shown) continue;
      const picked = selectedStretch?.option === option && selectedStretch.index === index;
      out.push(feature(f.geometry as GeoJSON.Geometry, { option, index, kind: f.properties.kind, picked }));
    }
    return out;
  }

  const fc = (features: GeoJSON.Feature[]): GeoJSON.FeatureCollection => ({ type: "FeatureCollection", features });

  function draw() {
    if (!map || !ready) return;
    const s = sources(geometry, selectedId);
    (map.getSource("polygon") as maplibregl.GeoJSONSource).setData(fc(s.polygon));
    (map.getSource("banks") as maplibregl.GeoJSONSource).setData(fc(s.bank));
    (map.getSource("centre") as maplibregl.GeoJSONSource).setData(fc(s.centre));
    (map.getSource("options") as maplibregl.GeoJSONSource).setData(fc(s.options));
    (map.getSource("bends") as maplibregl.GeoJSONSource).setData(fc(s.bends));
    (map.getSource("ticks") as maplibregl.GeoJSONSource).setData(fc(s.ticks));
    const st = stretchFeatures(selectedId);
    (map.getSource("stretches") as maplibregl.GeoJSONSource).setData(fc(st));
    drawn = selectedId;
    const picked = st.find((f) => f.properties?.picked);
    drawnStretch = picked ? `${picked.properties!.option}:${picked.properties!.index}` : "";
    // Chainage labels are HTML markers: the map needs no glyph server, so it works offline too.
    labels.forEach((m) => m.remove());
    labels = [];
    const shown = geometry?.options.find((o) => o.id === selectedId) ?? geometry?.options[0];
    // A tight bend is a disc with a cross: a shape as well as a colour. Also HTML, so it needs no glyphs.
    bendMarkers.forEach((m) => m.remove());
    bendMarkers = [];
    for (const o of geometry?.options ?? []) {
      for (const b of o.tight_bends) {
        const el = document.createElement("span");
        el.className = "bend-marker";
        el.textContent = "✕";
        el.title = `Radius ${Math.round(b.radius_m)} m`;
        bendMarkers.push(new maplibregl.Marker({ element: el }).setLngLat(b.position).addTo(map));
      }
    }
    for (const t of shown?.ticks ?? []) {
      const el = document.createElement("span");
      el.className = "tick-label";
      el.textContent = t.label;
      labels.push(new maplibregl.Marker({ element: el, offset: [0, -10] }).setLngLat(t.position).addTo(map));
    }
    // The ticks are applied to the new markers; a tick alone never redraws the geometry.
    untrack(applyLayers);
  }

  /** Move the station marker, or take it off the map. */
  function placeStation() {
    if (!map || !ready) return;
    if (!stationPosition) {
      stationMarker?.remove();
      stationMarker = undefined;
      return;
    }
    if (!stationMarker) {
      const el = document.createElement("div");
      el.className = "station-marker";
      el.dataset.testid = "station-marker";
      stationMarker = new maplibregl.Marker({ element: el }).setLngLat(stationPosition).addTo(map);
    } else {
      stationMarker.setLngLat(stationPosition);
    }
    stationMarker.getElement().dataset.position = stationPosition.map((v) => v.toFixed(6)).join(",");
  }

  /** The raster source for a layer. The attribution is drawn by the page (below), never by the map. */
  function rasterSource(l: ImageryLayer): maplibregl.RasterSourceSpecification {
    return { type: "raster", tiles: [l.tiles], tileSize: l.tileSize, maxzoom: l.maxzoom };
  }
  /** The imagery layer: drawn only from its own first zoom level in, when it has one. */
  function rasterLayer(l: ImageryLayer): maplibregl.RasterLayerSpecification {
    return { id: "basemap", type: "raster", source: "basemap", ...(l.minzoom ? { minzoom: l.minzoom } : {}) };
  }
  /** The open tiles under imagery that has a first zoom level: what shows further out, and while tiles load. */
  const underSource = (tiles: string): maplibregl.RasterSourceSpecification => ({ type: "raster", tiles: [tiles], tileSize: 256, maxzoom: 14 });
  const UNDER_LAYER: maplibregl.RasterLayerSpecification = { id: "under", type: "raster", source: "under" };

  // Counters for the tiles of the imagery now shown; a change of source starts them again.
  let failed = 0;
  let loaded = 0;
  let loadingTiles = $state(false);
  let zoomNow = $state(2);
  let drawnTiles = "";
  /** Put another imagery source under everything else. */
  function swapImagery() {
    if (!map || !ready || imagery.tiles === drawnTiles) return;
    drawnTiles = imagery.tiles;
    failed = 0;
    loaded = 0;
    offline = false;
    map.removeLayer("basemap");
    map.removeSource("basemap");
    if (map.getLayer("under")) {
      map.removeLayer("under");
      map.removeSource("under");
    }
    if (imagery.under) {
      map.addSource("under", underSource(imagery.under));
      map.addLayer(UNDER_LAYER, "hillshade");
    }
    map.addSource("basemap", rasterSource(imagery));
    map.addLayer(rasterLayer(imagery), "hillshade");
    shade();
  }

  let shownShade: string | null = null;
  /** Lay the hillshade under the vector layers, and show it only while the imagery is offline. */
  function shade() {
    if (!map || !ready) return;
    const src = map.getSource("hillshade") as maplibregl.ImageSource;
    if (hillshade && hillshade.url !== shownShade) {
      shownShade = hillshade.url;
      src.updateImage({ url: hillshade.url, coordinates: hillshade.corners });
    }
    // With Imagery unticked nothing is drawn under the design: no tiles, no hillshade.
    const on = ticks.imagery;
    map.setLayoutProperty("hillshade", "visibility", on && offline && hillshade ? "visible" : "none");
    map.setLayoutProperty("basemap", "visibility", on && !offline ? "visible" : "none");
    if (map.getLayer("under")) map.setLayoutProperty("under", "visibility", on ? "visible" : "none");
  }

  /** Move the map to the picked stretch: the box around its line, with room around it. */
  let fittedStretch = "";
  function fitStretch() {
    if (!map || !ready) return;
    const key = selectedStretch ? `${selectedStretch.option}:${selectedStretch.index}` : "";
    if (key === fittedStretch) return;
    fittedStretch = key;
    const f = stretchFeatures(selectedStretch?.option ?? null).find((x) => x.properties?.picked);
    const pts = (f?.geometry as GeoJSON.LineString | undefined)?.coordinates ?? [];
    if (!pts.length) return;
    const lons = pts.map((p) => p[0]);
    const lats = pts.map((p) => p[1]);
    map.fitBounds([[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]], { padding: 80, maxZoom: 16, animate: false });
  }

  let fitted: string | null = null;
  function fit() {
    if (!map || !geometry?.bbox) return;
    const key = geometry.bbox.join(",");
    if (key === fitted) return;
    fitted = key;
    const [w, s, e, n] = geometry.bbox;
    map.fitBounds([[w, s], [e, n]], { padding: 40, animate: false });
  }

  onMount(() => {
    drawnTiles = imagery.tiles;
    try {
      map = new maplibregl.Map({
        container: host,
        attributionControl: false,
        style: {
          version: 8,
          sources: {
            ...(imagery.under ? { under: underSource(imagery.under) } : {}),
            basemap: rasterSource(imagery),
          },
          layers: [...(imagery.under ? [UNDER_LAYER] : []), rasterLayer(imagery)],
        },
        center: [0, 5],
        zoom: 2,
        maxPitch: 80,
      });
    } catch (e) {
      failure = `The map could not start: ${e instanceof Error ? e.message : e}`;
      return;
    }
    // The compass shows the bearing and the tilt; a click on it turns the map north up again.
    map.addControl(new maplibregl.NavigationControl({ showCompass: true, visualizePitch: true }), "top-right");
    map.addControl(new maplibregl.ScaleControl({ unit: "metric" }), "bottom-left");
    map.on("click", (e) => {
      // A click on a weir or a lock opens its card; a click on the pool water reads the water there.
      const blocks = ["structures-3d", "structures-weir", "structures-lock", "structures-chamber"].filter((id) => map!.getLayer(id));
      const hit = ticks.structures && blocks.length ? map!.queryRenderedFeatures(e.point, { layers: blocks }) : [];
      if (hit.length) selectedWeir = Number(hit[0].properties.weir);
      readWater(e.lngLat.lng, e.lngLat.lat);
      readCut(e.lngLat.lng, e.lngLat.lat);
      onPick?.(e.lngLat.lng, e.lngLat.lat);
    });
    map.on("idle", raiseBlocks);
    map.on("styleimagemissing", (e) => levelPicture(e.id));
    map.on("load", () => {
      const m = map!;
      // A placeholder picture until the hillshade arrives; `setCoordinates` and `updateImage` replace it.
      m.addSource("hillshade", { type: "image", url: BLANK, coordinates: [[0, 1], [1, 1], [1, 0], [0, 0]] });
      for (const id of ["polygon", "banks", "centre", "options", "bends", "ticks", "stretches", "channel-bed", "channel-lines", "cut-daylight", "pool-reaches", "structures", "structures-3d"]) m.addSource(id, { type: "geojson", data: empty });
      m.addLayer({ id: "hillshade", type: "raster", source: "hillshade", paint: { "raster-fade-duration": 0 }, layout: { visibility: "none" } });
      // Colours are read from the tokens. On imagery the same cream and plate ink serve both themes.
      const cream = token("cream");
      const ink = token("plate-ink");
      m.addLayer({ id: "polygon", type: "fill", source: "polygon", paint: { "fill-color": token("river"), "fill-opacity": 0.18 } });
      // The polygon's edge: a pale line over a dark casing, so it reads on bright and on dark imagery alike.
      m.addLayer({ id: "polygon-casing", type: "line", source: "polygon", paint: { "line-color": ink, "line-width": 3, "line-opacity": 0.65 } });
      m.addLayer({ id: "polygon-outline", type: "line", source: "polygon", paint: { "line-color": token("river"), "line-width": 1.25 } });
      m.addLayer({ id: "banks", type: "line", source: "banks", paint: { "line-color": cream, "line-width": 1.5 } });
      m.addLayer({ id: "centre", type: "line", source: "centre", paint: { "line-color": cream, "line-width": 1, "line-dasharray": [3, 3] } });
      // The channel (MAP-5), under the design line: the bed as a pale band between its two edges, and the
      // edges at the water surface as thin lines. Pale on a dark casing, like the river polygon's edge.
      m.addLayer({ id: "channel-bed", type: "fill", source: "channel-bed", paint: { "fill-color": cream, "fill-opacity": 0.4 } });
      m.addLayer({ id: "channel-bed-casing", type: "line", source: "channel-lines", filter: ["==", ["get", "kind"], "bed"], paint: { "line-color": ink, "line-width": 3, "line-opacity": 0.6 } });
      m.addLayer({ id: "channel-bed-edges", type: "line", source: "channel-lines", filter: ["==", ["get", "kind"], "bed"], paint: { "line-color": cream, "line-width": 1.5 } });
      m.addLayer({ id: "channel-water", type: "line", source: "channel-lines", filter: ["==", ["get", "kind"], "water"], paint: { "line-color": WATER_EDGE, "line-width": 1 } });
      // The daylight lines of the cut: where the cut slopes meet the ground. Dashed, so they are not read as a bank.
      m.addLayer({ id: "cut-daylight-casing", type: "line", source: "cut-daylight", paint: { "line-color": ink, "line-width": 3, "line-opacity": 0.6 } });
      m.addLayer({ id: "cut-daylight", type: "line", source: "cut-daylight", paint: { "line-color": DAYLIGHT, "line-width": 1.5, "line-dasharray": [3, 2] } });
      // Each pool's reach: a wide pale band under the design line, of two blues in turn, so that the place
      // where one pool ends and the next begins shows.
      m.addLayer({ id: "pool-reaches", type: "line", source: "pool-reaches", layout: { "line-cap": "butt" }, paint: { "line-color": ["case", ["get", "odd"], token("river"), cream], "line-width": 14, "line-opacity": 0.45 } });
      // The deep line over a cream casing keeps each option readable on any imagery. The design line is the heaviest line.
      m.addLayer({ id: "options-casing", type: "line", source: "options", paint: { "line-color": token("opt-casing"), "line-width": ["case", ["get", "selected"], 8, 6] } });
      m.addLayer({ id: "options", type: "line", source: "options", paint: { "line-color": ["get", "colour"], "line-width": 4, "line-opacity": ["case", ["get", "dim"], 0.55, 1] } });
      // Constraint stretches: a dashed red band under the ticks, heavier and solid-cased when picked in the table.
      const fail = token("fail");
      m.addLayer({ id: "stretches-casing", type: "line", source: "stretches", filter: ["==", ["get", "picked"], true], paint: { "line-color": cream, "line-width": 12 } });
      m.addLayer({ id: "stretches", type: "line", source: "stretches", paint: { "line-color": fail, "line-width": ["case", ["get", "picked"], 7, 4], "line-dasharray": [2, 1] } });
      m.addLayer({ id: "ticks", type: "circle", source: "ticks", paint: { "circle-radius": 3, "circle-color": cream, "circle-stroke-color": ink, "circle-stroke-width": 1 } });
      // The weirs and locks (MAP-4), a placeholder model, not a design: the weir a pale bar across the river,
      // the lock two pale walls and two dark gates on the design line with water between. Flat on the 2D
      // map; in the 3D view the same shapes stand up as blocks (see `raiseBlocks`).
      const kind = (...kinds: string[]): maplibregl.FilterSpecification => ["in", ["get", "kind"], ["literal", kinds]];
      const river = token("river");
      m.addLayer({ id: "structures-chamber", type: "fill", source: "structures", filter: kind("lock_chamber"), paint: { "fill-color": river } });
      m.addLayer({ id: "structures-lock", type: "fill", source: "structures", filter: kind("lock_wall", "lock_gate"), paint: { "fill-color": ["match", ["get", "kind"], "lock_gate", ink, cream] } });
      m.addLayer({ id: "structures-weir", type: "fill", source: "structures", filter: kind("weir"), paint: { "fill-color": cream } });
      m.addLayer({ id: "structures-outline", type: "line", source: "structures", paint: { "line-color": ink, "line-width": 1 } });
      m.addLayer({ id: "structures-3d", type: "fill-extrusion", source: "structures-3d", layout: { visibility: "none" }, paint: { "fill-extrusion-color": ["match", ["get", "kind"], "lock_gate", ink, "lock_chamber", river, cream], "fill-extrusion-height": ["get", "height"], "fill-extrusion-base": 0, "fill-extrusion-opacity": 1 } });
      ready = true;
      // A test reads the map's own state (terrain, pitch) through this; only a build in test mode has it.
      if (import.meta.env.MODE === "test") (window as unknown as { __owdMap: maplibregl.Map }).__owdMap = m;
      onReady?.(m);
    });
    // Offline: tiles keep failing and none has loaded. One success puts the imagery back.
    // Imagery of the user's own choosing that fails is handed back to the page, which falls back to the open one.
    map.on("error", (e) => {
      if ((e as { sourceId?: string }).sourceId !== "basemap") return;
      failed += 1;
      const status = (e as { error?: { status?: number } }).error?.status;
      if (imagery.source !== "open") {
        if (loaded === 0 && (failed >= 2 || status === 401 || status === 403)) onImageryFail?.(imagery.source);
        return;
      }
      if (failed >= 2 && loaded === 0) offline = true;
    });
    map.on("sourcedata", (e) => {
      if (e.sourceId !== "basemap" || !("tile" in e) || !e.tile) return;
      loaded += 1;
      offline = false;
    });
    // Say so while imagery tiles are on their way, and where the sharper imagery starts.
    map.on("dataloading", (e) => {
      const id = (e as { sourceId?: string }).sourceId;
      if (id === "basemap" || id === "under") loadingTiles = true;
    });
    map.on("idle", () => (loadingTiles = false));
    map.on("zoom", () => (zoomNow = map?.getZoom() ?? 0));
  });

  onDestroy(() => map?.remove());

  $effect(() => {
    void geometry;
    void selectedId;
    void stretches;
    void selectedStretch;
    void ready;
    draw();
    fit();
    fitStretch();
  });

  $effect(() => {
    void stationPosition;
    void ready;
    placeStation();
  });

  $effect(() => {
    void imagery.tiles;
    void ready;
    swapImagery();
  });

  $effect(() => {
    void hillshade;
    void offline;
    void ready;
    shade();
  });

  $effect(() => {
    void ticks;
    void ready;
    applyLayers();
  });

  $effect(() => {
    void terrain;
    void prefs;
    void shownState;
    void ready;
    untrack(applyTerrain);
  });

  $effect(() => {
    void channel;
    void ready;
    untrack(drawChannel);
  });

  $effect(() => {
    void structures;
    void ready;
    untrack(drawStructures);
  });

  $effect(() => {
    void poolWater;
    void shownState;
    void terrain?.cut;
    void ready;
    untrack(applyPoolWater);
  });

  $effect(() => {
    void contourChoice;
    void ready;
    untrack(applyContours);
  });

  /** The contour interval the map draws at this zoom level, in metres; null further out than any contour. */
  /** From this zoom level in, a weir's mark has its words beside it and each pool its label. */
  const LABEL_ZOOM = 9;
  /** The deepest depth the pool water's colours tell apart, in metres. */
  const deepest = $derived(poolWater?.depth_stops[poolWater.depth_stops.length - 1]?.[0] ?? 0);
  const m1 = (v: number) => v.toFixed(1);
  const contourNow = $derived(zoomNow < CONTOUR_MIN_ZOOM ? null : intervalAt(contourChoice, Math.floor(zoomNow)));
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && (layersOpen = false)} />

<div class="map" class:far={zoomNow < LABEL_ZOOM} data-testid="plan-map" data-structures={structuresDrawn} data-pool-water={poolWaterDrawn} data-water-terrain={waterTerrain} data-layers-off={MAP_LAYERS.filter((l) => !ticks[l.id] && !STARTS_OFF.includes(l.id)).map((l) => l.id).join(" ")} data-terrain={terrainDrawn} data-terrain-state={stateDrawn} data-channel={channelDrawn} data-cut={cutDrawn} data-terrain-shading={ticks["terrain-shading"] && terrain ? "shown" : "hidden"} data-contours={contoursDrawn} data-contour-interval={shows("contours") ? (contourNow ?? "") : ""} data-hillshade={ticks.imagery && offline && hillshade ? "shown" : "hidden"} data-selected={drawn ?? ""} data-stretch={drawnStretch}>
  <div class="canvas" bind:this={host}></div>
  {#if failure}
    <p class="failure" role="alert" data-testid="map-failure">{failure}</p>
  {/if}
  <ul class="legend" data-testid="map-legend">
    {#if shows("design-line")}
      {#each geometry?.options ?? [] as o, i}
        <li data-testid="legend-option">
          <OptionSwatch index={i} />
          {o.name}{#if !o.radius_ok}<span class="fail">radius fails</span>{/if}
        </li>
      {/each}
      {#if geometry?.options.some((o) => o.tight_bends.length)}
        <li><span class="bend-key">✕</span>tight bend</li>
      {/if}
    {/if}
    {#if shows("channel")}
      <li data-testid="legend-channel"><span class="key bed" aria-hidden="true"></span>channel bed</li>
      <li data-testid="legend-channel-water"><span class="key" aria-hidden="true"><span class="stroke thin" style:border-color={WATER_EDGE}></span></span>channel edge at the water surface</li>
    {/if}
    {#if shows("cut")}
      <li data-testid="legend-cut-daylight"><span class="key" aria-hidden="true"><span class="stroke dashed" style:border-color={DAYLIGHT}></span></span>daylight line: the cut meets the ground</li>
      <li class="ramp" data-testid="legend-cut">
        <span>depth of cut (click the cut to read a point)</span>
        {#each [["dredge", "dredge, in the river"], ["dry", "dry cut"]] as const as [kind, label]}
          <span class="ramp-row" data-testid="legend-cut-{kind}">
            {#each channel?.legend ?? [] as c}<span class="swatch" style:background={c[kind]} title="{label}: {classLabel(c)}"></span>{/each}
            <span>{label}</span>
          </span>
        {/each}
        <span class="ramp-ends">{#each channel?.legend ?? [] as c}<span>{c.from_m}</span>{/each}<span>m</span></span>
      </li>
    {/if}
    {#if ticks["constraint-stretches"] && stretches?.features.some((f) => f.properties.option_id === selectedId)}
      <li data-testid="legend-stretch"><span class="stretch-key" aria-hidden="true"></span>constraint stretch</li>
    {/if}
    {#if shows("station-marks")}
      <li data-testid="legend-station-marks"><span class="key" aria-hidden="true"><span class="dot"></span></span>station mark</li>
    {/if}
    {#if shows("river-polygon")}
      <li data-testid="legend-river-polygon"><span class="key polygon" aria-hidden="true"></span>river polygon</li>
    {/if}
    {#if shows("bank-lines")}
      <li data-testid="legend-bank-lines"><span class="key" aria-hidden="true"><span class="stroke"></span></span>bank line</li>
    {/if}
    {#if shows("centreline")}
      <li data-testid="legend-centreline"><span class="key" aria-hidden="true"><span class="stroke dashed"></span></span>centreline</li>
    {/if}
    {#if shows("structures")}
      <li data-testid="legend-structures"><span class="key weir-key" aria-hidden="true"></span>weir and lock: {PLACEHOLDER_NOTE}</li>
    {/if}
    {#if shows("pool-levels")}
      <li data-testid="legend-pool-levels"><span class="key" aria-hidden="true"><span class="stroke pool"></span></span>pool, with its water level in metres</li>
    {/if}
    {#if shows("pool-water")}
      <li data-testid="legend-pool-water">
        {#if poolWater}
          <span class="depth-ramp" aria-hidden="true" style="background: linear-gradient(to right, {poolWater.depth_stops.map(([d, c]) => `${stopColour(c)} ${deepest ? (d / deepest) * 100 : 0}%`).join(", ")})"></span>pool water by depth: 0 m to {deepest} m and deeper
        {:else}
          <span class="key" aria-hidden="true"></span>{poolWaterNote ?? "pool water"}
        {/if}
      </li>
    {/if}
    {#if shows("contours")}
      <li data-testid="legend-contours"><span class="key" aria-hidden="true"><span class="stroke thin"></span></span>{contourNow ? `contour, every ${contourNow} m` : "contours: zoom in to see them"}</li>
      {#if contourNow}
        <li data-testid="legend-contours-index"><span class="key" aria-hidden="true"><span class="stroke"></span></span>index contour, every {contourNow * 5} m, level in metres</li>
      {/if}
    {/if}
  </ul>
  <div class="layers">
    <div class="map-buttons">
      {#if terrain && (terrainOn || shows("terrain-shading") || shows("contours"))}
        <div class="terrain-state" role="group" aria-label="Ground shown" data-testid="terrain-state" data-state={shownState}>
          {#each TERRAIN_STATES as st}
            <button class="state-button" class:on={shownState === st.id} data-testid="terrain-state-{st.id}" aria-pressed={shownState === st.id} disabled={st.id === "after" && !hasCut} title={st.id === "after" && !hasCut ? "After: run the volumes first, with the DEM loaded" : st.title} onclick={() => setTerrainState(st.id)}>{st.label}</button>
          {/each}
          <button
            class="state-button compare"
            data-testid="terrain-compare"
            disabled={!hasCut}
            aria-pressed={peek}
            title={hasCut ? "Compare: hold to see the other ground, let go to come back" : "Compare: run the volumes first, with the DEM loaded"}
            onpointerdown={() => (peek = true)}
            onpointerup={() => (peek = false)}
            onpointerleave={() => (peek = false)}
            onpointercancel={() => (peek = false)}
            onkeydown={(e) => (e.key === " " || e.key === "Enter") && (e.preventDefault(), (peek = true))}
            onkeyup={(e) => (e.key === " " || e.key === "Enter") && (peek = false)}
            onblur={() => (peek = false)}>Compare</button>
        </div>
      {/if}
      {#if terrainOn}
        <label class="exaggeration" title="Vertical exaggeration: how many times taller the ground is drawn">
          <span>Height</span>
          <select data-testid="terrain-exaggeration" aria-label="Vertical exaggeration" value={prefs.exaggeration} onchange={(e) => setPrefs({ ...prefs, exaggeration: Number(e.currentTarget.value) })}>
            {#each EXAGGERATIONS as x}<option value={x}>{x}×</option>{/each}
          </select>
        </label>
      {/if}
      <button class="layers-button three-d" class:on={terrainOn} data-testid="terrain-3d" aria-pressed={terrainOn} disabled={!terrain} title={terrain ? (terrainOn ? "3D: back to the flat map" : "3D: see the DEM as a surface. Right-drag or Ctrl-drag to turn and tilt") : (terrainNote ?? "3D: load a DEM first")} onclick={() => setPrefs({ ...prefs, on: !terrainOn })}>3D</button>
      <button class="layers-button" data-testid="layers-button" aria-label="Layers" title="Layers: show or hide what the map draws" aria-expanded={layersOpen} aria-controls="map-layers-list" onclick={() => (layersOpen = !layersOpen)}>
        <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M10 3 2.5 7 10 11l7.5-4L10 3Z" /><path d="M2.5 10.5 10 14.5l7.5-4" /><path d="M2.5 14 10 18l7.5-4" /></svg>
      </button>
    </div>
    {#if layersOpen}
      <fieldset class="layers-list" id="map-layers-list" data-testid="layers-list">
        <legend>Layers</legend>
        {#each MAP_LAYERS as l}
          <div class="layer-row">
            <label class:none={!present[l.id]} data-testid="layer-row-{l.id}">
              <input type="checkbox" data-testid="layer-{l.id}" checked={ticks[l.id]} disabled={!present[l.id]} onchange={(e) => setTick(l.id, e.currentTarget.checked)} />
              <span>{l.label}</span>
              {#if !present[l.id]}<span class="none-note">{noneNote(l.id)}</span>{/if}
            </label>
            {#if l.id === "pool-water" && shows("pool-water") && poolWater}
              <button class="land-button" data-testid="pool-land-button" aria-expanded={landOpen} title="The land each pool floods, in hectares" onclick={() => (landOpen = !landOpen)}>Land</button>
            {/if}
            {#if l.id === "contours" && shows("contours")}
              <select data-testid="contour-interval" aria-label="Contour interval" title="Contour interval. Auto picks it from the zoom level; zoomed far out, a chosen interval is drawn coarser so the map does not fill with lines" value={String(contourChoice)} onchange={(e) => setContourChoice(e.currentTarget.value === "auto" ? "auto" : (Number(e.currentTarget.value) as ContourChoice))}>
                <option value="auto">Auto</option>
                {#each INTERVALS as i}<option value={String(i)}>{i} m</option>{/each}
              </select>
            {/if}
          </div>
        {/each}
      </fieldset>
    {/if}
    {#if landOpen && shows("pool-water") && poolWater}
      <section class="land" data-testid="pool-land-table" aria-label="Land flooded by each pool">
        <header>
          <h3>Land flooded by each pool</h3>
          <button class="close" aria-label="Close" onclick={() => (landOpen = false)}>×</button>
        </header>
        <table>
          <thead>
            <tr><th>Pool</th><th>Level, m</th><th>Flooded, ha</th><th>Outside the river polygon, ha</th></tr>
          </thead>
          <tbody>
            {#each poolWater.pools as p}
              <tr data-testid="pool-land-row"><td>{p.number}</td><td>{m1(p.level_m)}</td><td>{m1(p.flooded_ha)}</td><td>{p.outside_river_ha == null ? "no polygon" : m1(p.outside_river_ha)}</td></tr>
            {/each}
          </tbody>
        </table>
        <p>Ground below each pool's level that is joined to the river, from the DEM. {poolWater.mask.factor > 1 ? `Whether ground is joined to the river is worked out on cells of ${Math.round(poolWater.mask.cell_m)} m.` : ""} A study figure, not a survey.</p>
      </section>
    {/if}
  </div>
  {#if cutRead && shows("cut")}
    <section class="cut-readout" data-testid="cut-readout" aria-label="The cut at the point clicked">
      <header>
        <strong>Cut at this point</strong>
        <button data-testid="cut-readout-close" aria-label="Close the cut read-out" onclick={() => (cutRead = null)}>×</button>
      </header>
      <dl>
        {#each readout(cutRead) as [name, value]}<dt>{name}</dt><dd>{value}</dd>{/each}
      </dl>
    </section>
  {/if}
  {#if weirShown && structures}
    <StructureCard weir={weirShown} {structures} {poolWater} onClose={() => (selectedWeir = null)} />
  {/if}
  {#if shows("pool-water") && waterRead}
    <p class="water-read" role="status" data-testid="pool-water-readout">{waterReadout(waterRead)}<button class="close" aria-label="Close" onclick={() => (waterRead = null)}>×</button></p>
  {:else if shows("pool-water") && poolWaterNote}
    <p class="water-read" role="status" data-testid="pool-water-note">{poolWaterNote}</p>
  {/if}
  {#if offline}
    <p class="offline" role="status" data-testid="offline-note">
      Imagery offline{hillshade ? " — showing the DEM hillshade" : " — no DEM to show a hillshade from"}
    </p>
  {/if}
  {#if !offline && imagery.minzoom && zoomNow < imagery.minzoom}
    <p class="imagery-state" role="status" data-testid="imagery-zoom-note">Zoom in to see the sharper imagery</p>
  {:else if !offline && loadingTiles}
    <p class="imagery-state" role="status" data-testid="imagery-loading">Loading imagery…</p>
  {/if}
  <p class="attribution" data-testid="attribution" data-source={imagery.source}>{imagery.attribution}</p>
</div>

<style>
  .map { position: relative; width: 100%; height: 100%; min-height: 160px; background: var(--paper); }
  .canvas { position: absolute; inset: 0; }
  .attribution { position: absolute; right: 0; bottom: 0; margin: 0; padding: 1px 6px; font-size: 10px; background: var(--surface); color: var(--ink-3); z-index: 2; max-width: 100%; }
  .legend { position: absolute; left: var(--sp-3); top: var(--sp-3); margin: 0; padding: var(--sp-2) var(--sp-3); list-style: none; font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); z-index: 2; }
  .legend li { display: flex; gap: var(--sp-2); align-items: center; margin: 3px 0; }
  .legend:not(:has(li)) { display: none; }
  .stretch-key { display: inline-block; width: 22px; height: 0; border-top: 4px dashed var(--fail); }
  /* The keys of lines drawn pale on the imagery: shown on a small dark chip, as they look on the map. */
  .key { display: inline-grid; place-items: center; width: 22px; height: 12px; background: var(--hillshade); box-sizing: border-box; }
  .key.polygon { background: color-mix(in srgb, var(--river) 35%, var(--hillshade)); border: 1.5px solid var(--river); outline: 1px solid var(--plate-ink); }
  .key .stroke { width: 18px; height: 0; border-top: 2px solid var(--cream); }
  .key .stroke.dashed { border-top-style: dashed; }
  .key .stroke.thin { border-top-width: 1px; }
  .key.bed { background: color-mix(in srgb, var(--cream) 55%, var(--hillshade)); border-top: 1.5px solid var(--cream); border-bottom: 1.5px solid var(--cream); outline: 1px solid var(--plate-ink); }
  .legend li.ramp { flex-direction: column; align-items: flex-start; gap: 2px; }
  .ramp-row { display: flex; align-items: center; gap: 0; }
  .ramp-row .swatch { width: 18px; height: 10px; }
  .ramp-row > span:last-child { margin-left: var(--sp-2); }
  .ramp-ends { display: flex; font-size: var(--fs-11); color: var(--ink-3); }
  .ramp-ends span { width: 18px; }
  .terrain-state { display: flex; align-items: stretch; background: var(--surface); border: var(--lw-thin) solid var(--ink); border-radius: var(--radius); }
  .state-button { padding: 0 var(--sp-2); font: 600 var(--fs-12) / 1 var(--font-ui); background: none; color: var(--ink); border: 0; border-right: var(--lw-thin) solid var(--rule); }
  .state-button:last-child { border-right: 0; }
  .state-button.on, .state-button.compare[aria-pressed="true"] { background: var(--ink); color: var(--surface); }
  .state-button:disabled { color: var(--ink-3); cursor: not-allowed; }
  .state-button.compare { font-weight: 400; touch-action: none; }
  /* At the top of the map, in the middle: clear of the legend, the buttons and the floating panels. */
  .cut-readout { position: absolute; left: 50%; top: var(--sp-3); transform: translateX(-50%); z-index: 3; min-width: 250px; padding: var(--sp-2) var(--sp-3); font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); }
  .cut-readout header { display: flex; justify-content: space-between; align-items: center; gap: var(--sp-3); }
  .cut-readout header button { padding: 0 var(--sp-1); background: none; border: 0; color: var(--ink); font-size: var(--fs-12); cursor: pointer; }
  .cut-readout dl { margin: var(--sp-1) 0 0; display: grid; grid-template-columns: auto auto; gap: 2px var(--sp-3); }
  .cut-readout dt { color: var(--ink-3); }
  .cut-readout dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
  .key .stroke.pool { border-top: 6px solid color-mix(in srgb, var(--river) 60%, var(--cream)); }
  .key.weir-key { background: var(--cream); border: 1px solid var(--plate-ink); }
  .depth-ramp { display: inline-block; width: 44px; height: 12px; border: 1px solid var(--plate-ink); box-sizing: border-box; background-color: var(--hillshade); }
  .land-button { padding: 0 var(--sp-2); font-size: var(--fs-12); background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); }
  .land { padding: var(--sp-2) var(--sp-3); font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); max-width: 330px; min-height: 0; overflow-y: auto; }
  .land header { display: flex; justify-content: space-between; align-items: center; gap: var(--sp-2); }
  .land h3 { margin: 0; font-size: var(--fs-12); font-weight: 600; }
  .land table { border-collapse: collapse; margin: var(--sp-1) 0; font-variant-numeric: tabular-nums; }
  .land th, .land td { padding: 1px var(--sp-2) 1px 0; text-align: right; vertical-align: bottom; }
  .land th { font-weight: 600; color: var(--ink-2); }
  .land p { margin: 0; color: var(--ink-2); }
  .close { width: 22px; height: 22px; padding: 0; line-height: 1; background: none; border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); color: var(--ink); }
  .water-read { position: absolute; left: 50%; bottom: 28px; transform: translateX(-50%); margin: 0; padding: 3px var(--sp-2); display: flex; gap: var(--sp-2); align-items: center; font-size: var(--fs-12); font-variant-numeric: tabular-nums; background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); z-index: 3; max-width: 90%; }
  :global(.weir-marker) { display: flex; gap: 4px; align-items: center; padding: 0; background: none; border: 0; cursor: pointer; font: 600 var(--fs-11) / 1 var(--font-ui); color: var(--cream); text-shadow: 0 0 3px var(--plate-ink), 0 0 3px var(--plate-ink); white-space: nowrap; }
  :global(.weir-mark) { display: grid; place-items: center; min-width: 20px; height: 20px; padding: 0 2px; box-sizing: border-box; background: var(--plate-ink); color: var(--cream); border: 2px solid var(--cream); box-shadow: 0 0 0 1px var(--plate-ink); text-shadow: none; }
  :global(.pool-label) { padding: 1px 4px; font: 600 var(--fs-11) / 1.2 var(--font-ui); color: var(--plate-ink); background: color-mix(in srgb, var(--cream) 85%, var(--river)); border: 1px solid var(--plate-ink); white-space: nowrap; pointer-events: none; }
  .far :global(.weir-text), .far :global(.pool-label) { display: none; }
  .key .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--cream); border: 1px solid var(--plate-ink); box-sizing: border-box; }
  /* Under the map's zoom buttons and compass, the same size as they are. */
  .layers { position: absolute; top: 107px; right: 10px; z-index: 4; max-height: calc(100% - 107px - 24px); display: flex; flex-direction: column; align-items: flex-end; gap: var(--sp-1); }
  .layers-button { display: grid; place-items: center; width: 31px; height: 31px; padding: 0; background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); border-radius: var(--radius); }
  .map-buttons { display: flex; gap: var(--sp-1); align-items: stretch; }
  .three-d { font: 600 var(--fs-12) / 1 var(--font-ui); }
  .three-d.on { background: var(--ink); color: var(--surface); }
  .three-d:disabled { color: var(--ink-3); border-color: var(--rule-strong); cursor: not-allowed; }
  .exaggeration { display: flex; gap: var(--sp-1); align-items: center; padding: 0 var(--sp-2); font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); border-radius: var(--radius); }
  .exaggeration select { font-size: var(--fs-12); }
  .layers-list { margin: 0; padding: var(--sp-2) var(--sp-3); display: flex; flex-direction: column; gap: 3px; font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); min-width: 190px; min-height: 0; overflow-y: auto; }
  .layers-list legend { padding: 0 var(--sp-1); font-weight: 600; }
  .layer-row { display: flex; gap: var(--sp-2); align-items: center; }
  .layer-row select { font-size: var(--fs-12); }
  .layers-list label { flex: 1; display: flex; gap: var(--sp-2); align-items: center; white-space: nowrap; }
  .layers-list label.none { color: var(--ink-3); }
  .none-note { margin-left: auto; padding-left: var(--sp-3); font-style: italic; }
  .bend-key { display: inline-grid; place-items: center; width: 14px; height: 14px; border-radius: 50%; border: var(--lw-thin) solid var(--ink); font-size: var(--fs-11); font-weight: 700; color: var(--fail); }
  .imagery-state { position: absolute; left: var(--sp-2); bottom: 28px; margin: 0; padding: 3px var(--sp-2); font-size: var(--fs-12); background: var(--surface, #fff); color: var(--ink, #222); border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); z-index: 2; }
  .offline { position: absolute; left: var(--sp-2); bottom: 28px; margin: 0; padding: 3px var(--sp-2); font-size: var(--fs-12); background: var(--warn-soft); color: var(--warn); border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); z-index: 2; }
  .fail { color: var(--fail); margin-left: var(--sp-2); font-weight: 600; }
  .fail::before { content: "✕ "; }
  .failure { position: absolute; left: 50%; top: 40%; transform: translateX(-50%); margin: 0; padding: var(--sp-2) var(--sp-3); background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); z-index: 2; max-width: 80%; text-align: center; }
  .failure { color: var(--fail); }
  :global(.station-marker) { width: 16px; height: 16px; border-radius: 50%; background: var(--plate-ink); border: 3px solid var(--cream); box-shadow: 0 0 0 1px var(--plate-ink); pointer-events: none; }
  :global(.tick-label) { font-size: var(--fs-11); color: var(--cream); text-shadow: 0 0 3px var(--plate-ink), 0 0 3px var(--plate-ink); white-space: nowrap; }
  :global(.bend-marker) { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: var(--surface); border: 1.5px solid var(--ink); color: var(--fail); font: 700 14px / 1 var(--font-ui); pointer-events: none; }
  :global(.maplibregl-ctrl-group) { border-radius: var(--radius); background: var(--surface); border: var(--lw-thin) solid var(--ink); box-shadow: none; }
  :global(.maplibregl-ctrl-group button) { border-color: var(--rule); }
  /* The control icons are dark drawings: show them in ink on the dark theme. */
  :global(:root[data-theme="dark"] .maplibregl-ctrl-icon) { filter: invert(1); }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"]) .maplibregl-ctrl-icon) { filter: invert(1); }
  }
  :global(.maplibregl-ctrl-scale) { background: var(--surface); color: var(--ink); border-color: var(--ink); }
</style>
