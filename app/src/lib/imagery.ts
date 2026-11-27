// The imagery setting: which pictures the map draws under the design. A per-user choice kept on this
// machine (like the theme), never in the design file, an export, a report or a drawing. Plain functions
// only, so the rules can be tested without a browser. The key is never put in a message or a log line.
import { ATTRIBUTION, BASEMAP_TILES } from "./basemap.ts";

export type ImagerySource = "open" | "esri" | "custom";

export interface ImagerySettings {
  source: ImagerySource;
  /** The user's own Esri API key. Kept only in this browser's storage. */
  esriKey: string;
  /** An XYZ tile address with {z}, {x} and {y}. */
  customUrl: string;
  /** What the map must show for the custom tiles. Required. */
  customAttribution: string;
}

/**
 * Esri World Imagery, as plain raster XYZ tiles: the World_Imagery map tile service that Esri's Basemap Styles
 * service draws its `arcgis/imagery` style from (256 x 256 px, Web Mercator). Source: "Service data", the list of
 * map tile services, https://developers.arcgis.com/rest/basemap-styles/service-data/ . Esri orders the path
 * level/row/column, so `{y}` comes before `{x}`. `{key}` is replaced by the user's key (the `token` query
 * parameter; the key needs the Basemaps privilege, `premium:user:basemaps`).
 * The Static Basemap Tiles service is not used: its one imagery style, `arcgis/imagery/labels`, is place names
 * only, with no picture.
 */
export const ESRI_TILES =
  "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}?token={key}";

/**
 * The service's metadata, which carries `copyrightText`, the data attribution Esri asks every map to show
 * (https://developers.arcgis.com/documentation/esri-and-data-attribution/interactive-maps/). Fetched with the key,
 * so it also tells us at once whether the key works. Esri does not bill for it.
 */
export const ESRI_METADATA =
  "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer?f=json&token={key}";

/**
 * Esri tiles are asked for only from this zoom level in (about 1 : 500,000 on screen, a river reach). Further out
 * the open imagery shows. Esri bills per tile, and a whole-river view gains nothing from sharper pictures.
 */
export const ESRI_MIN_ZOOM = 10;

export const ESRI_POWERED = "Powered by Esri";

/** The one line shown when Esri imagery fails. */
export const ESRI_FAILED =
  "Esri imagery could not load: check your key (it needs the basemap privilege) or choose another imagery source";
export const CUSTOM_FAILED = "The custom imagery could not load: check the tile address or choose another imagery source";

export const IMAGERY_KEY = "owd-imagery";

export const DEFAULT_IMAGERY: ImagerySettings = { source: "open", esriKey: "", customUrl: "", customAttribution: "" };

/** What the map needs to draw one source. */
export interface ImageryLayer {
  source: ImagerySource;
  tiles: string;
  tileSize: number;
  maxzoom: number;
  /** The layer is drawn, and its tiles asked for, only from this zoom level in. Absent: at every level. */
  minzoom?: number;
  /** Open tiles drawn under the layer, so the map is never empty: further out than `minzoom`, or while tiles load. */
  under?: string;
  /** The attribution text for the source shown, always. Esri's is added after its metadata arrives. */
  attribution: string;
}

/** Whether a custom address has all three placeholders and is a web address. */
export function validTileUrl(url: string): boolean {
  const u = url.trim();
  return /^https?:\/\//i.test(u) && ["{z}", "{x}", "{y}"].every((p) => u.includes(p));
}

/** Whether the settings can be used as they stand: Esri needs a key; custom needs a good address and an attribution. */
export function usable(s: ImagerySettings): boolean {
  if (s.source === "esri") return s.esriKey.trim().length > 0;
  if (s.source === "custom") return validTileUrl(s.customUrl) && s.customAttribution.trim().length > 0;
  return true;
}

/** The key as the field shows it once set: only the last four characters. */
export function maskKey(key: string): string {
  const k = key.trim();
  if (!k) return "";
  return k.length <= 8 ? "••••••••" : "••••••••" + k.slice(-4);
}

/** The Esri metadata address for a key. */
export const esriMetadataUrl = (key: string) => ESRI_METADATA.replace("{key}", encodeURIComponent(key.trim()));

/** Esri's attribution: the notice, then the data sources from the service when we have them. */
export function esriAttribution(copyright: string | null): string {
  const c = (copyright ?? "").trim();
  return c ? `${ESRI_POWERED}. ${c}` : ESRI_POWERED;
}

/** The layer for the settings, or the open default when they cannot be used. `openTiles` is the open address (a test may change it). */
export function layerFor(s: ImagerySettings, openTiles: string = BASEMAP_TILES, esriCopyright: string | null = null): ImageryLayer {
  if (s.source === "esri" && usable(s)) {
    return { source: "esri", tiles: ESRI_TILES.replace("{key}", encodeURIComponent(s.esriKey.trim())), tileSize: 256, maxzoom: 19, minzoom: ESRI_MIN_ZOOM, under: openTiles, attribution: `${esriAttribution(esriCopyright)} | ${ATTRIBUTION}` };
  }
  if (s.source === "custom" && usable(s)) {
    return { source: "custom", tiles: s.customUrl.trim(), tileSize: 256, maxzoom: 19, attribution: s.customAttribution.trim() };
  }
  return { source: "open", tiles: openTiles, tileSize: 256, maxzoom: 14, attribution: ATTRIBUTION };
}

/** What may be stored: the settings, as text. */
export function serialise(s: ImagerySettings): string {
  return JSON.stringify(s);
}

/** Settings from stored text; anything wrong gives the default. */
export function parse(text: string | null): ImagerySettings {
  if (!text) return { ...DEFAULT_IMAGERY };
  try {
    const o = JSON.parse(text) as Partial<ImagerySettings>;
    const str = (v: unknown) => (typeof v === "string" ? v : "");
    const source: ImagerySource = o.source === "esri" || o.source === "custom" ? o.source : "open";
    return { source, esriKey: str(o.esriKey), customUrl: str(o.customUrl), customAttribution: str(o.customAttribution) };
  } catch {
    return { ...DEFAULT_IMAGERY };
  }
}

/** The remembered settings, or the default. Storage may be blocked: then the default. */
export function loadImagery(): ImagerySettings {
  try {
    return parse(localStorage.getItem(IMAGERY_KEY));
  } catch {
    return { ...DEFAULT_IMAGERY };
  }
}

/** Remember the settings on this machine. Storage may be blocked: then they last for the session. */
export function saveImagery(s: ImagerySettings) {
  try {
    if (s.source === "open" && !s.esriKey && !s.customUrl && !s.customAttribution) localStorage.removeItem(IMAGERY_KEY);
    else localStorage.setItem(IMAGERY_KEY, serialise(s));
  } catch {
    /* storage is blocked: nothing to remember */
  }
}
