//! Feature inputs held by reference (BLD-1): buildings, points of interest and settlements.
//!
//! Each is a vector file the user owns (GeoJSON, or a GeoPackage converted by the caller), in
//! WGS84 or a WGS84 / UTM CRS. The design never holds the features: it records the file as it
//! records the DEM, by a path from the design's own folder, the SHA-256 of the file, the number of
//! features and the source CRS:
//!
//! ```json
//! "buildings": {"path": "../inputs/buildings.geojson", "sha256": "…", "count": 36012,
//!               "crs": "WGS 84", "min_confidence": 0.7, "min_area_m2": 0, "near_cut_m": 50}
//! ```
//!
//! A footprint file of 36,000 buildings is 8 MB; the design file is already large, and a copy in it
//! would go stale the day the source is updated. So the host attaches the file when it opens the
//! design ([`Session::attach_features`]): only the file whose hash was recorded is accepted, as for
//! the DEM and the cut-rate multiplier. In memory the session keeps one point per feature (a
//! representative point in the project CRS), its area, its confidence, its name and kind and, for
//! points of interest and settlements only, its other properties. Settlements also keep their
//! rings, for the share of each that a pool floods. Building footprints are not kept: the map draws
//! them from the file, and the cut test uses a disc of the building's area (see `buildings.rs`).

use super::crs::{Crs, Utm};
use super::session::{DesignError, Session};
use owd_core::geom::{Polygon, Pt};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// A feature input of a design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureInput {
    Buildings,
    PointsOfInterest,
    Settlements,
}

/// The default confidence below which a building is counted apart, not costed.
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;
/// The default smallest building area counted, m2 (0: every building).
pub const DEFAULT_MIN_AREA_M2: f64 = 0.0;
/// The default distance from the cut within which a building is reported as near it, metres.
pub const DEFAULT_NEAR_CUT_M: f64 = 50.0;

impl FeatureInput {
    pub const ALL: [FeatureInput; 3] = [
        FeatureInput::Buildings,
        FeatureInput::PointsOfInterest,
        FeatureInput::Settlements,
    ];

    /// The key under `inputs` (in the design and in `owd-project.json`).
    pub fn key(self) -> &'static str {
        match self {
            FeatureInput::Buildings => "buildings",
            FeatureInput::PointsOfInterest => "points_of_interest",
            FeatureInput::Settlements => "settlements",
        }
    }

    /// The words the app and the CLI show.
    pub fn label(self) -> &'static str {
        match self {
            FeatureInput::Buildings => "Buildings",
            FeatureInput::PointsOfInterest => "Points of interest",
            FeatureInput::Settlements => "Settlements",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            FeatureInput::Buildings => "buildings file",
            FeatureInput::PointsOfInterest => "points of interest file",
            FeatureInput::Settlements => "settlements file",
        }
    }

    pub fn from_key(key: &str) -> Option<FeatureInput> {
        FeatureInput::ALL.into_iter().find(|f| f.key() == key)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One feature, as the session keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// The feature's `id` (its own, or `properties.id`), else its number in the file from 1.
    pub id: String,
    /// A representative point in the project CRS: the area centroid of a polygon (or, when that
    /// falls outside it, the middle of the widest crossing at its height), the middle of a line.
    pub at: Pt,
    /// The same point in WGS84 longitude and latitude.
    pub lon_lat: [f64; 2],
    /// `properties.area_m2` when given, else the polygon's area in the project CRS; NaN for a point
    /// with none.
    pub area_m2: f64,
    /// `properties.confidence` (0 to 1); NaN when the file gives none.
    pub confidence: f64,
    pub name: String,
    pub kind: String,
    /// Every property, as read (points of interest and settlements; empty for buildings).
    pub props: Map<String, Value>,
    /// The polygon in the project CRS (settlements only; empty otherwise).
    pub polygons: Vec<Polygon>,
}

/// The features of one input, read from the file the design names.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureSet {
    pub input: FeatureInput,
    pub features: Vec<Feature>,
    /// The source CRS's name.
    pub crs: String,
    /// Features with no geometry, which are not counted.
    pub skipped: usize,
}

