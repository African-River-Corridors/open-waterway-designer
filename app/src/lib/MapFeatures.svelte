<script lang="ts">
  // Buildings, places and settlements on the map (BLD-1). The design names each file by path and hash;
  // the worker reads it and gives the map WGS84 GeoJSON once, which MapLibre tiles off the main thread
  // (its GeoJSON source cuts vector tiles in its own worker), so the map stays quick with 36,000 footprints:
  // further out than FOOTPRINT_ZOOM a building is a dot, further out than DOT_ZOOM only a density.
  //
  // The places' names are pictures drawn by this page (the map has no glyph server, as for the contour
  // labels): one picture per distinct kind and name, made when the map first asks for it.
  import { onDestroy, untrack } from "svelte";
  import type * as maplibregl from "maplibre-gl";
  import type * as GeoJSON from "geojson";
  import * as api from "./api";
  import type { BuildingConflicts, FeatureKey, FeatureMap, FeaturesFlooded, PoolWaterInfo, Snapshot } from "./api";
  import { token } from "./theme";
  import { DOT_ZOOM, FOOTPRINT_ZOOM, PLACE_BLOCK_NOTE, PLACE_KINDS, cardRows, kindsPresent, parsePlaceImageId, placeBlock, placeImageId, placeKind, placeSortKey, ringCentre, shortName, type FeatureCard, type PlaceBlock, type PlaceKind } from "./features";
  import { extrusionHeight } from "./structures";

  let {
    map,
    ready,
    features,
    ticks,
    poolOn,
    poolWater,
    minDepth = 0.3,
    flooded = $bindable(null),
    terrainOn = false,
    exaggeration = 1,
    channelKey = "",
  }: {
    map: maplibregl.Map | undefined;
    ready: boolean;
    features: Snapshot["feature_inputs"] | undefined;
    ticks: { buildings: boolean; places: boolean; settlements: boolean };
    /** The pool water is shown on the map. */
    poolOn: boolean;
    poolWater: PoolWaterInfo | null;
    /** The least depth that counts a building as flooded, metres (the optimise setting). */
    minDepth?: number;
    /** The buildings the pool water floods, by pool: for the Land table. Null with the pool water off. */
    flooded?: FeaturesFlooded | null;
    /** The map is in 3D: the places' blocks stand up. */
    terrainOn?: boolean;
    exaggeration?: number;
    /** Changes when the channel may have changed: the buildings' place against the cut is asked again. */
    channelKey?: string;
  } = $props();
  /** The buildings and the channel (BLD-1): in the cut, near it, severed. A report, not a cost. */
  let conflicts = $state<BuildingConflicts | null>(null);
  const TAKEN = "#b2182b";
  const NEAR = "#fdae6b";
  const SEVERED = "#8073ac";
  /** The places' placeholder blocks, flat (BLD-1 groundwork for 3D models). */
  let blocks: (PlaceBlock & { fid: number })[] = [];
  let blocksKey = "";
  let blocksDrawn = $state(0);

  /** Fixed colours: the layers lie on imagery in both themes, as the design's own lines do. */
  const FLOODED = "#e4572e";
  const CREAM = "#f2e8cf";
  const INK = "#2b2925";
  const KEYS: FeatureKey[] = ["buildings", "points_of_interest", "settlements"];
  const empty = { type: "FeatureCollection", features: [] } as unknown as GeoJSON.FeatureCollection;

  /** What the map has of each input, by the hash of the file it was made from. */
  let drawnSha: Partial<Record<FeatureKey, string>> = {};
  /** The kinds of place on the map, for the legend. */
  let kinds = $state<PlaceKind[]>([]);
  /** For a test: the state of each layer's data, and how long the buildings took to show. */
  let buildingsState = $state("none");
  let buildingsMs = $state<number | null>(null);
  let placesCount = $state(0);
  let placeImages = $state(0);
  let added = false;

  function addLayers(m: maplibregl.Map) {
    if (added) return;
    added = true;
    for (const id of ["buildings", "building-points", "places", "settlements", "places-blocks", "places-blocks-3d"]) m.addSource(id, { type: "geojson", data: empty, promoteId: "fid" });
    // Over the imagery and the pool water, under the channel and the design line.
    const before = ["channel-bed", "pool-reaches", "options-casing"].find((id) => m.getLayer(id));
    const vis = (on: boolean) => ({ visibility: on ? "visible" : "none" }) as const;
    const t = untrack(() => ticks);
    const state = ["boolean", ["feature-state", "flooded"], false] as unknown as maplibregl.ExpressionSpecification;
    m.addLayer({ id: "settlements-fill", type: "fill", source: "settlements", layout: vis(t.settlements), paint: { "fill-color": ["case", state, FLOODED, CREAM], "fill-opacity": 0.14 } }, before);
    m.addLayer({ id: "settlements-outline", type: "line", source: "settlements", layout: vis(t.settlements), paint: { "line-color": CREAM, "line-width": 1, "line-opacity": 0.9 } }, before);
    m.addLayer({ id: "buildings-density", type: "heatmap", source: "building-points", maxzoom: DOT_ZOOM + 1, layout: vis(t.buildings), paint: { "heatmap-radius": ["interpolate", ["linear"], ["zoom"], 8, 6, DOT_ZOOM + 1, 14], "heatmap-intensity": 0.6, "heatmap-opacity": ["interpolate", ["linear"], ["zoom"], DOT_ZOOM, 0.85, DOT_ZOOM + 1, 0], "heatmap-color": ["interpolate", ["linear"], ["heatmap-density"], 0, "rgba(0,0,0,0)", 0.2, "rgba(242,232,207,0.55)", 0.6, "rgba(253,174,107,0.8)", 1, "rgba(228,87,46,0.95)"] } }, before);
    m.addLayer({ id: "buildings-dots", type: "circle", source: "building-points", minzoom: DOT_ZOOM, layout: vis(t.buildings), paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], DOT_ZOOM, 1.8, FOOTPRINT_ZOOM, 3], "circle-color": ["case", state, FLOODED, CREAM], "circle-stroke-color": INK, "circle-stroke-width": 0.75, "circle-pitch-alignment": "map", "circle-opacity": ["interpolate", ["linear"], ["zoom"], FOOTPRINT_ZOOM, 1, FOOTPRINT_ZOOM + 0.5, ["case", ["==", ["get", "point"], true], 1, 0]], "circle-stroke-opacity": ["interpolate", ["linear"], ["zoom"], FOOTPRINT_ZOOM, 1, FOOTPRINT_ZOOM + 0.5, ["case", ["==", ["get", "point"], true], 1, 0]] } }, before);
    m.addLayer({ id: "buildings-fill", type: "fill", source: "buildings", minzoom: FOOTPRINT_ZOOM, layout: vis(t.buildings), paint: { "fill-color": ["case", state, FLOODED, CREAM], "fill-opacity": 0.8 } }, before);
    // The cut conflict style: the edge says how the building stands against the channel's cut.
    const cut = ["coalesce", ["feature-state", "cut"], ""] as unknown as maplibregl.ExpressionSpecification;
    const cutColour = ["match", cut, "taken", TAKEN, "near", NEAR, "severed", SEVERED, INK] as unknown as maplibregl.ExpressionSpecification;
    m.addLayer({ id: "buildings-outline", type: "line", source: "buildings", minzoom: FOOTPRINT_ZOOM, layout: vis(t.buildings), paint: { "line-color": cutColour, "line-width": ["match", cut, "taken", 2.6, "near", 2, "severed", 2, ["case", state, 1.6, 0.8]] } }, before);
    m.setPaintProperty("buildings-dots", "circle-stroke-color", cutColour);
    m.setPaintProperty("buildings-dots", "circle-stroke-width", ["match", cut, "", 0.75, 1.8]);
    // A bridge's deck and a quay's slab: a placeholder block, pale with a dark edge like the weirs; in 3D it stands up.
    m.addLayer({ id: "places-blocks", type: "fill", source: "places-blocks", layout: vis(t.places), paint: { "fill-color": CREAM, "fill-outline-color": INK, "fill-opacity": 0.9 } });
    m.addLayer({ id: "places-blocks-3d", type: "fill-extrusion", source: "places-blocks-3d", layout: vis(t.places), paint: { "fill-extrusion-color": CREAM, "fill-extrusion-height": ["get", "height"], "fill-extrusion-base": ["get", "base"], "fill-extrusion-opacity": 1 } });
    m.on("idle", raiseBlocks);
    // Names over everything but the cards: the larger place is placed first where two would collide.
    m.addLayer({ id: "places", type: "symbol", source: "places", layout: { ...vis(t.places), "icon-image": ["get", "img"], "icon-anchor": "left", "icon-offset": [-7, 0], "symbol-sort-key": ["get", "sort"], "icon-padding": 2, "icon-pitch-alignment": "viewport", "icon-rotation-alignment": "viewport" } });
    m.on("styleimagemissing", (e) => placePicture(m, e.id));
    m.on("click", click);
  }

  /** The picture of a place: its kind's mark and its name, cream on a dark halo. */
  function placePicture(m: maplibregl.Map, id: string) {
    const p = parsePlaceImageId(id);
    if (!p || m.hasImage(id)) return;
    const scale = 2;
    const font = `600 ${12 * scale}px "Public Sans", system-ui, -apple-system, "Segoe UI", sans-serif`;
    const canvas = document.createElement("canvas");
    const measure = canvas.getContext("2d");
    if (!measure) return;
    measure.font = font;
    const mark = 14 * scale;
    const gap = p.name ? 4 * scale : 0;
    canvas.width = Math.ceil(mark + gap + (p.name ? measure.measureText(p.name).width : 0) + 6 * scale);
    canvas.height = 20 * scale;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    const k = PLACE_KINDS.find((x) => x.kind === p.kind)!;
    const cy = canvas.height / 2;
    const cx = 2 * scale + mark / 2;
    const r = mark / 2 - scale;
    ctx.lineWidth = 2 * scale;
    ctx.strokeStyle = INK;
    ctx.fillStyle = CREAM;
    ctx.beginPath();
    if (k.shape === "circle") ctx.arc(cx, cy, r * 0.7, 0, Math.PI * 2);
    else if (k.shape === "square") ctx.rect(cx - r, cy - r, 2 * r, 2 * r);
    else {
      ctx.moveTo(cx, cy - r);
      ctx.lineTo(cx + r, cy);
      ctx.lineTo(cx, cy + r);
      ctx.lineTo(cx - r, cy);
      ctx.closePath();
    }
    ctx.fill();
    ctx.stroke();
    if (k.letter) {
      ctx.fillStyle = INK;
      ctx.font = `700 ${9 * scale}px "Public Sans", system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(k.letter, cx, cy + 0.5 * scale);
    }
    if (p.name) {
      ctx.font = font;
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.lineJoin = "round";
      ctx.lineWidth = 4 * scale;
      ctx.strokeStyle = INK;
      ctx.fillStyle = CREAM;
      const x = 2 * scale + mark + gap;
      ctx.strokeText(p.name, x, cy);
      ctx.fillText(p.name, x, cy);
    }
    m.addImage(id, ctx.getImageData(0, 0, canvas.width, canvas.height), { pixelRatio: scale });
    placeImages += 1;
  }

  /** Give the map one input's features, or take them off. */
  async function load(m: maplibregl.Map, key: FeatureKey) {
    const info = features?.[key];
    const want = info?.attached ? info.sha256 : "";
    if ((drawnSha[key] ?? "") === want) return;
    drawnSha[key] = want;
    const data: FeatureMap | null = want ? await api.featureMap(key).catch(() => null) : null;
    if (drawnSha[key] !== want) return;
    const fs = data?.features ?? [];
    if (key === "buildings") {
      const start = performance.now();
      buildingsState = fs.length ? "loading" : "none";
      const points = fs.flatMap((f) => {
        const c = ringCentre(f.geometry);
        return c ? [{ type: "Feature", geometry: { type: "Point", coordinates: c }, properties: { fid: f.properties.fid, point: f.geometry.type === "Point" } }] : [];
      });
      (m.getSource("buildings") as maplibregl.GeoJSONSource).setData((data ?? empty) as unknown as GeoJSON.FeatureCollection);
      (m.getSource("building-points") as maplibregl.GeoJSONSource).setData({ type: "FeatureCollection", features: points } as unknown as GeoJSON.FeatureCollection);
      if (fs.length) m.once("idle", () => ((buildingsMs = Math.round(performance.now() - start)), (buildingsState = "on")));
      floodedKey = "";
      untrack(markFlooded);
      conflictsKey = "";
      untrack(markConflicts);
    } else if (key === "points_of_interest") {
      const pts = fs.map((f) => {
        const kind = placeKind(f.properties.kind);
        const c = f.geometry.type === "Point" ? (f.geometry.coordinates as [number, number]) : ringCentre(f.geometry);
        const name = shortName(String(f.properties.name ?? ""));
        return { type: "Feature", geometry: { type: "Point", coordinates: c }, properties: { fid: f.properties.fid, kind, img: placeImageId(kind, name), sort: placeSortKey(f.properties.area_ha) } };
      });
      kinds = kindsPresent(fs.map((f) => f.properties.kind));
      blocks = fs.flatMap((f, i) => {
        const at = pts[i].geometry.coordinates;
        const b = at ? placeBlock(f.properties, at as [number, number]) : null;
        return b ? [{ ...b, fid: f.properties.fid }] : [];
      });
      blocksDrawn = blocks.length;
      (m.getSource("places-blocks") as maplibregl.GeoJSONSource).setData({ type: "FeatureCollection", features: blocks.map((b) => ({ type: "Feature", geometry: { type: "Polygon", coordinates: [b.ring] }, properties: { fid: b.fid, kind: b.kind, note: PLACE_BLOCK_NOTE } })) } as unknown as GeoJSON.FeatureCollection);
      blocksKey = "";
      raiseBlocks();
      placesCount = pts.length;
      (m.getSource("places") as maplibregl.GeoJSONSource).setData({ type: "FeatureCollection", features: pts } as unknown as GeoJSON.FeatureCollection);
    } else {
      (m.getSource("settlements") as maplibregl.GeoJSONSource).setData((data ?? empty) as unknown as GeoJSON.FeatureCollection);
    }
  }

  let conflictsKey = "";
  let conflictsAsk = 0;
  /** Mark each building's place against the cut, as the engine reads it from the channel's stations. */
  function markConflicts() {
    const m = map;
    if (!m || !added) return;
    const key = features?.buildings?.attached ? `${channelKey}|${drawnSha.buildings}|${features.buildings.near_cut_m}` : "";
    if (key === conflictsKey) return;
    conflictsKey = key;
    const ask = ++conflictsAsk;
    const apply = (c: BuildingConflicts | null) => {
      if (ask !== conflictsAsk) return;
      for (const s of ["buildings", "building-points"]) if (m.getSource(s)) for (const kind of ["taken", "near", "severed"] as const) for (const fid of conflicts?.fids[kind] ?? []) m.setFeatureState({ source: s, id: fid }, { cut: "" });
      conflicts = c;
      for (const s of ["buildings", "building-points"]) for (const kind of ["near", "severed", "taken"] as const) for (const fid of c?.fids[kind] ?? []) m.setFeatureState({ source: s, id: fid }, { cut: kind });
    };
    if (!key) return apply(null);
    api.buildingConflicts().then(apply, () => apply(null));
  }

  /** Stand the places' blocks up in the 3D view: the top at the deck level (or the given height above the
   * ground), measured as the map measures a block, from the terrain at its middle (see `extrusionHeight`). */
  function raiseBlocks() {
    const m = map;
    if (!m || !added) return;
    const on = terrainOn && !!m.getTerrain();
    const feats = on
      ? blocks.map((b) => {
          const mid = ringCentre({ type: "Polygon", coordinates: [b.ring] })!;
          const drawn = m.queryTerrainElevation(mid) ?? 0;
          const top = b.top_m != null ? extrusionHeight(b.top_m, drawn, exaggeration) : (b.height_m ?? 1) * exaggeration;
          const base = Math.max(top - b.thickness_m * exaggeration, 0);
          return { type: "Feature", geometry: { type: "Polygon", coordinates: [b.ring] }, properties: { fid: b.fid, height: Math.round(top * 20) / 20, base: Math.round(base * 20) / 20 } };
        })
      : [];
    const key = JSON.stringify(feats.map((f) => [f.properties.fid, f.properties.height]));
    if (key === blocksKey) return;
    blocksKey = key;
    (m.getSource("places-blocks-3d") as maplibregl.GeoJSONSource | undefined)?.setData({ type: "FeatureCollection", features: feats } as unknown as GeoJSON.FeatureCollection);
    if (m.getLayer("places-blocks")) m.setPaintProperty("places-blocks", "fill-opacity", on ? 0 : 0.9);
  }

  /** The flood mask and depth the flooded buildings were marked for; "" for none. */
  let floodedKey = "";
  let floodedAsk = 0;
  /** Colour the buildings the pool water covers, and count them by pool. */
  function markFlooded() {
    const m = map;
    if (!m || !added) return;
    const on = poolOn && !!poolWater && !!features?.buildings?.attached;
    const key = on ? `${poolWater!.id}/${minDepth}/${drawnSha.buildings}` : "";
    if (key === floodedKey) return;
    floodedKey = key;
    for (const s of ["buildings", "building-points"]) if (m.getSource(s)) m.removeFeatureState({ source: s });
    if (!on) {
      flooded = null;
      return;
    }
    const ask = ++floodedAsk;
    api.featuresFlooded("buildings", poolWater!.id, minDepth).then(
      (f) => {
        if (ask !== floodedAsk) return;
        for (const fid of f.fids) for (const s of ["buildings", "building-points"]) m.setFeatureState({ source: s, id: fid }, { flooded: true });
        flooded = f;
      },
      () => ask === floodedAsk && (flooded = null),
    );
  }

  /** The card open now. */
  let card = $state<FeatureCard | null>(null);
  let cardAsk = 0;
  function click(e: maplibregl.MapMouseEvent) {
    const m = map;
    if (!m) return;
    const order: [string, FeatureKey][] = [
      ["places", "points_of_interest"],
      ["places-blocks", "points_of_interest"],
      ["places-blocks-3d", "points_of_interest"],
      ["buildings-fill", "buildings"],
      ["buildings-dots", "buildings"],
      ["settlements-fill", "settlements"],
    ];
    for (const [layer, key] of order) {
      if (!m.getLayer(layer) || m.getLayoutProperty(layer, "visibility") === "none") continue;
      const box: [maplibregl.PointLike, maplibregl.PointLike] = [
        [e.point.x - 3, e.point.y - 3],
        [e.point.x + 3, e.point.y + 3],
      ];
      const hit = m.queryRenderedFeatures(box, { layers: [layer] });
      if (!hit.length) continue;
      const fid = Number(hit[0].properties?.fid);
      const ask = ++cardAsk;
      api.featureCard(key, fid, poolOn && poolWater ? poolWater.id : null).then(
        (c) => ask === cardAsk && (card = c),
        () => ask === cardAsk && (card = null),
      );
      return;
    }
  }

  $effect(() => {
    void ready;
    void features;
    const m = map;
    if (!m || !ready) return;
    untrack(() => {
      addLayers(m);
      for (const k of KEYS) void load(m, k);
    });
  });
  $effect(() => {
    void poolOn;
    void poolWater?.id;
    void minDepth;
    void features?.buildings?.sha256;
    untrack(markFlooded);
  });
  $effect(() => {
    void channelKey;
    void features?.buildings?.near_cut_m;
    untrack(markConflicts);
  });
  $effect(() => {
    void terrainOn;
    void exaggeration;
    untrack(() => {
      blocksKey = "";
      raiseBlocks();
    });
  });
  onDestroy(() => {
    map?.off("click", click);
  });

  const TITLES: Record<string, string> = { buildings: "Building", points_of_interest: "Place", settlements: "Settlement" };
</script>

<div hidden data-testid="map-features" data-buildings={buildingsState} data-buildings-ms={buildingsMs ?? ""} data-places={placesCount} data-place-images={placeImages} data-flooded={flooded ? flooded.fids.length : ""} data-place-kinds={kinds.join(" ")} data-place-blocks={blocksDrawn} data-cut-taken={conflicts?.taken ?? ""} data-cut-near={conflicts?.near ?? ""} data-cut-severed={conflicts?.severed ?? ""}></div>
{#if ticks.buildings && conflicts && conflicts.taken + conflicts.near + conflicts.severed > 0}
  <ul class="kinds cut" data-testid="building-cut-legend" aria-label="Buildings against the channel">
    <li><span class="edge" style="border-color: {TAKEN}"></span>In the cut: {conflicts.taken}</li>
    <li><span class="edge" style="border-color: {NEAR}"></span>Within {conflicts.near_cut_m} m: {conflicts.near}</li>
    <li><span class="edge" style="border-color: {SEVERED}"></span>On severed land: {conflicts.severed}</li>
  </ul>
{/if}
{#if ticks.places && kinds.length}
  <ul class="kinds" data-testid="place-legend" aria-label="Kinds of place">
    {#each kinds as k}<li data-kind={k}><span class="mark {PLACE_KINDS.find((x) => x.kind === k)?.shape}">{PLACE_KINDS.find((x) => x.kind === k)?.letter}</span>{PLACE_KINDS.find((x) => x.kind === k)?.label}</li>{/each}
  </ul>
{/if}
{#if card}
  <section class="feature-card" data-testid="feature-card" data-input={card.input} aria-label="{TITLES[card.input]} at the point clicked">
    <header>
      <strong>{TITLES[card.input]}{card.name ? `: ${card.name}` : card.input === "buildings" ? ` ${card.id}` : ""}</strong>
      <button aria-label="Close" data-testid="feature-card-close" onclick={() => (card = null)}>×</button>
    </header>
    <dl>
      {#each cardRows(card) as [k, v]}<dt>{k}</dt><dd>{v}</dd>{/each}
    </dl>
    {#if card.input === "buildings"}<p class="note">Machine-detected footprint, not a survey.</p>{/if}
    {#if card.water !== undefined}<p class="note">Water from the map's pool water; the weir search's count can differ near a weir.</p>{/if}
  </section>
{/if}

<style>
  .feature-card { position: absolute; left: 10px; bottom: 34px; z-index: 5; max-width: 300px; background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); padding: var(--sp-2) var(--sp-3); font-size: var(--fs-12); }
  .feature-card header { display: flex; justify-content: space-between; gap: var(--sp-2); align-items: baseline; }
  .feature-card button { padding: 0 6px; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 2px var(--sp-2); margin: var(--sp-1) 0 0; }
  dt { color: var(--ink-2); }
  dd { margin: 0; font-variant-numeric: tabular-nums; }
  .note { margin: var(--sp-1) 0 0; color: var(--ink-3); font-size: var(--fs-11); }
  .kinds { position: absolute; right: 50px; bottom: 34px; z-index: 3; list-style: none; margin: 0; padding: var(--sp-1) var(--sp-2); background: var(--surface); border: var(--lw-thin) solid var(--ink); font-size: var(--fs-11); color: var(--ink); display: flex; flex-direction: column; gap: 2px; }
  .kinds li { display: flex; align-items: center; gap: var(--sp-1); }
  .mark { display: inline-grid; place-items: center; width: 12px; height: 12px; background: #f2e8cf; border: 1.5px solid #2b2925; color: #2b2925; font-size: 8px; font-weight: 700; line-height: 1; }
  .mark.circle { border-radius: 50%; width: 9px; height: 9px; }
  .kinds.cut { right: auto; left: 10px; bottom: auto; top: 10px; }
  .edge { display: inline-block; width: 10px; height: 10px; border: 2.5px solid; background: #f2e8cf; }
  .mark.diamond { transform: rotate(45deg) scale(0.8); }
</style>
