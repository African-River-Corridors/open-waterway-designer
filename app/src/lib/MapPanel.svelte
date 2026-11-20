<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import * as maplibregl from "maplibre-gl";
  import "maplibre-gl/dist/maplibre-gl.css";
  import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?url";
  import type * as GeoJSON from "geojson";
  import { terrainTile, type Hillshade, type MapGeometry, type StretchCollection, type TerrainInfo } from "./api";
  import { DEFAULT_IMAGERY, layerFor, type ImageryLayer, type ImagerySource } from "./imagery";
  import OptionSwatch from "./OptionSwatch.svelte";
  import { optionColour, token } from "./theme";
  import { MAP_LAYERS, STARTS_OFF, STYLE_LAYERS, loadLayerTicks, saveLayerTicks, type LayerTicks, type MapLayerId } from "./mapLayers";
  import { EXAGGERATIONS, PITCH_3D, TERRAIN_PROTOCOL, loadTerrainPrefs, parseTileUrl, saveTerrainPrefs, tileTemplate, type TerrainPrefs } from "./terrain3d";

  let {
    geometry,
    selectedId = null,
    stretches = null,
    selectedStretch = null,
    hillshade = null,
    terrain = null,
    terrainNote = null,
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
  maplibregl.addProtocol(TERRAIN_PROTOCOL, async (params, abort) => {
    const tile = parseTileUrl(params.url);
    if (!tile) throw new Error(`not a terrain tile address: ${params.url}`);
    for (let tries = 0; ; tries += 1) {
      try {
        const png = await terrainTile(tile.z, tile.x, tile.y);
        return { data: png.buffer.slice(png.byteOffset, png.byteOffset + png.byteLength) as ArrayBuffer };
      } catch (e) {
        if (tries >= 4 || abort.signal.aborted) throw new Error(String(e));
        await wait(1000, abort.signal);
      }
    }
  });

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
  /** The layers this design has something to draw for. Imagery is always there. */
  const present = $derived<Record<MapLayerId, boolean>>({
    "river-polygon": !!geometry?.river_polygon,
    centreline: !!geometry?.centreline,
    "bank-lines": !!geometry?.banks.length,
    "design-line": !!geometry?.options.length,
    "constraint-stretches": !!stretches?.features.length,
    "station-marks": !!geometry?.options.some((o) => o.ticks.length),
    "terrain-shading": !!terrain,
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
  /** The DEM whose tiles the map has now, by its number; "" for none. */
  let drawnTerrain = "";
  /** Whether the map is tilted for 3D now, so the camera moves only when 3D goes on or off. */
  let tilted = false;
  /** What the map itself has done, for a test to read: "on" once the terrain is set. */
  let terrainDrawn = $state("off");
  /** Give the map the DEM's tiles (or take them away), then set 3D, the exaggeration and the shading. */
  function applyTerrain() {
    if (!map || !ready) return;
    const key = terrain ? String(terrain.id) : "";
    if (key !== drawnTerrain) {
      // Another DEM, or none: everything drawn from the old tiles goes.
      if (map.getTerrain()) map.setTerrain(null);
      for (const id of ["terrain-tint", "terrain-shading"]) if (map.getLayer(id)) map.removeLayer(id);
      for (const id of ["dem", "dem-shade"]) if (map.getSource(id)) map.removeSource(id);
      if (terrain) {
        // Two sources of the same tiles: the map's own advice, so the 3D surface and the shading
        // each keep their own tiles. The engine makes each tile once and answers both.
        const source: maplibregl.RasterDEMSourceSpecification = { type: "raster-dem", tiles: [tileTemplate(terrain.id)], tileSize: terrain.tile_size, maxzoom: terrain.max_zoom, encoding: terrain.encoding };
        map.addSource("dem", source);
        // The shading is drawn only where the DEM is: outside its box there is no terrain to shade.
        map.addSource("dem-shade", { ...source, bounds: terrain.bounds });
        // Over the imagery, under the design. The tint colours the ground by its height, low ground dark
        // and high ground pale, between the DEM's lowest and highest height: flat river country has little
        // slope to shade, so without the imagery the tint is what shows the DEM.
        const [low, mid, high] = [token("plate-ink"), token("hillshade"), token("cream")];
        const span = Math.max(terrain.max_m - terrain.min_m, 0.01);
        map.addLayer({ id: "terrain-tint", type: "color-relief", source: "dem-shade", layout: { visibility: "none" }, paint: { "color-relief-color": ["interpolate", ["linear"], ["elevation"], terrain.min_m, low, terrain.min_m + span / 2, mid, terrain.max_m, high] } }, "polygon");
        map.addLayer({ id: "terrain-shading", type: "hillshade", source: "dem-shade", layout: { visibility: "none" }, paint: { "hillshade-exaggeration": 0.6, "hillshade-shadow-color": low, "hillshade-highlight-color": high } }, "polygon");
      }
      drawnTerrain = key;
    }
    if (terrainOn) {
      map.setTerrain({ source: "dem", exaggeration: prefs.exaggeration });
      if (!tilted) map.easeTo({ pitch: PITCH_3D, duration: 600 });
    } else {
      if (map.getTerrain()) map.setTerrain(null);
      // Back to the flat view, north up.
      if (tilted) map.easeTo({ pitch: 0, bearing: 0, duration: 600 });
    }
    tilted = terrainOn;
    terrainDrawn = map.getTerrain() ? "on" : "off";
    // With the imagery: the shading alone, laid over it. Without: the tint under the shading, the DEM itself.
    const shaded = ticks["terrain-shading"];
    if (map.getLayer("terrain-shading")) map.setLayoutProperty("terrain-shading", "visibility", shaded ? "visible" : "none");
    if (map.getLayer("terrain-tint")) map.setLayoutProperty("terrain-tint", "visibility", shaded && !ticks.imagery ? "visible" : "none");
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
      if (l.id === "imagery" || l.id === "terrain-shading") continue;
      for (const id of STYLE_LAYERS[l.id]) if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", ticks[l.id] ? "visible" : "none");
    }
    // The chainage labels and the tight-bend marks are HTML markers, not map layers.
    for (const m of labels) m.getElement().style.display = ticks["station-marks"] ? "" : "none";
    for (const m of bendMarkers) m.getElement().style.display = ticks["design-line"] ? "" : "none";
    shade();
    applyTerrain();
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
    map.on("click", (e) => onPick?.(e.lngLat.lng, e.lngLat.lat));
    map.on("load", () => {
      const m = map!;
      // A placeholder picture until the hillshade arrives; `setCoordinates` and `updateImage` replace it.
      m.addSource("hillshade", { type: "image", url: BLANK, coordinates: [[0, 1], [1, 1], [1, 0], [0, 0]] });
      for (const id of ["polygon", "banks", "centre", "options", "bends", "ticks", "stretches"]) m.addSource(id, { type: "geojson", data: empty });
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
      // The deep line over a cream casing keeps each option readable on any imagery. The design line is the heaviest line.
      m.addLayer({ id: "options-casing", type: "line", source: "options", paint: { "line-color": token("opt-casing"), "line-width": ["case", ["get", "selected"], 8, 6] } });
      m.addLayer({ id: "options", type: "line", source: "options", paint: { "line-color": ["get", "colour"], "line-width": 4, "line-opacity": ["case", ["get", "dim"], 0.55, 1] } });
      // Constraint stretches: a dashed red band under the ticks, heavier and solid-cased when picked in the table.
      const fail = token("fail");
      m.addLayer({ id: "stretches-casing", type: "line", source: "stretches", filter: ["==", ["get", "picked"], true], paint: { "line-color": cream, "line-width": 12 } });
      m.addLayer({ id: "stretches", type: "line", source: "stretches", paint: { "line-color": fail, "line-width": ["case", ["get", "picked"], 7, 4], "line-dasharray": [2, 1] } });
      m.addLayer({ id: "ticks", type: "circle", source: "ticks", paint: { "circle-radius": 3, "circle-color": cream, "circle-stroke-color": ink, "circle-stroke-width": 1 } });
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
    void ready;
    untrack(applyTerrain);
  });
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && (layersOpen = false)} />

<div class="map" data-testid="plan-map" data-layers-off={MAP_LAYERS.filter((l) => !ticks[l.id] && !STARTS_OFF.includes(l.id)).map((l) => l.id).join(" ")} data-terrain={terrainDrawn} data-terrain-shading={ticks["terrain-shading"] && terrain ? "shown" : "hidden"} data-hillshade={ticks.imagery && offline && hillshade ? "shown" : "hidden"} data-selected={drawn ?? ""} data-stretch={drawnStretch}>
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
  </ul>
  <div class="layers">
    <div class="map-buttons">
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
          <label class:none={!present[l.id]} data-testid="layer-row-{l.id}">
            <input type="checkbox" data-testid="layer-{l.id}" checked={ticks[l.id]} disabled={!present[l.id]} onchange={(e) => setTick(l.id, e.currentTarget.checked)} />
            <span>{l.label}</span>
            {#if !present[l.id]}<span class="none-note">none loaded</span>{/if}
          </label>
        {/each}
      </fieldset>
    {/if}
  </div>
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
  .layers-list label { display: flex; gap: var(--sp-2); align-items: center; white-space: nowrap; }
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
