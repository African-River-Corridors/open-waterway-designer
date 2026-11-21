// Contour lines of the user's DEM on the map (MAP-3). The engine traces them in this browser from the
// DEM in memory, one map tile at a time: the tile address has a scheme of its own, so no request leaves
// the page. The interval is a setting of this machine, like the layer ticks: kept in localStorage, never
// in the design file.

/** The scheme of the contour tile address. The map asks the page for these tiles, never the network. */
export const CONTOUR_PROTOCOL = "owd-contour";

/** The intervals offered, in metres. */
export const INTERVALS = [1, 2, 5, 10, 20] as const;
export type ContourInterval = (typeof INTERVALS)[number];
/** Auto picks the interval from the zoom level. */
export type ContourChoice = "auto" | ContourInterval;
export const DEFAULT_CHOICE: ContourChoice = "auto";

export const CONTOUR_KEY = "owd-map-contours";

/** Further out than this zoom level no contour is drawn: the DEM is too small on the screen to read one. */
export const CONTOUR_MIN_ZOOM = 9;
/** Contour tiles are made down to this zoom level at least, so every interval can be drawn when zoomed in. */
export const CONTOUR_DEEP_ZOOM = 15;
/** Every fifth contour is an index contour: heavier, with its level along it. */
export const INDEX_EVERY = 5;

/** The interval Auto draws at a zoom level, in metres. A screen pixel is about 2.4 m of ground at zoom 15
 * and twice that at each level further out; the interval grows in step (1, 2, 5, 10, 20 m), so lines on a
 * slope of 1 in 20 stay 8 to 10 pixels apart at every zoom level. */
export function autoInterval(zoom: number): ContourInterval {
  if (zoom >= 15) return 1;
  if (zoom >= 14) return 2;
  if (zoom >= 13) return 5;
  if (zoom >= 12) return 10;
  return 20;
}

/** The interval drawn at a zoom level for a choice. A chosen interval is drawn from one zoom level
 * further out than where Auto would pick it (1 m from zoom 14, 2 m from 13, 5 m from 12, 10 m from 11);
 * further out than that the map draws Auto's interval for one level in, so it never floods with lines. */
export function intervalAt(choice: ContourChoice, zoom: number): number {
  return choice === "auto" ? autoInterval(zoom) : Math.max(choice, autoInterval(zoom + 1));
}

/** The choice from the stored text. Anything that is not ours reads as Auto. */
export function parseChoice(text: string | null): ContourChoice {
  if (!text) return DEFAULT_CHOICE;
  try {
    const v: unknown = JSON.parse(text);
    if ((INTERVALS as readonly unknown[]).includes(v)) return v as ContourInterval;
  } catch {
    /* not ours: the default */
  }
  return DEFAULT_CHOICE;
}

/** The stored text for the choice, or null for Auto (nothing to keep). */
export const serialiseChoice = (choice: ContourChoice): string | null => (choice === DEFAULT_CHOICE ? null : JSON.stringify(choice));

/** The remembered interval, or Auto. Storage may be blocked: then Auto. */
export function loadContourChoice(): ContourChoice {
  try {
    return parseChoice(localStorage.getItem(CONTOUR_KEY));
  } catch {
    return DEFAULT_CHOICE;
  }
}

/** Remember the interval on this machine. Storage may be blocked: then it lasts for the session. */
export function saveContourChoice(choice: ContourChoice) {
  try {
    const text = serialiseChoice(choice);
    if (text == null) localStorage.removeItem(CONTOUR_KEY);
    else localStorage.setItem(CONTOUR_KEY, text);
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** The tile address the map source takes for the DEM numbered `id` and an interval choice. A new DEM or
 * another choice has another address, so no old tile is shown for it. With a terrain state other than
 * "before" (MAP-5) the address names the state and the number of what is laid on the ground. */
export const contourTemplate = (id: number, choice: ContourChoice, state = "before", rev = 0) =>
  state === "before" ? `${CONTOUR_PROTOCOL}://${id}/${choice}/{z}/{x}/{y}` : `${CONTOUR_PROTOCOL}://${id}/${state}/${rev}/${choice}/{z}/{x}/{y}`;

/** The tile a contour tile address names, with its terrain state when it is not the DEM as it is, or null
 * when it is not one. */
export function parseContourUrl(url: string): { id: number; choice: ContourChoice; z: number; x: number; y: number; state?: string } | null {
  const m = /^owd-contour:\/\/(\d+)\/(?:([a-z]+)\/\d+\/)?(auto|\d+)\/(\d+)\/(\d+)\/(\d+)$/.exec(url);
  if (!m || m[2] === "auto") return null;
  const choice = m[3] === "auto" ? "auto" : Number(m[3]);
  if (choice !== "auto" && !(INTERVALS as readonly number[]).includes(choice)) return null;
  const [id, z, x, y] = [m[1], m[4], m[5], m[6]].map(Number);
  return { id, choice: choice as ContourChoice, z, x, y, ...(m[2] ? { state: m[2] } : {}) };
}

/** The name of the picture of an index contour's level, which the map asks the page to draw. */
export const LABEL_PREFIX = "owd-contour-level:";
