// The channel and its cut on the map (MAP-5). The engine derives the channel's edges from the design, and
// makes the cut-depth picture one map tile at a time from the DEM in memory: the tile address has a scheme
// of its own, so no request leaves the page. Which ground the 3D view shows (before or after the cut) is a
// setting of this machine, like the layer ticks: kept in localStorage, never in the design file.
import type { LonLat } from "./api";

/** The two edges of one kind, as lines, and the band between them as closed rings: one of each for every
 * run of stations on the DEM. */
export interface ChannelEdges {
  left: LonLat[][];
  right: LonLat[][];
  polygons: LonLat[][];
}

/** The channel of the design line as the map draws it. */
export interface ChannelGeometry {
  option_id: string;
  stations: number;
  /** The stations are the ones the volumes stage stored. */
  from_volumes: boolean;
  /** The edges of the channel bed. */
  bed: ChannelEdges;
  /** Where the side slopes meet the pool's water level. */
  water_surface: ChannelEdges;
  /** Where the cut slopes meet the existing ground. Null with no ground read or no stored stations. */
  daylight: ChannelEdges | null;
}

/** One depth class of the cut-depth picture, with its two colours. */
export interface CutClass {
  from_m: number;
  /** Null: no upper end. */
  to_m: number | null;
  dredge: string;
  dry: string;
}

/** What the engine answers for the channel: the geometry, or why there is none yet. */
export interface ChannelView {
  /** A text that changes when the channel may have changed. */
  key: string;
  geometry: ChannelGeometry | null;
  note: string | null;
  legend: CutClass[];
}

/** What a click on the cut reads out. */
export interface CutAt {
  chainage_m: number;
  offset_m: number;
  ground_m: number;
  after_m: number;
  depth_m: number;
  kind: "dredge" | "dry";
  station_chainage_m: number;
  dredge_area_m2: number;
  excav_area_m2: number;
  max_face_h_m: number;
}

/** The ground the 3D view shows: as it is, or with the channel cut into it. */
export const TERRAIN_STATES = [
  { id: "before", label: "Before", title: "Before: the existing ground" },
  { id: "after", label: "After", title: "After: the ground with the channel cut into it" },
] as const;
export type TerrainState = (typeof TERRAIN_STATES)[number]["id"];
export const DEFAULT_STATE: TerrainState = "before";
export const STATE_KEY = "owd-map-terrain-state";

/** The state from the stored text. Anything that is not ours reads as Before. */
export function parseState(text: string | null): TerrainState {
  return TERRAIN_STATES.some((s) => s.id === text) ? (text as TerrainState) : DEFAULT_STATE;
}

/** The stored text for the state, or null for Before (nothing to keep). */
export const serialiseState = (state: TerrainState): string | null => (state === DEFAULT_STATE ? null : state);

/** The remembered state, or Before. Storage may be blocked: then Before. */
export function loadTerrainState(): TerrainState {
  try {
    return parseState(localStorage.getItem(STATE_KEY));
  } catch {
    return DEFAULT_STATE;
  }
}

/** Remember the state on this machine. Storage may be blocked: then it lasts for the session. */
export function saveTerrainState(state: TerrainState) {
  try {
    const text = serialiseState(state);
    if (text == null) localStorage.removeItem(STATE_KEY);
    else localStorage.setItem(STATE_KEY, text);
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** The other state: what hold-to-compare shows. */
export const otherState = (state: TerrainState): TerrainState => (state === "before" ? "after" : "before");

/** The scheme of the cut-depth tile address. The map asks the page for these tiles, never the network. */
export const CUT_PROTOCOL = "owd-cut";

/** The tile address of the cut-depth picture for the DEM numbered `id` and the cut numbered `cut`. A new
 * DEM or a new cut has another address, so no old tile is shown for it. */
export const cutTemplate = (id: number, cut: number) => `${CUT_PROTOCOL}://${id}/${cut}/{z}/{x}/{y}`;

/** The tile a cut-depth tile address names, or null when it is not one. */
export function parseCutUrl(url: string): { id: number; cut: number; z: number; x: number; y: number } | null {
  const m = /^owd-cut:\/\/(\d+)\/(\d+)\/(\d+)\/(\d+)\/(\d+)$/.exec(url);
  if (!m) return null;
  const [id, cut, z, x, y] = m.slice(1).map(Number);
  return { id, cut, z, x, y };
}

/** A depth class in words: "0 to 1 m", "20 m and more". */
export const classLabel = (c: CutClass) => (c.to_m == null ? `${c.from_m} m and more` : `${c.from_m} to ${c.to_m} m`);

/** A chainage as kilometres plus metres: 12+345. */
export const chainageLabel = (m: number) => `${Math.floor(m / 1000)}+${String(Math.floor(m % 1000)).padStart(3, "0")}`;

/** The rows of the read-out of a point of the cut, in the order the engineer checks them. */
export function readout(c: CutAt): [string, string][] {
  const side = c.offset_m < 0 ? "left" : "right";
  return [
    ["Chainage", `${chainageLabel(c.chainage_m)} (${Math.abs(c.offset_m).toFixed(1)} m ${side} of the line)`],
    ["Ground before", `${c.ground_m.toFixed(2)} m`],
    ["Level after", `${c.after_m.toFixed(2)} m`],
    ["Depth of cut", `${c.depth_m.toFixed(2)} m`],
    ["Kind", c.kind === "dredge" ? "dredge (in the river)" : "dry cut"],
    [`Station ${chainageLabel(c.station_chainage_m)}`, `dredge ${c.dredge_area_m2.toFixed(0)} m², dry cut ${c.excav_area_m2.toFixed(0)} m², face ${c.max_face_h_m.toFixed(1)} m`],
  ];
}
