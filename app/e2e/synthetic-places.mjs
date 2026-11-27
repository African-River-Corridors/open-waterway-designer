// Synthetic buildings, places and settlements beside the web example's river (BLD-1), for the web tests
// and for measuring the map with many buildings. Made up here, from a seed: no real data.
//
// The web example's river runs east from about (499985, 600000) to (502413, 600000) in UTM zone 30N.

/** UTM zone 30N easting and northing to WGS84 longitude and latitude (the standard series, to well under
 * a millimetre here). */
export function utm30ToLonLat(x, y) {
  const a = 6378137.0;
  const f = 1 / 298.257223563;
  const k0 = 0.9996;
  const e2 = f * (2 - f);
  const ep2 = e2 / (1 - e2);
  const lon0 = (-3 * Math.PI) / 180;
  const m = y / k0;
  const mu = m / (a * (1 - e2 / 4 - (3 * e2 * e2) / 64 - (5 * e2 ** 3) / 256));
  const e1 = (1 - Math.sqrt(1 - e2)) / (1 + Math.sqrt(1 - e2));
  const phi1 = mu + ((3 * e1) / 2 - (27 * e1 ** 3) / 32) * Math.sin(2 * mu) + ((21 * e1 * e1) / 16 - (55 * e1 ** 4) / 32) * Math.sin(4 * mu) + ((151 * e1 ** 3) / 96) * Math.sin(6 * mu) + ((1097 * e1 ** 4) / 512) * Math.sin(8 * mu);
  const n1 = a / Math.sqrt(1 - e2 * Math.sin(phi1) ** 2);
  const t1 = Math.tan(phi1) ** 2;
  const c1 = ep2 * Math.cos(phi1) ** 2;
  const r1 = (a * (1 - e2)) / (1 - e2 * Math.sin(phi1) ** 2) ** 1.5;
  const d = (x - 500000) / (n1 * k0);
  const lat = phi1 - ((n1 * Math.tan(phi1)) / r1) * ((d * d) / 2 - ((5 + 3 * t1 + 10 * c1 - 4 * c1 * c1 - 9 * ep2) * d ** 4) / 24 + ((61 + 90 * t1 + 298 * c1 + 45 * t1 * t1 - 252 * ep2 - 3 * c1 * c1) * d ** 6) / 720);
  const lon = lon0 + (d - ((1 + 2 * t1 + c1) * d ** 3) / 6 + ((5 - 2 * c1 + 28 * t1 - 3 * c1 * c1 + 8 * ep2 + 24 * t1 * t1) * d ** 5) / 120) / Math.cos(phi1);
  return [(lon * 180) / Math.PI, (lat * 180) / Math.PI];
}

/** A small seeded random: the same file every run. */
function random(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

const square = (x, y, side) => [[x, y], [x + side, y], [x + side, y + side], [x, y + side], [x, y]].map(([u, v]) => utm30ToLonLat(u, v));

/** `n` building footprints within `reach` metres either side of the river's line (y = 600000), each 6 to 20 m
 * a side, with `area_m2` and a `confidence` from 0.6 to 1. The first few sit in a row at known places. */
export function buildings(n, { reach = 1500, x0 = 499990, x1 = 502400, seed = 7 } = {}) {
  const r = random(seed);
  const features = [];
  for (let i = 0; i < n; i++) {
    const side = 6 + 14 * r();
    const x = x0 + (x1 - x0) * r();
    const off = (r() < 0.5 ? -1 : 1) * (60 + (reach - 60) * r());
    const conf = Math.round((0.6 + 0.4 * r()) * 100) / 100;
    features.push({ type: "Feature", properties: { area_m2: Math.round(side * side * 10) / 10, confidence: conf }, geometry: { type: "Polygon", coordinates: [square(x, 600000 + off, side)] } });
  }
  return { type: "FeatureCollection", features };
}

/** Places along the river: communities of several sizes, a bridge and a quay, and one of an unknown kind. */
export function places() {
  const at = (x, y) => utm30ToLonLat(x, y);
  const p = (name, kind, x, y, extra = {}) => ({ type: "Feature", properties: { name, kind, ...extra }, geometry: { type: "Point", coordinates: at(x, y) } });
  return {
    type: "FeatureCollection",
    features: [
      p("Riverbend", "community", 500300, 600400, { area_ha: 45.5, distance_to_river_m: 120, parts: 3 }),
      p("Little Ford", "community", 500330, 600420, { area_ha: 2.1, distance_to_river_m: 140, parts: 1 }),
      p("Eastmoor", "community", 501800, 599600, { area_ha: 12, distance_to_river_m: 300, parts: 2 }),
      p("Old Crossing", "bridge", 501000, 600000, { bearing_deg: 0, length_m: 120, width_m: 8, deck_level_m: 36 }),
      p("North Wharf", "quay", 501500, 600200, { bearing_deg: 90, length_m: 60, width_m: 15, height_m: 2 }),
      p("Pump House", "water tower", 502000, 600500),
    ],
  };
}

/** Settlements: polygons, most with no name. */
export function settlements() {
  return {
    type: "FeatureCollection",
    features: [
      { type: "Feature", properties: { name: "Riverbend", kind: "community", area_ha: 45.5 }, geometry: { type: "Polygon", coordinates: [square(500100, 600150, 400)] } },
      { type: "Feature", properties: { name: "" }, geometry: { type: "Polygon", coordinates: [square(501700, 599300, 250)] } },
    ],
  };
}
