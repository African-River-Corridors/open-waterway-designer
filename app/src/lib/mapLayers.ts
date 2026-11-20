// The map layers a user can tick on and off (MAP-1). The ticks are a setting of this machine, like the
// theme and the imagery: kept in localStorage, never in the design file.

export const MAP_LAYERS = [
  { id: "river-polygon", label: "River polygon" },
  { id: "centreline", label: "Centreline" },
  { id: "bank-lines", label: "Bank lines" },
  { id: "design-line", label: "Design line" },
  { id: "constraint-stretches", label: "Constraint stretches" },
  { id: "station-marks", label: "Station marks" },
  { id: "terrain-shading", label: "Terrain shading" },
  { id: "imagery", label: "Imagery" },
] as const;

export type MapLayerId = (typeof MAP_LAYERS)[number]["id"];
/** True: the layer is ticked on. */
export type LayerTicks = Record<MapLayerId, boolean>;

export const LAYERS_KEY = "owd-map-layers";

/** The layers that start unticked. Terrain shading darkens the imagery, so the user asks for it (MAP-2). */
export const STARTS_OFF: readonly MapLayerId[] = ["terrain-shading"];

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
  "constraint-stretches": ["stretches-casing", "stretches"],
  "station-marks": ["ticks"],
  "terrain-shading": ["terrain-tint", "terrain-shading"],
  imagery: ["under", "basemap", "hillshade"],
};
