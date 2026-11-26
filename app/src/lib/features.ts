// Buildings, places and settlements on the map (BLD-1). Pure helpers: the kinds of place and their marks,
// the point the map draws a building at when zoomed out, the order the names are placed in, the label
// pictures' names, and the words of a click card. The map work is in MapFeatures.svelte.
//
// Written so that Node can run it without a build (type stripping): no parameter properties, no enums.

/** From this zoom level in, buildings are drawn as their footprints; further out, as dots and a density. */
export const FOOTPRINT_ZOOM = 14;
/** From this zoom level in, buildings are dots; further out only the density shows. */
export const DOT_ZOOM = 12;

/** The kinds of place the map knows, each with a mark: a shape and a letter, so colour is never the only signal. */
export const PLACE_KINDS = [
  { kind: "community", label: "Community", shape: "circle", letter: "" },
  { kind: "bridge", label: "Bridge", shape: "square", letter: "B" },
  { kind: "quay", label: "Quay", shape: "square", letter: "Q" },
  { kind: "loading_site", label: "Loading site", shape: "square", letter: "L" },
  { kind: "ferry", label: "Ferry", shape: "diamond", letter: "F" },
  { kind: "intake", label: "Intake", shape: "diamond", letter: "I" },
  { kind: "structure", label: "Structure", shape: "square", letter: "S" },
  { kind: "other", label: "Other", shape: "diamond", letter: "" },
] as const;
export type PlaceKind = (typeof PLACE_KINDS)[number]["kind"];

/** The kind a place's `kind` property names: lower case, spaces and dashes read as `_`; anything else is "other". */
export function placeKind(kind: unknown): PlaceKind {
  const k = String(kind ?? "")
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
  if (k === "") return "community";
  return (PLACE_KINDS.find((p) => p.kind === k)?.kind ?? "other") as PlaceKind;
}

/** The kinds present in a list of places, in the registry's order: what the legend lists. */
export function kindsPresent(kinds: unknown[]): PlaceKind[] {
  const seen = new Set(kinds.map(placeKind));
  return PLACE_KINDS.map((p) => p.kind).filter((k) => seen.has(k));
}

/** The prefix of a place label picture's name. The rest is the kind and the name. */
export const PLACE_LABEL_PREFIX = "owd-place:";
/** The picture's name for a place: one picture per distinct kind and name. */
export const placeImageId = (kind: PlaceKind, name: string) => `${PLACE_LABEL_PREFIX}${kind}:${name}`;
/** The kind and the name back from a picture's name, or null when it is not one. */
export function parsePlaceImageId(id: string): { kind: PlaceKind; name: string } | null {
  if (!id.startsWith(PLACE_LABEL_PREFIX)) return null;
  const rest = id.slice(PLACE_LABEL_PREFIX.length);
  const at = rest.indexOf(":");
  if (at < 0) return null;
  return { kind: placeKind(rest.slice(0, at)), name: rest.slice(at + 1) };
}
/** The longest name drawn in full; a longer one is cut with an ellipsis (the card has it whole). */
export const NAME_MAX = 28;
export const shortName = (name: string) => (name.length > NAME_MAX ? `${name.slice(0, NAME_MAX - 1)}…` : name);

/** The order names are placed in: the larger place first (by `area_ha`, when the file gives it), so where
 * two names would collide the larger one stays. MapLibre places a lower sort key first. */
export const placeSortKey = (areaHa: unknown) => {
  const a = Number(areaHa);
  return Number.isFinite(a) && a > 0 ? -a : 0;
};

/** A point for a footprint: the mean of its outer ring's corners (the last, which repeats the first, left out). */
export function ringCentre(geometry: { type: string; coordinates: unknown }): [number, number] | null {
  const ring = (g: unknown): number[][] | null => {
    if (geometry.type === "Point") return [g as number[]];
    if (geometry.type === "Polygon") return (g as number[][][])[0] ?? null;
    if (geometry.type === "MultiPolygon") return (g as number[][][][])[0]?.[0] ?? null;
    if (geometry.type === "LineString") return g as number[][];
    return null;
  };
  const r = ring(geometry.coordinates);
  if (!r?.length) return null;
  const pts = r.length > 1 && r[0][0] === r[r.length - 1][0] && r[0][1] === r[r.length - 1][1] ? r.slice(0, -1) : r;
  let x = 0;
  let y = 0;
  for (const p of pts) {
    x += p[0];
    y += p[1];
  }
  return [x / pts.length, y / pts.length];
}

/** What a click on a building, a place or a settlement reads, as the engine answers it. */
export interface FeatureCard {
  input: "buildings" | "points_of_interest" | "settlements";
  fid: number;
  id: string;
  name: string;
  kind: string;
  properties: Record<string, unknown>;
  lon_lat: [number, number];
  area_m2: number | null;
  area_ha?: number | null;
  confidence: number | null;
  below_confidence?: boolean;
  below_area?: boolean;
  ground_m: number | null;
  line_distance_m?: number | null;
  line_chainage_m?: number | null;
  cut?: { chainage_m: number; offset_m: number; outside_m: number; daylight: boolean; from_volumes: boolean; relation: "taken" | "near" | "clear" } | null;
  /** The map's pool water there: null on dry ground; absent with the pool water off. */
  water?: { pool: number; water_level_m: number; ground_m: number; depth_m: number } | null;
  /** Settlements, with the pool water on: the share of the settlement the water covers. */
  flooded_share?: number | null;
}

