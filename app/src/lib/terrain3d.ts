// The 3D view of the user's DEM (MAP-2). The map reads its terrain from tiles the engine makes in this
// browser from the DEM in memory: the tile address has a scheme of its own, so no request leaves the page.
// 3D on or off and the vertical exaggeration are settings of this machine, like the layer ticks: kept in
// localStorage, never in the design file.

/** The scheme of the terrain tile address. The map asks the page for these tiles, never the network. */
export const TERRAIN_PROTOCOL = "owd-dem";

/** The vertical exaggerations offered. */
export const EXAGGERATIONS = [1, 2, 5, 10] as const;
/** River country is flat: twice the height shows the banks and the floodplain. */
export const DEFAULT_EXAGGERATION = 2;
/** The tilt of the 3D view, in degrees from straight down. */
export const PITCH_3D = 60;

export interface TerrainPrefs {
  /** The user turned 3D on. The map shows it only while a DEM is in memory. */
  on: boolean;
  exaggeration: number;
}

export const TERRAIN_KEY = "owd-map-3d";

export const defaultPrefs = (): TerrainPrefs => ({ on: false, exaggeration: DEFAULT_EXAGGERATION });

/** The settings from the stored text. Anything that is not ours reads as the default. */
export function parsePrefs(text: string | null): TerrainPrefs {
  const prefs = defaultPrefs();
  if (!text) return prefs;
  try {
    const v: unknown = JSON.parse(text);
    if (v && typeof v === "object" && !Array.isArray(v)) {
      const o = v as { on?: unknown; exaggeration?: unknown };
      if (o.on === true) prefs.on = true;
      if ((EXAGGERATIONS as readonly unknown[]).includes(o.exaggeration)) prefs.exaggeration = o.exaggeration as number;
    }
  } catch {
    /* not ours: the default */
  }
  return prefs;
}

/** The stored text for the settings, or null when they are the default (nothing to keep). */
export function serialisePrefs(prefs: TerrainPrefs): string | null {
  const d = defaultPrefs();
  return prefs.on === d.on && prefs.exaggeration === d.exaggeration ? null : JSON.stringify({ on: prefs.on, exaggeration: prefs.exaggeration });
}

/** The remembered settings, or the default. Storage may be blocked: then the default. */
export function loadTerrainPrefs(): TerrainPrefs {
  try {
    return parsePrefs(localStorage.getItem(TERRAIN_KEY));
  } catch {
    return defaultPrefs();
  }
}

/** Remember the settings on this machine. Storage may be blocked: then they last for the session. */
export function saveTerrainPrefs(prefs: TerrainPrefs) {
  try {
    const text = serialisePrefs(prefs);
    if (text == null) localStorage.removeItem(TERRAIN_KEY);
    else localStorage.setItem(TERRAIN_KEY, text);
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** The tile address the map source takes for the DEM numbered `id`. A new DEM has a new number, so no old tile is shown for it. */
export const tileTemplate = (id: number) => `${TERRAIN_PROTOCOL}://${id}/{z}/{x}/{y}`;

/** The tile a terrain tile address names, or null when it is not one. */
export function parseTileUrl(url: string): { id: number; z: number; x: number; y: number } | null {
  const m = /^owd-dem:\/\/(\d+)\/(\d+)\/(\d+)\/(\d+)$/.exec(url);
  if (!m) return null;
  const [id, z, x, y] = m.slice(1).map(Number);
  return { id, z, x, y };
}