/// What the stages read of the buildings: which ones count, and the distance of the cut report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildingFilters {
    /// A building with a confidence below this is counted apart and never costed.
    pub min_confidence: f64,
    /// A building smaller than this, m2, is counted apart and never costed.
    pub min_area_m2: f64,
    /// Buildings within this distance of the cut are reported as near it, metres.
    pub near_cut_m: f64,
}

impl Default for BuildingFilters {
    fn default() -> Self {
        BuildingFilters {
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            min_area_m2: DEFAULT_MIN_AREA_M2,
            near_cut_m: DEFAULT_NEAR_CUT_M,
        }
    }
}

impl BuildingFilters {
    /// The building is costed: its confidence (when the file gives one) and its area (when known)
    /// pass the thresholds.
    pub fn counts(&self, f: &Feature) -> bool {
        !self.below_confidence(f) && !self.below_area(f)
    }

    pub fn below_confidence(&self, f: &Feature) -> bool {
        f.confidence.is_finite() && f.confidence < self.min_confidence
    }

    pub fn below_area(&self, f: &Feature) -> bool {
        f.area_m2.is_finite() && f.area_m2 < self.min_area_m2
    }

    fn check(&self) -> Result<(), DesignError> {
        if !(self.min_confidence.is_finite() && (0.0..=1.0).contains(&self.min_confidence)) {
            return Err(DesignError::Input(
                "the building confidence threshold must be a number from 0 to 1".into(),
            ));
        }
        if !(self.min_area_m2.is_finite() && self.min_area_m2 >= 0.0) {
            return Err(DesignError::Input(
                "the smallest building area must be a number of m2 >= 0".into(),
            ));
        }
        if !(self.near_cut_m.is_finite() && self.near_cut_m >= 0.0) {
            return Err(DesignError::Input(
                "the distance a building counts as near the cut must be a number of metres >= 0"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// The features the session holds in memory, by input. Never stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attached([Option<Arc<FeatureSet>>; 3]);

fn bad(input: FeatureInput, what: impl AsRef<str>) -> String {
    format!("the {}: {}", input.noun(), what.as_ref())
}

fn ring_area(ring: &[Pt]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        twice += a[0] * b[1] - b[0] * a[1];
    }
    twice / 2.0
}

/// The area of a polygon (its outer ring less its holes), m2 in a metric CRS.
pub fn polygon_area(p: &Polygon) -> f64 {
    ring_area(&p.exterior).abs() - p.holes.iter().map(|h| ring_area(h).abs()).sum::<f64>()
}

/// Whether `p` is inside the polygon (even-odd, so a hole is outside).
pub fn polygon_contains(poly: &Polygon, p: Pt) -> bool {
    let mut inside = false;
    for ring in std::iter::once(&poly.exterior).chain(poly.holes.iter()) {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (ring[i], ring[j]);
            if (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
            {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

/// A point inside the polygon to label it by: the area centroid of its outer ring, or, when that is
/// outside (a crescent), the middle of the widest crossing of the polygon at the centroid's height.
pub fn representative_point(poly: &Polygon) -> Pt {
    let r = &poly.exterior;
    let n = r.len();
    let (mut cx, mut cy, mut a2) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (r[i], r[(i + 1) % n]);
        let c = a[0] * b[1] - b[0] * a[1];
        a2 += c;
        cx += (a[0] + b[0]) * c;
        cy += (a[1] + b[1]) * c;
    }
    let centroid = if a2.abs() > 1e-12 {
        [cx / (3.0 * a2), cy / (3.0 * a2)]
    } else {
        let k = n.max(1) as f64;
        [
            r.iter().map(|p| p[0]).sum::<f64>() / k,
            r.iter().map(|p| p[1]).sum::<f64>() / k,
        ]
    };
    if n < 3 || polygon_contains(poly, centroid) {
        return centroid;
    }
    // The crossings of every ring with the line y = centroid's y, in pairs.
    let y = centroid[1];
    let mut xs: Vec<f64> = Vec::new();
    for ring in std::iter::once(&poly.exterior).chain(poly.holes.iter()) {
        let m = ring.len();
        for i in 0..m {
            let (a, b) = (ring[i], ring[(i + 1) % m]);
            if (a[1] > y) != (b[1] > y) {
                xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>()
        .0
        .iter()
        .max_by(|p, q| (p[1] - p[0]).total_cmp(&(q[1] - q[0])))
        .map(|p| [(p[0] + p[1]) / 2.0, y])
        .unwrap_or(centroid)
}

fn line_middle(line: &[Pt]) -> Pt {
    let total: f64 = line
        .windows(2)
        .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
        .sum();
    let mut run = 0.0;
    for w in line.windows(2) {
        let d = libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        if run + d >= total / 2.0 && d > 0.0 {
            let t = (total / 2.0 - run) / d;
            return [
                w[0][0] + t * (w[1][0] - w[0][0]),
                w[0][1] + t * (w[1][1] - w[0][1]),
            ];
        }
        run += d;
    }
    line.first().copied().unwrap_or([f64::NAN, f64::NAN])
}

/// A geometry as read, in the source CRS.
enum Shape {
    Point(Pt),
    Line(Vec<Pt>),
    Polygons(Vec<Polygon>),
}

fn position(v: &Value) -> Option<Pt> {
    let a = v.as_array()?;
    let (x, y) = (a.first()?.as_f64()?, a.get(1)?.as_f64()?);
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn positions(v: &Value) -> Option<Vec<Pt>> {
    v.as_array()?.iter().map(position).collect()
}

fn polygon_of(v: &Value) -> Option<Polygon> {
    let rings = v.as_array()?;
    let mut it = rings.iter().map(positions);
    let exterior = it.next()??;
    let holes = it.collect::<Option<Vec<_>>>()?;
    Some(Polygon { exterior, holes })
}

fn shape_of(g: &Value) -> Result<Option<Shape>, String> {
    let kind = g.get("type").and_then(Value::as_str).unwrap_or("");
    let c = &g["coordinates"];
    let wrong = || format!("a {kind} with coordinates that are not numbers");
    Ok(Some(match kind {
        "Point" => Shape::Point(position(c).ok_or_else(wrong)?),
        "MultiPoint" => match positions(c).ok_or_else(wrong)?.first() {
            Some(p) => Shape::Point(*p),
            None => return Ok(None),
        },
        "LineString" => Shape::Line(positions(c).ok_or_else(wrong)?),
        "MultiLineString" => {
            let lines: Vec<Vec<Pt>> = c
                .as_array()
                .ok_or_else(wrong)?
                .iter()
                .map(positions)
                .collect::<Option<_>>()
                .ok_or_else(wrong)?;
            match lines.into_iter().max_by_key(|l| l.len()) {
                Some(l) => Shape::Line(l),
                None => return Ok(None),
            }
        }
        "Polygon" => Shape::Polygons(vec![polygon_of(c).ok_or_else(wrong)?]),
        "MultiPolygon" => {
            let ps: Vec<Polygon> = c
                .as_array()
                .ok_or_else(wrong)?
                .iter()
                .map(polygon_of)
                .collect::<Option<_>>()
                .ok_or_else(wrong)?;
            if ps.is_empty() {
                return Ok(None);
            }
            Shape::Polygons(ps)
        }
        other => return Err(format!("a `{other}` geometry, which is not read")),
    }))
}

/// Read a feature file (GeoJSON text) into the project CRS `project`.
pub fn read_features(input: FeatureInput, text: &str, project: Utm) -> Result<FeatureSet, String> {
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| bad(input, format!("not valid JSON: {e}")))?;
    let crs = match root.get("crs") {
        None | Some(Value::Null) => Crs::Wgs84,
        Some(c) => Crs::parse(
            c.pointer("/properties/name")
                .and_then(Value::as_str)
                .ok_or_else(|| bad(input, "the `crs` member has no `properties.name`"))?,
        )
        .map_err(|e| bad(input, e))?,
    };
    let crs_name = match crs {
        Crs::Wgs84 => "WGS 84".to_string(),
        Crs::Utm(u) => u.name(),
    };
    let feats: Vec<&Value> = match root.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => root
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| bad(input, "a FeatureCollection needs a `features` list"))?
            .iter()
            .collect(),
        Some(_) => vec![&root],
        None => return Err(bad(input, "this is not GeoJSON: it has no `type`")),
    };
    // Every position to the project CRS, through WGS84 when the file is in another.
    let to_lonlat: Box<dyn Fn(Pt) -> Option<Pt>> = match crs {
        Crs::Wgs84 => Box::new(Some),
        Crs::Utm(u) => Box::new(u.inverse_fn().map_err(|e| bad(input, e))?),
    };
    let fwd = project.forward_fn().map_err(|e| bad(input, e))?;
    let to_project = |p: Pt| -> Option<Pt> {
        match crs {
            Crs::Utm(u) if u == project => Some(p),
            _ => fwd(to_lonlat(p)?),
        }
    };
    let inv = project.inverse_fn().map_err(|e| bad(input, e))?;
    let mut features = Vec::with_capacity(feats.len());
    let mut skipped = 0;
    for (i, f) in feats.iter().enumerate() {
        let n = i + 1;
        let (geometry, props) = if f.get("type").and_then(Value::as_str) == Some("Feature") {
            (f.get("geometry"), f.get("properties"))
        } else {
            (Some(*f), None)
        };
        let Some(g) = geometry.filter(|g| !g.is_null()) else {
            skipped += 1;
            continue;
        };
        let shape = shape_of(g)
            .map_err(|e| bad(input, format!("feature {n} has {e}")))?
            .ok_or_else(|| bad(input, format!("feature {n} has an empty geometry")));
        let Ok(shape) = shape else {
            skipped += 1;
            continue;
        };
        let props: Map<String, Value> = props
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let text = |k: &str| -> Result<String, String> {
            match props.get(k) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.trim().to_string()),
                Some(other) => Err(bad(
                    input,
                    format!("feature {n} has `{k}` {other}: it must be text in quotes"),
                )),
            }
        };
        let name = text("name")?;
        let kind = text("kind")?;
        let number = |k: &str| -> Result<f64, String> {
            match props.get(k) {
                None | Some(Value::Null) => Ok(f64::NAN),
                Some(v) => v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| {
                    bad(
                        input,
                        format!("feature {n} has `{k}` {v}: it must be a number"),
                    )
                }),
            }
        };
        let confidence = number("confidence")?;
        if confidence.is_finite() && !(0.0..=1.0).contains(&confidence) {
            return Err(bad(
                input,
                format!("feature {n} has a confidence of {confidence}: it must be from 0 to 1"),
            ));
        }
        let given_area = number("area_m2")?;
        let id = match f.get("id").or_else(|| props.get("id")) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(x)) => x.to_string(),
            _ => n.to_string(),
        };
        let project_pts = |pts: &[Pt]| -> Result<Vec<Pt>, String> {
            pts.iter()
                .map(|&p| to_project(p))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    bad(
                        input,
                        format!(
                            "feature {n} has a position that cannot be converted to the project CRS"
                        ),
                    )
                })
        };
        let (at, area, polygons) = match shape {
            Shape::Point(p) => (project_pts(&[p])?[0], given_area, Vec::new()),
            Shape::Line(l) => {
                if input == FeatureInput::Buildings {
                    return Err(bad(
                        input,
                        format!("feature {n} is a line: a building must be a polygon or a point"),
                    ));
                }
                (line_middle(&project_pts(&l)?), given_area, Vec::new())
            }
            Shape::Polygons(ps) => {
                let mut polys = Vec::with_capacity(ps.len());
                for p in &ps {
                    polys.push(Polygon {
                        exterior: project_pts(&p.exterior)?,
                        holes: p
                            .holes
                            .iter()
                            .map(|h| project_pts(h))
                            .collect::<Result<_, _>>()?,
                    });
                }
                let biggest = polys
                    .iter()
                    .max_by(|a, b| polygon_area(a).total_cmp(&polygon_area(b)))
                    .cloned();
                let Some(biggest) = biggest else {
                    skipped += 1;
                    continue;
                };
                let area = if given_area.is_finite() {
                    given_area
                } else {
                    polys.iter().map(polygon_area).sum()
                };
                (representative_point(&biggest), area, polys)
            }
        };
        if input == FeatureInput::Settlements && polygons.is_empty() {
            return Err(bad(
                input,
                format!("feature {n} is not a polygon: a settlement must be a polygon"),
            ));
        }
        let lon_lat =
            inv(at).ok_or_else(|| bad(input, format!("feature {n} is off the project CRS")))?;
        features.push(Feature {
            id,
            at,
            lon_lat,
            area_m2: area,
            confidence,
            name,
            kind,
            props: if input == FeatureInput::Buildings {
                Map::new()
            } else {
                props
            },
            polygons: if input == FeatureInput::Settlements {
                polygons
            } else {
                Vec::new()
            },
        });
    }
    if features.is_empty() {
        return Err(bad(input, "it holds no feature with a geometry"));
    }
    Ok(FeatureSet {
        input,
        features,
        crs: crs_name,
        skipped,
    })
}

/// The file as the map draws it: every feature in WGS84, with its number (`fid`, from 0, the
/// session's order), its id, and, for buildings, its area and confidence; points of interest and
/// settlements keep their properties. Geometries are as in the file, converted to WGS84.
pub fn map_geojson(input: FeatureInput, text: &str) -> Result<Value, String> {
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| bad(input, format!("not valid JSON: {e}")))?;
    let crs = match root.pointer("/crs/properties/name").and_then(Value::as_str) {
        None => Crs::Wgs84,
        Some(n) => Crs::parse(n).map_err(|e| bad(input, e))?,
    };
    let inv: Option<Box<dyn Fn(Pt) -> Option<Pt>>> = match crs {
        Crs::Wgs84 => None,
        Crs::Utm(u) => Some(Box::new(u.inverse_fn().map_err(|e| bad(input, e))?)),
    };
    fn walk(v: &mut Value, f: &dyn Fn(Pt) -> Option<Pt>) {
        if let Some(p) = position(v)
            && v.as_array().is_some_and(|a| a.iter().all(Value::is_number))
        {
            if let Some(q) = f(p) {
                *v = json!([q[0], q[1]]);
            }
            return;
        }
        if let Some(a) = v.as_array_mut() {
            for x in a {
                walk(x, f);
            }
        }
    }
    let feats: Vec<Value> = match root.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => root["features"].as_array().cloned().unwrap_or_default(),
        _ => vec![root.clone()],
    };
    let mut out = Vec::with_capacity(feats.len());
    for (i, f) in feats.into_iter().enumerate() {
        let (mut geometry, props) = if f.get("type").and_then(Value::as_str) == Some("Feature") {
            (f["geometry"].clone(), f["properties"].clone())
        } else {
            (f.clone(), Value::Null)
        };
        // The features the session keeps, and no other, so `fid` is the session's number.
        if geometry.is_null() || !matches!(shape_of(&geometry), Ok(Some(_))) {
            continue;
        }
        if let Some(inv) = &inv {
            walk(&mut geometry["coordinates"], inv.as_ref());
        }
        let id = match f.get("id").or_else(|| props.get("id")) {
            Some(Value::String(s)) => json!(s),
            Some(Value::Number(x)) => json!(x.to_string()),
            _ => json!((i + 1).to_string()),
        };
        let mut p = match input {
            FeatureInput::Buildings => json!({
                "area_m2": props.get("area_m2").cloned().unwrap_or(Value::Null),
                "confidence": props.get("confidence").cloned().unwrap_or(Value::Null),
            }),
            _ => match props {
                Value::Object(m) => Value::Object(m),
                _ => json!({}),
            },
        };
        p["fid"] = json!(out.len());
        p["id"] = id;
        out.push(json!({"type": "Feature", "geometry": geometry, "properties": p}));
    }
    Ok(json!({"type": "FeatureCollection", "features": out}))
}

