// What the cross-section panel draws of the cut face (MAP-5). The stored profile keeps the cut template
// out to the ends of the section, where the dry slope runs on far above the ground. The panel draws the
// face only as far as the cut goes: out to the daylight point on each side, where the face meets the
// ground. Display geometry only: every level comes from the session.

export interface FaceInput {
  offsets_m: number[];
  terrain_m: (number | null)[];
  cut_face_m: (number | null)[];
  water_level_m: number;
  design_invert_m: number;
}

export interface FacePoint {
  offset_m: number;
  level_m: number;
}

export interface FaceView {
  /** The face to draw, left to right: the channel under water and the cut slopes up to the daylight points. */
  points: FacePoint[];
  /** Where the cut face meets the ground on each side; null where that side has no cut. */
  daylight: { left: FacePoint | null; right: FacePoint | null };
  /** The lowest and highest level drawn: the ground, the drawn face, the water level and the invert. */
  lo: number;
  hi: number;
}

const finite = (v: number | null | undefined): v is number => v != null && Number.isFinite(v);

/** The face as drawn, its daylight points, and the levels the plot is scaled to. */
export function faceView(s: FaceInput): FaceView {
  const off = s.offsets_m;
  const n = off.length;
  /** The face is at or below the ground at sample `i`: the sample is cut (the volumes' own test). */
  const cut = (i: number) => finite(s.cut_face_m[i]) && finite(s.terrain_m[i]) && (s.cut_face_m[i] as number) <= (s.terrain_m[i] as number);
  /** The sample is part of the channel under water: drawn even where the ground is already below it. */
  const wet = (i: number) => finite(s.cut_face_m[i]) && (s.cut_face_m[i] as number) <= s.water_level_m + 1e-9;
  // The outermost cut sample on each side: the top of cut.
  let topLeft = -1;
  let topRight = -1;
  for (let i = 0; i < n; i += 1) {
    if (!cut(i)) continue;
    if (off[i] < 0 && topLeft < 0) topLeft = i;
    if (off[i] > 0) topRight = i;
  }
  // The face runs from the further of the top of cut and the water's edge on the left to the same on the right.
  let from = -1;
  let to = -1;
  for (let i = 0; i < n; i += 1) {
    if (!wet(i)) continue;
    if (from < 0) from = i;
    to = i;
  }
  if (topLeft >= 0 && (from < 0 || topLeft < from)) from = topLeft;
  if (topRight >= 0 && topRight > to) to = topRight;
  if (from < 0 && topRight >= 0) from = topRight;
  if (to < 0 && topLeft >= 0) to = topLeft;
  /** The point between cut sample `i` and the next sample out (`step` is -1 on the left) where the face
   * meets the ground; the sample itself when the next one cannot say. */
  const crossing = (i: number, step: number): FacePoint => {
    const at: FacePoint = { offset_m: off[i], level_m: s.cut_face_m[i] as number };
    const j = i + step;
    if (j < 0 || j >= n || !finite(s.cut_face_m[j]) || !finite(s.terrain_m[j])) return at;
    const g0 = (s.terrain_m[i] as number) - (s.cut_face_m[i] as number);
    const g1 = (s.terrain_m[j] as number) - (s.cut_face_m[j] as number);
    if (!(g0 >= 0 && g1 < 0)) return at;
    const t = g0 / (g0 - g1);
    return { offset_m: off[i] + (off[j] - off[i]) * t, level_m: (s.cut_face_m[i] as number) + ((s.cut_face_m[j] as number) - (s.cut_face_m[i] as number)) * t };
  };
  const left = topLeft >= 0 ? crossing(topLeft, -1) : null;
  const right = topRight >= 0 ? crossing(topRight, 1) : null;
  const points: FacePoint[] = [];
  if (from >= 0 && to >= from) {
    if (left && from === topLeft && left.offset_m < off[from]) points.push(left);
    for (let i = from; i <= to; i += 1) if (finite(s.cut_face_m[i])) points.push({ offset_m: off[i], level_m: s.cut_face_m[i] as number });
    if (right && to === topRight && right.offset_m > off[to]) points.push(right);
  }
  const levels = [...s.terrain_m.filter(finite), ...points.map((p) => p.level_m), s.water_level_m, s.design_invert_m];
  return { points, daylight: { left, right }, lo: Math.min(...levels), hi: Math.max(...levels) };
}