const m = (v: number | null | undefined, d = 1) => (v == null || !Number.isFinite(v) ? "unknown" : `${v.toFixed(d)} m`);

/** The rows of a card: label and value, in the order shown. */
export function cardRows(c: FeatureCard): [string, string][] {
  const rows: [string, string][] = [];
  if (c.input !== "buildings") {
    if (c.name) rows.push(["Name", c.name]);
    if (c.input === "points_of_interest") rows.push(["Kind", PLACE_KINDS.find((k) => k.kind === placeKind(c.kind))!.label + (placeKind(c.kind) === "other" && c.kind ? ` (${c.kind})` : "")]);
    for (const [k, v] of Object.entries(c.properties ?? {})) {
      if (k === "name" || k === "kind" || v == null || v === "") continue;
      rows.push([k, typeof v === "number" ? String(Math.round(v * 100) / 100) : String(v)]);
    }
  }
  if (c.input === "buildings") {
    rows.push(["Area", c.area_m2 == null ? "unknown" : `${Math.round(c.area_m2)} m²`]);
    rows.push(["Confidence", c.confidence == null ? "not given" : `${c.confidence.toFixed(2)}${c.below_confidence ? " (below the threshold: not costed)" : ""}`]);
    if (c.below_area) rows.push(["Counted", "no: below the smallest area"]);
  }
  if (c.input === "settlements" && c.area_ha != null) rows.push(["Area", `${c.area_ha.toFixed(1)} ha`]);
  rows.push(["Ground level", m(c.ground_m, 2)]);
  if (c.water !== undefined) {
    if (c.water) {
      rows.push(["Pool water level", `${c.water.water_level_m.toFixed(2)} m (pool ${c.water.pool})`]);
      rows.push(["Depth of flooding", `${c.water.depth_m.toFixed(2)} m`]);
    } else rows.push(["Flooding", "dry"]);
  }
  if (c.flooded_share != null) rows.push(["Area flooded", `${Math.round(c.flooded_share * 100)} %`]);
  if (c.input === "points_of_interest" && placeBlock(c.properties ?? {}, c.lon_lat)) rows.push(["Model", PLACE_BLOCK_NOTE]);
  if (c.line_distance_m != null) rows.push(["To the design line", m(c.line_distance_m, 0)]);
  if (c.line_chainage_m != null) rows.push(["Chainage of the nearest point", `${(c.line_chainage_m / 1000).toFixed(3)} km`]);
  if (c.cut) {
    const words = c.cut.relation === "taken" ? "inside the cut" : `${c.cut.outside_m.toFixed(0)} m outside it`;
    rows.push([c.cut.daylight ? "To the daylight line" : "To the channel's water edge", words]);
  } else if (c.cut === null) rows.push(["To the daylight line", "far from the channel"]);
  return rows;
}

/** What a placeholder block of a place reads (BLD-1 groundwork for 3D models): drawn only for a bridge or a
 * quay that gives its size. Not a design: like the weirs and locks, a block shows where and how big. */
export const PLACE_BLOCK_NOTE = "placeholder model, not a design";
/** The kinds that may have a block, and the shape each is: a bridge a deck across the river at its level,
 * a quay a slab along the bank. */
export const BLOCK_KINDS: PlaceKind[] = ["bridge", "quay"];

/** A place's block: its four corners (longitude, latitude; closed), and how its top is set. `top_m` is a
 * level (the bridge's deck); `height_m` a height above the ground (a quay with no level given). Null when
 * the place has no block: another kind, or a size not given. */
export interface PlaceBlock {
  ring: [number, number][];
  kind: PlaceKind;
  top_m: number | null;
  height_m: number | null;
  /** The deck or slab's thickness, metres: the block is drawn from top less this to the top. */
  thickness_m: number;
}

const num = (v: unknown) => (v == null || v === "" ? NaN : Number(v));

export function placeBlock(props: Record<string, unknown>, at: [number, number]): PlaceBlock | null {
  const kind = placeKind(props.kind);
  const wanted = BLOCK_KINDS.includes(kind) || props.model === "block";
  if (!wanted) return null;
  const length = num(props.length_m);
  const width = num(props.width_m);
  if (!(length > 0 && width > 0)) return null;
  const bearing = Number.isFinite(num(props.bearing_deg)) ? num(props.bearing_deg) : 0;
  const deck = num(props.deck_level_m);
  const height = num(props.height_m);
  // Metres to degrees at this latitude (a block is small: the plane is close enough).
  const mLat = 111_320;
  const mLon = 111_320 * Math.cos((at[1] * Math.PI) / 180);
  const b = (bearing * Math.PI) / 180;
  // The long axis along the bearing (clockwise from north), the short axis square to it.
  const along: [number, number] = [Math.sin(b), Math.cos(b)];
  const across: [number, number] = [Math.cos(b), -Math.sin(b)];
  const corner = (u: number, v: number): [number, number] => {
    const e = (u * along[0] + v * across[0]) / mLon;
    const n = (u * along[1] + v * across[1]) / mLat;
    return [at[0] + e, at[1] + n];
  };
  const [hl, hw] = [length / 2, width / 2];
  const ring = [corner(-hl, -hw), corner(hl, -hw), corner(hl, hw), corner(-hl, hw), corner(-hl, -hw)];
  return {
    ring,
    kind,
    top_m: Number.isFinite(deck) ? deck : null,
    height_m: Number.isFinite(deck) ? null : Number.isFinite(height) ? height : 1,
    thickness_m: kind === "bridge" ? 1.5 : Number.isFinite(height) ? height : 1,
  };
}
