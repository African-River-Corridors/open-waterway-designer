// The map layers a user can tick on and off (MAP-1). The ticks are a setting of this machine, like the
// theme and the imagery: kept in localStorage, never in the design file.

export const MAP_LAYERS = [
  { id: "river-polygon", label: "River polygon" },
  { id: "centreline", label: "Centreline" },
  { id: "bank-lines", label: "Bank lines" },
  { id: "design-line", label: "Design line" },
  { id: "constraint-stretches", label: "Constraint stretches" },
  { id: "station-marks", label: "Station marks" },
  { id: "imagery", label: "Imagery" },
] as const;

export type MapLayerId = (typeof MAP_LAYERS)[number]["id"];
/** True: the layer is ticked on. */
export type LayerTicks = Record<MapLayerId, boolean>;

export const LAYERS_KEY = "owd-map-layers";

/** Every layer on: the default. */
export const allOn = (): LayerTicks => Object.fromEntries(MAP_LAYERS.map((l) => [l.id, true])) as LayerTicks;

/** The ticks from the stored text: a JSON list of the layers turned off. Anything else reads as all on. */
export function parseTicks(text: string | null): LayerTicks {
  const ticks = allOn();
  if (!text) return ticks;
  try {
    const off: unknown = JSON.parse(text);
    if (!Array.isArray(off)) return ticks;
    for (const l of MAP_LAYERS) if (off.includes(l.id)) ticks[l.id] = false;
  } catch {
    /* not ours: all on */
  }
  return ticks;
}

/** The stored text for the ticks, or null when every layer is on (nothing to keep). */
export function serialiseTicks(ticks: LayerTicks): string | null {
  const off = MAP_LAYERS.filter((l) => !ticks[l.id]).map((l) => l.id);
  return off.length ? JSON.stringify(off) : null;
}

/** The remembered ticks, or all on. Storage may be blocked: then all on. */
export function loadLayerTicks(): LayerTicks {
  try {
    return parseTicks(localStorage.getItem(LAYERS_KEY));
  } catch {
    return allOn();
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
  imagery: ["under", "basemap", "hillshade"],
};
