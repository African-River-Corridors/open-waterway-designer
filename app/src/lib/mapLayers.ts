// The map layers a user can tick on and off (MAP-1). The ticks are a setting of this machine, like the
// theme and the imagery: kept in localStorage, never in the design file.

export const MAP_LAYERS = [
  { id: "river-polygon", label: "River polygon" },
  { id: "centreline", label: "Centreline" },
  { id: "bank-lines", label: "Bank lines" },
  { id: "design-line", label: "Design line" },
  { id: "channel", label: "Channel" },
  { id: "cut", label: "Cut extent" },
  { id: "constraint-stretches", label: "Constraint stretches" },
  { id: "station-marks", label: "Station marks" },
  { id: "structures", label: "Weirs and locks" },
  { id: "pool-levels", label: "Pool levels" },
  { id: "pool-water", label: "Pool water" },
  { id: "places", label: "Places" },
  { id: "buildings", label: "Buildings" },
  { id: "settlements", label: "Settlements" },
  { id: "terrain-shading", label: "Terrain shading" },
  { id: "contours", label: "Contours" },
  { id: "imagery", label: "Imagery" },
] as const;

export type MapLayerId = (typeof MAP_LAYERS)[number]["id"];
/** True: the layer is ticked on. */
export type LayerTicks = Record<MapLayerId, boolean>;

export const LAYERS_KEY = "owd-map-layers";

/** The layers that start unticked. The cut extent colours the ground over the imagery, the pool water
 * covers the ground and takes a moment to work out, terrain shading darkens the imagery, and contours
 * lay lines over everything, so the user asks for them (MAP-2, MAP-3, MAP-4, MAP-5). Buildings and
 * settlements cover the imagery near the river; the places' names are few, so they start on (BLD-1). */
export const STARTS_OFF: readonly MapLayerId[] = ["cut", "pool-water", "terrain-shading", "contours", "buildings", "settlements"];

/** The default ticks: every layer on but those in STARTS_OFF. */
export const defaultTicks = (): LayerTicks => Object.fromEntries(MAP_LAYERS.map((l) => [l.id, !STARTS_OFF.includes(l.id)])) as LayerTicks;

/** The ticks from the stored text: a JSON list of the layers whose tick is not the default (a layer that
 * starts on and is off, or one that starts off and is on). Anything else reads as the default. */
export function parseTicks(text: string | null): LayerTicks {
  const ticks = defaultTicks();
  if (!text) return ticks;
  try {
    const changed: unknown = JSON.parse(text);
    if (!Array.isArray(changed)) return ticks;
    for (const l of MAP_LAYERS) if (changed.includes(l.id)) ticks[l.id] = !ticks[l.id];
  } catch {
    /* not ours: the default */
  }
  return ticks;
}

/** The stored text for the ticks, or null when every tick is the default (nothing to keep). */
export function serialiseTicks(ticks: LayerTicks): string | null {
  const d = defaultTicks();
  const changed = MAP_LAYERS.filter((l) => ticks[l.id] !== d[l.id]).map((l) => l.id);
  return changed.length ? JSON.stringify(changed) : null;
}

/** The remembered ticks, or the default. Storage may be blocked: then the default. */
export function loadLayerTicks(): LayerTicks {
  try {
    return parseTicks(localStorage.getItem(LAYERS_KEY));
  } catch {
    return defaultTicks();
  }
}

/** Remember the ticks on this machine. Storage may be blocked: then they last for the session. */
export function saveLayerTicks(ticks: LayerTicks) {
  try {
    const text = serialiseTicks(ticks);
    if (text == null) localStorage.removeItem(LAYERS_KEY);
    else localStorage.setItem(LAYERS_KEY, text);
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** The map's own layers and sources each tick shows or hides. Markers (labels, bend marks) are handled by the panel. */
export const STYLE_LAYERS: Record<MapLayerId, string[]> = {
  "river-polygon": ["polygon", "polygon-casing", "polygon-outline"],
  centreline: ["centre"],
  "bank-lines": ["banks"],
  "design-line": ["options-casing", "options"],
  channel: ["channel-bed", "channel-bed-casing", "channel-bed-edges", "channel-water"],
  cut: ["cut-depth", "cut-daylight-casing", "cut-daylight"],
  "constraint-stretches": ["stretches-casing", "stretches"],
  "station-marks": ["ticks"],
  // The weir and pool labels are HTML markers, and the 3D blocks show only in the 3D view: the panel sets those.
  structures: ["structures-chamber", "structures-lock", "structures-weir", "structures-outline", "structures-3d"],
  "pool-levels": ["pool-reaches"],
  "pool-water": ["pool-water"],
  // BLD-1: added by MapFeatures.svelte.
  places: ["places", "places-blocks", "places-blocks-3d"],
  buildings: ["buildings-density", "buildings-dots", "buildings-fill", "buildings-outline"],
  settlements: ["settlements-fill", "settlements-outline"],
  "terrain-shading": ["terrain-tint", "terrain-shading"],
  contours: ["contours-casing", "contours", "contours-labels"],
  imagery: ["under", "basemap", "hillshade"],
};