/// The GeoJSON text of a feature file: GeoJSON as it is, a GeoPackage's first layer converted.
pub fn file_text(input: FeatureInput, bytes: &[u8]) -> Result<String, DesignError> {
    if super::geopackage::is_geopackage(bytes) {
        return super::geopackage::to_geojson(bytes, None)
            .map_err(|e| DesignError::Input(bad(input, e)));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| DesignError::Input(bad(input, "not GeoJSON text or a GeoPackage")))
}

impl Session {
    fn feature_utm(&self) -> Result<Utm, DesignError> {
        let epsg = self.doc["project"]["crs"]["epsg"].as_u64().ok_or_else(|| {
            DesignError::Input(
                "load the river first: buildings, places and settlements are placed in the \
                 project CRS, which the river's import sets"
                    .into(),
            )
        })?;
        match Crs::from_epsg(epsg as u32).map_err(DesignError::Input)? {
            Crs::Utm(u) => Ok(u),
            Crs::Wgs84 => Err(DesignError::Input(
                "the project CRS is not a UTM zone".into(),
            )),
        }
    }

    /// The record of a feature input in the document, or `None`.
    pub fn feature_record(&self, input: FeatureInput) -> Option<&Value> {
        self.doc["inputs"]
            .get(input.key())
            .filter(|v| v.is_object())
    }

