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
 * Esri World Imagery, as plain raster XYZ tiles: the Static Basemap Tiles service, style `arcgis/imagery/labels`
 * (512 x 512 px PNG, Web Mercator, level 0 to 22). Source: "Static Basemap Tiles service" request
 * `/arcgis/imagery/labels/static/tile/{level}/{row}/{column}`,
 * https://developers.arcgis.com/rest/static-basemap-tiles/arcgis-imagery-labels-tile-get/ , and the service
 * introduction, https://developers.arcgis.com/documentation/mapping-and-location-services/mapping/basemaps/introduction-static-basemap-tiles-service/ .
 * Esri orders the path level/row/column, so `{y}` comes before `{x}`. `{key}` is replaced by the user's key
 * (the documented `token` query parameter; the key needs the `premium:user:staticbasemaptiles` privilege).
 * Chosen over the Basemap Styles service, whose imagery is a vector style for a library that reads Esri's
 * style JSON: this one is a tile address any raster layer takes. Esri has no imagery-only style here: the
 * labels are part of the picture.
 */
export const ESRI_TILES =
  "https://static-map-tiles-api.arcgis.com/arcgis/rest/services/static-basemap-tiles-service/v1/arcgis/imagery/labels/static/tile/{z}/{y}/{x}?token={key}";

/**
 * The service's metadata, which carries `copyrightText`, the data attribution Esri asks every map to show
 * (https://developers.arcgis.com/documentation/esri-and-data-attribution/interactive-maps/). Fetched with the key,
 * which also tells us at once whether the key works.
 */
export const ESRI_METADATA =
  "https://static-map-tiles-api.arcgis.com/arcgis/rest/services/static-basemap-tiles-service/v1/arcgis/imagery/labels/static?f=json&token={key}";

/** The Esri notice the terms require in any application that uses Esri services (same page as above). */
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
    return { source: "esri", tiles: ESRI_TILES.replace("{key}", encodeURIComponent(s.esriKey.trim())), tileSize: 512, maxzoom: 19, attribution: esriAttribution(esriCopyright) };
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
