// Where the floating panels sit: a per-user setting, kept in this browser and never in a design.

/** A panel's place over the map, in CSS pixels from the map area's top-left corner. */
export interface PanelRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

const KEY = "owd-panels";
export const MIN_W = 240;
export const MIN_H = 140;

const isRect = (r: unknown): r is PanelRect =>
  typeof r === "object" && r !== null && (["x", "y", "w", "h"] as const).every((k) => Number.isFinite((r as Record<string, unknown>)[k]));

function readAll(): Record<string, PanelRect> {
  try {
    const all: unknown = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    if (typeof all !== "object" || all === null) return {};
    return Object.fromEntries(Object.entries(all).filter(([, r]) => isRect(r))) as Record<string, PanelRect>;
  } catch {
    /* storage is blocked, or holds something else: use the defaults */
    return {};
  }
}

/** The remembered place of a panel, if there is one. */
export function storedRect(id: string): PanelRect | null {
  return readAll()[id] ?? null;
}

/** Remember a panel's place. Storage may be unavailable: then it lasts for the session. */
export function storeRect(id: string, rect: PanelRect) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...readAll(), [id]: rect }));
  } catch {
    /* storage is blocked: nothing to remember */
  }
}

/** Keep a panel inside an area of the given size, and no smaller than it can be used at. */
export function clampRect(r: PanelRect, width: number, height: number): PanelRect {
  const w = Math.round(Math.min(Math.max(r.w, MIN_W), Math.max(width, MIN_W)));
  const h = Math.round(Math.min(Math.max(r.h, MIN_H), Math.max(height, MIN_H)));
  const x = Math.round(Math.min(Math.max(r.x, 0), Math.max(width - w, 0)));
  const y = Math.round(Math.min(Math.max(r.y, 0), Math.max(height - h, 0)));
  return { x, y, w, h };
}