    /// The path of the file the document names for `input`, from the document's folder.
    pub fn features_path(&self, input: FeatureInput) -> Option<&str> {
        self.feature_record(input)?["path"].as_str()
    }

    /// Say the path of the file again (the design moved; the hash stays, so nothing goes stale).
    pub fn set_features_path(&mut self, input: FeatureInput, path: &str) {
        if self.feature_record(input).is_some() {
            self.doc["inputs"][input.key()]["path"] = json!(path);
        }
    }

    /// The features in memory, when the file is attached.
    pub fn features(&self, input: FeatureInput) -> Option<&Arc<FeatureSet>> {
        self.features.0[input.index()].as_ref()
    }

    /// Record the file `path` (from the document's folder) as `input`, and keep its features. The
    /// file is read and checked; a refusal changes nothing. The stages that read it go stale.
    pub fn set_features(
        &mut self,
        input: FeatureInput,
        path: &str,
        bytes: &[u8],
    ) -> Result<&FeatureSet, DesignError> {
        let utm = self.feature_utm()?;
        let text = file_text(input, bytes)?;
        let set = read_features(input, &text, utm).map_err(DesignError::Input)?;
        let filters = self.building_filters();
        let mut rec = json!({
            "path": path,
            "sha256": super::hash::sha256_hex(bytes),
            "count": set.features.len(),
            "crs": set.crs,
        });
        if input == FeatureInput::Buildings {
            rec["min_confidence"] = json!(filters.min_confidence);
            rec["min_area_m2"] = json!(filters.min_area_m2);
            rec["near_cut_m"] = json!(filters.near_cut_m);
        }
        let inputs = self.doc["inputs"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no inputs".into()))?;
        inputs.insert(input.key().into(), rec);
        self.features.0[input.index()] = Some(Arc::new(set));
        Ok(self.features(input).expect("just set"))
    }

    /// Remove `input` from the document and from memory.
    pub fn clear_features(&mut self, input: FeatureInput) {
        if let Some(i) = self.doc["inputs"].as_object_mut() {
            i.shift_remove(input.key());
        }
        self.features.0[input.index()] = None;
    }

    /// Attach the file the document names for `input`, from the bytes the host found at
    /// [`Session::features_path`]: only the file whose hash was recorded is accepted.
    pub fn attach_features(
        &mut self,
        input: FeatureInput,
        bytes: &[u8],
    ) -> Result<(), DesignError> {
        let Some(rec) = self.feature_record(input) else {
            return Err(DesignError::Input(format!(
                "the design names no {}",
                input.noun()
            )));
        };
        let path = rec["path"].as_str().unwrap_or("?").to_string();
        if rec["sha256"].as_str() != Some(super::hash::sha256_hex(bytes).as_str()) {
            return Err(DesignError::Input(format!(
                "the {} `{path}` has changed since its hash was recorded: load it again to use \
                 the new file",
                input.noun()
            )));
        }
        let utm = self.feature_utm()?;
        let text = file_text(input, bytes)?;
        let set = read_features(input, &text, utm).map_err(DesignError::Input)?;
        self.features.0[input.index()] = Some(Arc::new(set));
        Ok(())
    }

    /// A warning when the file the document names for `input` is missing (`bytes` is `None`) or
    /// is not the file whose hash was recorded, as for the DEM.
    pub fn features_warning(&self, input: FeatureInput, bytes: Option<&[u8]>) -> Option<String> {
        let rec = self.feature_record(input)?;
        let path = rec["path"].as_str().unwrap_or("?");
        match bytes {
            None => Some(format!(
                "the {} `{path}` is missing: load it again, or the map and the reports go without it",
                input.noun()
            )),
            Some(b) if rec["sha256"].as_str() != Some(super::hash::sha256_hex(b).as_str()) => {
                Some(format!(
                    "the {} `{path}` has changed since its hash was recorded: load it again to use \
                     the new file",
                    input.noun()
                ))
            }
            _ => None,
        }
    }

    /// The building filters the document records (the defaults when it records no buildings).
    pub fn building_filters(&self) -> BuildingFilters {
        let d = BuildingFilters::default();
        let Some(r) = self.feature_record(FeatureInput::Buildings) else {
            return d;
        };
        BuildingFilters {
            min_confidence: r["min_confidence"].as_f64().unwrap_or(d.min_confidence),
            min_area_m2: r["min_area_m2"].as_f64().unwrap_or(d.min_area_m2),
            near_cut_m: r["near_cut_m"].as_f64().unwrap_or(d.near_cut_m),
        }
    }

    /// Change the building filters. Refused with no buildings recorded, or with a bad value.
    pub fn set_building_filters(&mut self, f: BuildingFilters) -> Result<(), DesignError> {
        f.check()?;
        if self.feature_record(FeatureInput::Buildings).is_none() {
            return Err(DesignError::Input(
                "the design names no buildings file: load one first".into(),
            ));
        }
        let r = &mut self.doc["inputs"]["buildings"];
        r["min_confidence"] = json!(f.min_confidence);
        r["min_area_m2"] = json!(f.min_area_m2);
        r["near_cut_m"] = json!(f.near_cut_m);
        Ok(())
    }

    /// The buildings' record without its path, when the optimise stage's settings read the
    /// buildings: a building cost, or the flood terms (cost, limit or report) with buildings named.
    pub(super) fn buildings_read_by_optimise(&self) -> Option<Value> {
        let rec = self.feature_record(FeatureInput::Buildings)?;
        let st = &self.doc["stages"]["optimise"]["settings"];
        let num_on = |k: &str| st[k].as_f64().is_some_and(|v| v != 0.0);
        let reads = num_on("building_cost")
            || num_on("flood_cost_per_ha")
            || st["max_above_bank_m"].is_number()
            || st["flood_report"] == json!(true);
        if !reads {
            return None;
        }
        let mut r = rec.clone();
        if let Some(o) = r.as_object_mut() {
            o.remove("path");
        }
        Some(r)
    }

    /// What the app shows of each feature input: `{key, label, path, count, attached}`; a key
    /// with no file named is left out.
    pub fn feature_inputs(&self) -> Value {
        let mut out = Map::new();
        for input in FeatureInput::ALL {
            let Some(r) = self.feature_record(input) else {
                continue;
            };
            let mut v = r.clone();
            v["label"] = json!(input.label());
            v["attached"] = json!(self.features(input).is_some());
            if let Some(set) = self.features(input)
                && input == FeatureInput::Buildings
            {
                let f = self.building_filters();
                let below_conf = set
                    .features
                    .iter()
                    .filter(|b| f.below_confidence(b))
                    .count();
                let below_area = set
                    .features
                    .iter()
                    .filter(|b| !f.below_confidence(b) && f.below_area(b))
                    .count();
                v["counted"] = json!(set.features.len() - below_conf - below_area);
                // The least depth that counts a building as flooded: the optimise setting.
                v["min_depth_m"] = json!(
                    self.doc["stages"]["optimise"]["settings"]["building_min_depth_m"]
                        .as_f64()
                        .unwrap_or(super::optimise::BUILDING_MIN_DEPTH_M)
                );
                v["below_confidence"] = json!(below_conf);
                v["below_area"] = json!(below_area);
            }
            out.insert(input.key().into(), v);
        }
        Value::Object(out)
    }
}

impl Attached {
    pub fn get(&self, input: FeatureInput) -> Option<&Arc<FeatureSet>> {
        self.0[input.index()].as_ref()
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> Polygon {
        Polygon {
            exterior: vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]],
            holes: vec![],
        }
    }

    #[test]
    fn a_square_has_its_centre_and_area_and_a_crescent_a_point_inside() {
        let p = sq(0.0, 0.0, 10.0);
        assert_eq!(polygon_area(&p), 100.0);
        assert_eq!(representative_point(&p), [5.0, 5.0]);
        // A U shape: its centroid falls in the gap.
        let u = Polygon {
            exterior: vec![
                [0.0, 0.0],
                [30.0, 0.0],
                [30.0, 30.0],
                [20.0, 30.0],
                [20.0, 5.0],
                [10.0, 5.0],
                [10.0, 30.0],
                [0.0, 30.0],
            ],
            holes: vec![],
        };
        let r = representative_point(&u);
        assert!(polygon_contains(&u, r), "{r:?}");
    }
}
