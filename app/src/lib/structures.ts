// The weirs, the locks and the pools on the map (MAP-4). The session makes the list on demand from the
// design; this file turns it into what the map draws, and names the tile addresses of the pool water,
// which the engine makes in this browser from the DEM in memory. Nothing here is kept in the design file.
import type * as GeoJSON from "geojson";
import type { LonLat, MapStructures, ModelPart, PoolReach, WaterAt, WeirSite } from "./api";

/** What every name of the block model carries. */
export const PLACEHOLDER_NOTE = "placeholder model, not a design";

/** The scheme of the pool water tile address: the picture of the water, coloured by depth. */
export const POOL_PROTOCOL = "owd-pool";
/** The scheme of the terrain with the water on it, for the 3D view. */
export const POOL_SURFACE_PROTOCOL = "owd-pool-dem";

/** The tile address the map source takes for pool water `id`. A new flood mask has a new number. */
export const poolTemplate = (id: number) => `${POOL_PROTOCOL}://${id}/{z}/{x}/{y}`;
export const poolSurfaceTemplate = (id: number, state = "before", rev = 0) => (state === "before" ? `${POOL_SURFACE_PROTOCOL}://${id}/{z}/{x}/{y}` : `${POOL_SURFACE_PROTOCOL}://${id}/${state}/${rev}/{z}/{x}/{y}`);

/** The tile a pool water address names (either scheme), or null when it is not one. The terrain with the
 * water on it may name the terrain state it stands on (MAP-5: "after", the cut channel filled to the pool
 * level) and the number of the cut. */
export function parsePoolUrl(url: string): { surface: boolean; id: number; z: number; x: number; y: number; state?: string } | null {
  const m = /^owd-pool(-dem)?:\/\/(\d+)\/(?:([a-z]+)\/\d+\/)?(\d+)\/(\d+)\/(\d+)$/.exec(url);
  if (!m || (m[3] && !m[1])) return null;
  const [id, z, x, y] = [m[2], m[4], m[5], m[6]].map(Number);
  return { surface: !!m[1], id, z, x, y, ...(m[3] ? { state: m[3] } : {}) };
}

const m1 = (v: number) => v.toFixed(1);
/** A chainage in kilometres, to 100 m: "km 110.3". */
export const km = (chainage_m: number) => `km ${(chainage_m / 1000).toFixed(1)}`;

/** The label of a weir: its number, where it is, the pool level it holds (upstream) and its lift. */
export const weirLabel = (w: WeirSite) => `Weir ${w.number} · ${km(w.chainage_m)} · pool ${m1(w.upstream_level_m)} m · lift ${m1(w.lift_m)} m`;
/** The label of a pool, at the middle of its reach. */
export const poolLabel = (p: PoolReach) => `Pool ${p.number} · ${m1(p.level_m)} m`;
/** What a click on the pool water reads. */
export const waterReadout = (w: WaterAt) => `water level ${m1(w.water_level_m)} m, ground ${m1(w.ground_m)} m, depth ${m1(w.depth_m)} m, pool ${w.pool}`;

/** The lock chamber's size in words, or why there is no lock. */
export function chamberText(s: MapStructures): string {
  const m = s.model;
  if (m.chamber_length_m == null || m.chamber_width_m == null) return "no lock drawn: choose a vessel first";
  return `${m1(m.chamber_length_m)} m × ${m1(m.chamber_width_m)} m`;
}

const feature = (geometry: GeoJSON.Geometry, properties: GeoJSON.GeoJsonProperties): GeoJSON.Feature => ({ type: "Feature", geometry, properties });

/** The blocks of every weir and lock, flat: what the 2D map draws. */
export function partFeatures(s: MapStructures | null): GeoJSON.Feature[] {
  return (s?.weirs ?? []).flatMap((w) => w.parts.map((p) => feature({ type: "Polygon", coordinates: [p.ring] }, { weir: w.number, kind: p.kind })));
}

/** Each pool's reach of the design line. */
export function poolFeatures(s: MapStructures | null): GeoJSON.Feature[] {
  return (s?.pools ?? []).filter((p) => p.line.length > 1).map((p) => feature({ type: "LineString", coordinates: p.line }, { pool: p.number, odd: p.number % 2 === 1 }));
}

/** The middle of a block: the mean of its corners, the point the map reads the ground at for an extruded shape. */
export function partCentre(p: ModelPart): LonLat {
  const ring = p.ring.length > 1 && p.ring[0][0] === p.ring[p.ring.length - 1][0] && p.ring[0][1] === p.ring[p.ring.length - 1][1] ? p.ring.slice(0, -1) : p.ring;
  const n = Math.max(ring.length, 1);
  return [ring.reduce((a, c) => a + c[0], 0) / n, ring.reduce((a, c) => a + c[1], 0) / n];
}

/** How tall the map must draw a block for its top to stand at `top_m`.
 *
 * The map (MapLibre `fill-extrusion` over terrain) measures a block's height from the terrain at the middle of
 * the block (the mean of its corners), not from sea level, and gives the whole block that one base: its top is
 * flat. The terrain is drawn `exaggeration` times taller and the block's height is not, so the height that
 * puts the top at `top_m` on the drawn terrain is `top_m * exaggeration` less the drawn terrain height there.
 * `drawn_ground_m` is what `map.queryTerrainElevation` gives at the middle (it has the exaggeration in it). */
export function extrusionHeight(top_m: number, drawn_ground_m: number, exaggeration: number): number {
  return Math.max(top_m * exaggeration - drawn_ground_m, 0);
}

/** The blocks with the height each is drawn at, for the 3D view. `ground` gives the drawn terrain height at a point. */
export function extrusionFeatures(s: MapStructures | null, exaggeration: number, ground: (at: LonLat) => number): GeoJSON.Feature[] {
  return (s?.weirs ?? []).flatMap((w) =>
    w.parts.map((p) => {
      // To the nearest 5 cm, so that a tile arriving does not redraw the blocks for nothing.
      const height = Math.round(extrusionHeight(p.top_m, ground(partCentre(p)), exaggeration) * 20) / 20;
      return feature({ type: "Polygon", coordinates: [p.ring] }, { weir: w.number, kind: p.kind, top_m: p.top_m, height });
    }),
  );
}

/** A CSS colour for a depth stop of the pool water. */
export const stopColour = (c: [number, number, number, number]) => `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${(c[3] / 255).toFixed(2)})`;
