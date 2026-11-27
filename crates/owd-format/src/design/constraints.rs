//! Constraints (spec 0017, ticket 13): no-go areas and the cut-rate multiplier. The document
//! stores them as `inputs.constraints`.
//!
//! - A **no-go area** is a polygon (with holes) in the project CRS, with a name and a kind (free
//!   text: farmland, heritage, settlement). The re-route treats it as impassable; every design line
//!   reports the no-go areas it touches ([`constraints_touched`]).
//! - The **cut-rate multiplier** is a raster, referenced like the DEM: path, SHA-256, CRS and nodata.
//!   Its cell value multiplies the cut rate in that cell; no data means 1. The re-route weighs its
//!   cut cost by it, and the volumes stage stores the cut weighted by it beside the plain cut, which
//!   the cost stage prices. A cell at or above [`HARD_EXCLUSION`] is impassable to the re-route.
//!
//! **The channel footprint** is the strip of the channel's straight bottom width centred on the
//! line (half of `bottom_width_straight_m` each side). It is the channel's bed, not its cut slopes:
//! how far the slopes reach depends on the ground, which only the volumes stage reads. Bend widening
//! is not in it either ([`FOOTPRINT_NOTE`]).
//!
//! A design line of any method whose footprint enters a no-go area or a hard-exclusion cell fails
//! the check `no_go_clear`; the re-route refuses such a line.
//!
//! [`Session::set_constraints`] is the one function the CLI and the web app set them through.

use super::session::{DesignError, Session};
use crate::geotiff::Dem;
use crate::types::{Chainage, Polyline, StageError, TerrainSampler};
use owd_core::geom::overlay::contains_xy;
use owd_core::geom::{Polygon, Pt, linear};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// The input's key in `inputs`.
pub const INPUT: &str = "constraints";

/// The name of the input's schema contract.
pub const CONTRACT: &str = "inputs.constraints";

/// The multiplier at or above which a cell is impassable to the re-route (spec 0017).
pub const HARD_EXCLUSION: f64 = owd_core::alignment::route::HARD_EXCLUSION;

/// The step along the line, metres, at which the footprint is tested against a no-go area. OWD's
/// own choice, as [`super::design_line::CUT_OFF_STEP_M`] is for cut-offs.
pub const TOUCH_STEP_M: f64 = 5.0;

/// The step across the footprint, metres. OWD's own choice.
pub const TOUCH_ACROSS_M: f64 = 1.0;

/// The `kind` of the ground a line touches where the multiplier is at or above [`HARD_EXCLUSION`].
pub const HARD_EXCLUSION_KIND: &str = "hard_exclusion";

/// What the footprint tested against the constraints covers, as every `no_go_clear` check says.
pub const FOOTPRINT_NOTE: &str = "the footprint tested is the channel's straight bottom width only: \
                                  the cut slopes and bend widening are not tested";

/// The finest step, metres, at which the footprint is walked over the multiplier raster: a bound on
/// the work for a very fine raster. OWD's own choice.
pub const MIN_WALK_STEP_M: f64 = 0.25;

/// The cut-rate multiplier as a stage reads it: the value at points in the project CRS (NaN where
/// there is no data), and the raster's cell size in metres, so a line can be walked over it cell by
/// cell.
#[derive(Clone, Copy)]
pub struct Multiplier<'a> {
    /// The multiplier at points, `at(xs, ys)`.
    pub at: TerrainSampler<'a>,
    /// The raster's cell size, metres (the shorter side).
    pub cell_m: f64,
}

/// A polygon in the project CRS: an outer ring and holes. A ring is closed by implication.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Area {
    /// The outer ring.
    pub shell: Polyline,
    /// Holes: ground inside the shell that is not part of the area.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Polyline>,
}

impl Area {
    /// The polygon as the core's `Polygon`, holes kept.
    pub fn to_polygon(&self) -> Polygon {
        Polygon {
            exterior: self.shell.0.clone(),
            holes: self.holes.iter().map(|h| h.0.clone()).collect(),
        }
    }
}

/// One no-go area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoGoArea {
    /// The name, unique among the run's no-go areas.
    pub name: String,
    /// What it is, in the engineer's words (free text).
    pub kind: String,
    /// The area, in the project CRS.
    pub polygon: Area,
}

/// The cut-rate multiplier raster, by reference, as the DEM is: it is never embedded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MultiplierRaster {
    /// Path relative to the document.
    pub path: String,
    /// SHA-256 of the file, in hex.
    pub sha256: String,
    /// The raster's CRS: `EPSG:nnnn`, or `unknown`.
    pub crs: String,
    /// The value that marks no data, if the file has one. No data multiplies by 1.
    pub nodata: Option<f64>,
}

/// The run's constraints: `inputs.constraints`. Absent from a document that has none.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    /// The no-go areas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_go: Vec<NoGoArea>,
    /// The cut-rate multiplier raster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_rate_multiplier: Option<MultiplierRaster>,
}

/// One no-go area, or one stretch of hard-exclusion ground, a design line's footprint enters: where
/// along the line, and how much of the footprint is in it. A line that enters one area twice touches
/// it twice. The footprint is the channel's straight bottom width only, `footprint_width_m` wide,
/// centred on the line: the cut slopes and bend widening are not tested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConstraintTouched {
    /// The no-go area's name; for hard-exclusion ground, the multiplier raster's path.
    pub name: String,
    /// The no-go area's kind; `hard_exclusion` for multiplier cells at or above 1e6.
    pub kind: String,
    /// Design-line chainage where the footprint enters it.
    pub from: Chainage,
    /// Design-line chainage where the footprint leaves it.
    pub to: Chainage,
    /// The area of the footprint inside it, m². A no-go area is sampled every [`TOUCH_STEP_M`]
    /// along the line and [`TOUCH_ACROSS_M`] across it; hard-exclusion ground every half cell of
    /// the raster both ways.
    pub area_m2: f64,
    /// The width of the footprint tested, metres: the channel's straight bottom width only. The cut
    /// slopes and bend widening are not tested.
    pub footprint_width_m: f64,
}

fn refuse(code: &str, message: impl AsRef<str>) -> StageError {
    StageError::new(INPUT, code, message.as_ref())
}

fn input_error(e: StageError) -> DesignError {
    DesignError::Input(format!("{}: {}", e.code, e.message))
}

fn ring_area(ring: &[Pt]) -> f64 {
    let n = ring.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    (twice / 2.0).abs()
}

fn check_ring(name: &str, what: &str, ring: &Polyline) -> Result<(), StageError> {
    let bad = |why: &str| {
        refuse(
            "CONSTRAINTS_POLYGON_INVALID",
            format!("the no-go area `{name}`: its {what} {why}"),
        )
    };
    if ring.0.iter().flatten().any(|v| !v.is_finite()) {
        return Err(bad("has a point that is not two finite numbers"));
    }
    let mut distinct: Vec<Pt> = Vec::new();
    for p in &ring.0 {
        if !distinct.contains(p) {
            distinct.push(*p);
        }
    }
    if distinct.len() < 3 {
        return Err(bad("needs at least three distinct points"));
    }
    if ring_area(&ring.0) <= 0.0 {
        return Err(bad("has no area"));
    }
    Ok(())
}

impl Constraints {
    /// True when there is nothing in it: no no-go area and no multiplier.
    pub fn is_empty(&self) -> bool {
        self.no_go.is_empty() && self.cut_rate_multiplier.is_none()
    }

    /// Check what can be checked without the raster file: each area has a name (`CONSTRAINTS_NAME_
    /// MISSING`) unique among them (`CONSTRAINTS_NAME_DUPLICATE`), a kind (`CONSTRAINTS_KIND_
    /// MISSING`), and rings of three or more distinct finite points with an area
    /// (`CONSTRAINTS_POLYGON_INVALID`).
    pub fn validate(&self) -> Result<(), StageError> {
        for (i, a) in self.no_go.iter().enumerate() {
            let name = a.name.trim();
            if name.is_empty() {
                return Err(refuse(
                    "CONSTRAINTS_NAME_MISSING",
                    format!("no-go area {} has no name: give each one a name", i + 1),
                ));
            }
            if a.kind.trim().is_empty() {
                return Err(refuse(
                    "CONSTRAINTS_KIND_MISSING",
                    format!("the no-go area `{name}` has no kind: say what it is"),
                ));
            }
            if self.no_go[..i].iter().any(|b| b.name.trim() == name) {
                return Err(refuse(
                    "CONSTRAINTS_NAME_DUPLICATE",
                    format!("two no-go areas are named `{name}`: each name is used once"),
                ));
            }
            check_ring(name, "outer ring", &a.polygon.shell)?;
            for h in &a.polygon.holes {
                check_ring(name, "hole", h)?;
            }
        }
        Ok(())
    }
}

/// Read and check constraints from JSON. A key the input does not know is refused
/// (`CONSTRAINTS_INVALID`), so a setting is never lost in silence.
pub fn read_constraints(v: &Value) -> Result<Constraints, StageError> {
    let c: Constraints = serde_json::from_value(v.clone())
        .map_err(|e| refuse("CONSTRAINTS_INVALID", format!("not constraints: {e}")))?;
    c.validate()?;
    Ok(c)
}

/// No-go areas from a GeoJSON `FeatureCollection` of `Polygon` features in the project CRS, each with
/// the properties `name` and `kind`. Anything else is refused by code; nothing is skipped.
pub fn no_go_from_geojson(v: &Value) -> Result<Vec<NoGoArea>, StageError> {
    let features = match (v["type"].as_str(), v["features"].as_array()) {
        (Some("FeatureCollection"), Some(f)) => f,
        _ => {
            return Err(refuse(
                "CONSTRAINTS_INVALID",
                "no-go areas come as a GeoJSON FeatureCollection of polygons",
            ));
        }
    };
    let mut out = Vec::with_capacity(features.len());
    for (i, f) in features.iter().enumerate() {
        let text = |k: &str| f["properties"][k].as_str().unwrap_or_default().to_string();
        let name = text("name");
        if name.trim().is_empty() {
            return Err(refuse(
                "CONSTRAINTS_NAME_MISSING",
                format!(
                    "feature {} has no `name` property: give each no-go area a name",
                    i + 1
                ),
            ));
        }
        let g = &f["geometry"];
        let bad = || {
            refuse(
                "CONSTRAINTS_POLYGON_INVALID",
                format!("the no-go area `{name}` is not a GeoJSON Polygon of [x, y] positions"),
            )
        };
        if g["type"] != "Polygon" {
            return Err(bad());
        }
        let rings: Vec<Polyline> = g["coordinates"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|r| {
                r.as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(|p| match p.as_array().map(|a| a.as_slice()) {
                        Some([x, y, ..]) => {
                            Ok([x.as_f64().ok_or_else(bad)?, y.as_f64().ok_or_else(bad)?])
                        }
                        _ => Err(bad()),
                    })
                    .collect::<Result<Vec<Pt>, StageError>>()
                    .map(Polyline)
            })
            .collect::<Result<_, _>>()?;
        let (shell, holes) = rings.split_first().ok_or_else(bad)?;
        out.push(NoGoArea {
            name,
            kind: text("kind"),
            polygon: Area {
                shell: shell.clone(),
                holes: holes.to_vec(),
            },
        });
    }
    let c = Constraints {
        no_go: out,
        cut_rate_multiplier: None,
    };
    c.validate()?;
    Ok(c.no_go)
}

/// The footprint of a line, cut in cells along it: each cell's sample points (its centre, moved
/// across the line by each offset), the cell length `ds`, the step across `dt` and the line's length.
struct Footprint {
    cells: Vec<Vec<Pt>>,
    ds: f64,
    dt: f64,
    total: f64,
    width: f64,
}

impl Footprint {
    /// The strip of `half_width_m` each side of `line`, in cells about `along` long, sampled about
    /// `across` apart. `None` for a line with no length.
    fn new(line: &[Pt], half_width_m: f64, along: f64, across: f64) -> Option<Footprint> {
        if line.len() < 2 {
            return None;
        }
        let total = linear::length(line);
        if total.is_nan() || total <= 0.0 {
            return None;
        }
        let n = (total / along).ceil().max(1.0) as usize;
        let ds = total / n as f64;
        let half = if half_width_m.is_finite() {
            half_width_m.max(0.0)
        } else {
            0.0
        };
        let m = (half / across).ceil() as usize;
        let dt = if m > 0 { half / m as f64 } else { 0.0 };
        let at = |c: f64| linear::interpolate(line, c.clamp(0.0, total)).unwrap_or([f64::NAN; 2]);
        let cells = (0..n)
            .map(|i| {
                let c = (i as f64 + 0.5) * ds;
                let (p, a, b) = (at(c), at(c - ds / 2.0), at(c + ds / 2.0));
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = libm::hypot(dx, dy);
                let (nx, ny) = if len > 0.0 {
                    (-dy / len, dx / len)
                } else {
                    (0.0, 0.0)
                };
                if m == 0 {
                    return vec![p];
                }
                (0..2 * m)
                    .map(|j| {
                        let t = (j as f64 - m as f64 + 0.5) * dt;
                        [p[0] + t * nx, p[1] + t * ny]
                    })
                    .collect()
            })
            .collect();
        Some(Footprint {
            cells,
            ds,
            dt,
            total,
            width: 2.0 * half,
        })
    }

    /// Each run of cells with a sample in the ground, `inside(cell, samples) -> how many`: its
    /// chainage from and to and the area inside.
    fn runs(
        &self,
        name: &str,
        kind: &str,
        inside: impl Fn(usize, &[Pt]) -> usize,
    ) -> Vec<ConstraintTouched> {
        let mut out = Vec::new();
        let mut run: Option<(usize, usize, usize)> = None; // first cell, last cell, samples in
        let close = |run: (usize, usize, usize), out: &mut Vec<ConstraintTouched>| {
            out.push(ConstraintTouched {
                name: name.to_string(),
                kind: kind.to_string(),
                from: Chainage(run.0 as f64 * self.ds),
                to: Chainage(((run.1 + 1) as f64 * self.ds).min(self.total)),
                area_m2: run.2 as f64 * self.ds * self.dt,
                footprint_width_m: self.width,
            });
        };
        for (i, pts) in self.cells.iter().enumerate() {
            let k = inside(i, pts);
            match (k > 0, run) {
                (true, None) => run = Some((i, i, k)),
                (true, Some((f, _, s))) => run = Some((f, i, s + k)),
                (false, Some(r)) => {
                    close(r, &mut out);
                    run = None;
                }
                (false, None) => {}
            }
        }
        if let Some(r) = run {
            close(r, &mut out);
        }
        out
    }
}

fn by_chainage(out: &mut [ConstraintTouched]) {
    out.sort_by(|x, y| {
        x.from
            .0
            .total_cmp(&y.from.0)
            .then_with(|| x.name.cmp(&y.name))
    });
}

/// The no-go areas a line's footprint enters: for each area, each run of the line where the strip of
/// `half_width_m` each side of it has a sample in the area, with its chainage from and to and the
/// area of the strip inside. The strip is sampled in cells [`TOUCH_STEP_M`] long and up to
/// [`TOUCH_ACROSS_M`] wide, each counted by its centre. In order of chainage, then of name.
pub fn constraints_touched(
    line: &[Pt],
    half_width_m: f64,
    no_go: &[NoGoArea],
) -> Vec<ConstraintTouched> {
    if no_go.is_empty() {
        return Vec::new();
    }
    let Some(f) = Footprint::new(line, half_width_m, TOUCH_STEP_M, TOUCH_ACROSS_M) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for a in no_go {
        let poly = [a.polygon.to_polygon()];
        out.extend(f.runs(&a.name, &a.kind, |_, pts| {
            pts.iter().filter(|p| contains_xy(&poly, **p)).count()
        }));
    }
    by_chainage(&mut out);
    out
}

/// The hard-exclusion ground a line's footprint enters: each run of the line where the strip of
/// `half_width_m` each side of it has a point whose multiplier is at or above [`HARD_EXCLUSION`].
/// The strip is walked over the raster at half its cell size both ways (never finer than
/// [`MIN_WALK_STEP_M`]), so a strip of hard exclusion one cell wide cannot fall between two samples,
/// whatever the re-route's station step. Each run is named `name` (the raster's path), of kind
/// [`HARD_EXCLUSION_KIND`].
pub fn hard_exclusion_touched(
    line: &[Pt],
    half_width_m: f64,
    multiplier: &Multiplier,
    name: &str,
) -> Vec<ConstraintTouched> {
    let step = if multiplier.cell_m.is_finite() && multiplier.cell_m > 0.0 {
        (multiplier.cell_m / 2.0).max(MIN_WALK_STEP_M)
    } else {
        TOUCH_ACROSS_M
    };
    let Some(f) = Footprint::new(line, half_width_m, step, step) else {
        return Vec::new();
    };
    let (xs, ys): (Vec<f64>, Vec<f64>) = f.cells.iter().flatten().map(|p| (p[0], p[1])).unzip();
    let values = (multiplier.at)(&xs, &ys);
    let mut first = Vec::with_capacity(f.cells.len());
    let mut k = 0;
    for c in &f.cells {
        first.push(k);
        k += c.len();
    }
    let mut out = f.runs(name, HARD_EXCLUSION_KIND, |i, pts| {
        (first[i]..first[i] + pts.len())
            .filter(|&j| values.get(j).is_some_and(|&v| v >= HARD_EXCLUSION))
            .count()
    });
    by_chainage(&mut out);
    out
}

/// The multiplier raster's cell size in metres in the project CRS, the shorter side: for a raster in
/// WGS84, measured at its centre once projected.
pub fn multiplier_cell_m(raster: &Dem, project_epsg: Option<u32>) -> Result<f64, StageError> {
    let [a, _, c, _, e, f] = raster.geotransform;
    let cell = match raster_zone(raster, project_epsg)? {
        None => a.abs().min(e.abs()),
        Some(u) => {
            let lon = c + a * raster.width as f64 / 2.0;
            let lat = f + e * raster.height as f64 / 2.0;
            let p = |lon: f64, lat: f64| {
                u.forward(lon, lat)
                    .map_err(|m| refuse("CONSTRAINTS_RASTER_CRS", m))
            };
            let (o, x, y) = (p(lon, lat)?, p(lon + a, lat)?, p(lon, lat + e)?);
            libm::hypot(x[0] - o[0], x[1] - o[1]).min(libm::hypot(y[0] - o[0], y[1] - o[1]))
        }
    };
    Ok(cell)
}

/// The multiplier at a point: the value of the cell that holds it, NaN (no data, so 1) outside the
/// raster or on a nodata cell. A cell's value is not blended with its neighbours': it multiplies
/// the cut rate in that cell.
pub fn cell_value_at(raster: &Dem, x: f64, y: f64) -> f64 {
    let [a, _, c, _, e, f] = raster.geotransform;
    let col = ((x - c) / a).floor();
    let row = ((y - f) / e).floor();
    if !(col >= 0.0 && row >= 0.0 && col < raster.width as f64 && row < raster.height as f64) {
        return f64::NAN;
    }
    raster.value(row as usize, col as usize)
}

/// How the multiplier raster's CRS relates to the project's: `None` when it is the project's own,
/// `Some(zone)` when it is WGS84 and the project is that UTM zone. Anything else is refused
/// (`CONSTRAINTS_RASTER_CRS`), as for the DEM.
fn raster_zone(
    raster: &Dem,
    project_epsg: Option<u32>,
) -> Result<Option<super::crs::Utm>, StageError> {
    use super::crs::Crs;
    let project = project_epsg.and_then(|c| Crs::from_epsg(c).ok());
    match (raster.epsg, project) {
        (Some(4326), Some(Crs::Utm(u))) => Ok(Some(u)),
        (e, Some(p)) if e == Some(p.epsg()) => Ok(None),
        (e, Some(p)) => Err(refuse(
            "CONSTRAINTS_RASTER_CRS",
            format!(
                "the multiplier raster's CRS ({}) is not WGS84 and not the channel design's \
                 EPSG:{}. Convert it to one of them.",
                e.map_or("unknown".to_string(), |e| format!("EPSG:{e}")),
                p.epsg()
            ),
        )),
        (_, None) => Err(refuse(
            "CONSTRAINTS_RASTER_CRS",
            "the channel design has no project CRS yet: import the river first",
        )),
    }
}

/// The multiplier as a sampler for points in the project CRS, the shape of a terrain sampler
/// (`weight(xs, ys)`): the cell value at each point, NaN where there is no data.
pub fn multiplier_sampler(
    raster: &Dem,
    project_epsg: Option<u32>,
) -> Result<impl Fn(&[f64], &[f64]) -> Vec<f64> + '_, StageError> {
    let zone = raster_zone(raster, project_epsg)?;
    Ok(move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let p = match &zone {
                    Some(u) => u.inverse(x, y).unwrap_or([f64::NAN; 2]),
                    None => [x, y],
                };
                cell_value_at(raster, p[0], p[1])
            })
            .collect()
    })
}

/// Read a multiplier raster and check it: a GeoTIFF OWD reads (`CONSTRAINTS_RASTER_UNREADABLE`), in
/// WGS84 or the project's CRS (`CONSTRAINTS_RASTER_CRS`), every cell a finite number at or above
/// zero, or no data (`CONSTRAINTS_RASTER_VALUE`).
pub fn read_multiplier(bytes: &[u8], project_epsg: Option<u32>) -> Result<Dem, StageError> {
    let raster = crate::geotiff::read_bytes(bytes).map_err(|e| {
        refuse(
            "CONSTRAINTS_RASTER_UNREADABLE",
            format!("the multiplier raster does not read: {e}"),
        )
    })?;
    raster_zone(&raster, project_epsg)?;
    for r in 0..raster.height {
        for c in 0..raster.width {
            let v = raster.value(r, c);
            if v.is_nan() {
                continue;
            }
            if !(v.is_finite() && v >= 0.0) {
                return Err(refuse(
                    "CONSTRAINTS_RASTER_VALUE",
                    format!(
                        "the multiplier raster holds {v} at row {r}, column {c}: a multiplier is \
                         a finite number, zero or more (no data counts as 1)"
                    ),
                ));
            }
        }
    }
    Ok(raster)
}

/// A multiplier raster file to set: its path relative to the document, and its bytes.
#[derive(Debug, Clone, Copy)]
pub struct MultiplierFile<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// A change to the constraints. A part left `None` is kept as it is.
#[derive(Debug, Clone, Default)]
pub struct ConstraintsEdit<'a> {
    /// The no-go areas, replacing the stored ones (an empty list removes them).
    pub no_go: Option<Vec<NoGoArea>>,
    /// The multiplier raster: `Some(Some(file))` sets it, `Some(None)` removes it.
    pub multiplier: Option<Option<MultiplierFile<'a>>>,
}

impl Session {
    fn project_epsg_value(&self) -> Option<u32> {
        self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32)
    }

    /// The constraints the document holds, or `None` when it holds none.
    pub fn constraints(&self) -> Result<Option<Constraints>, DesignError> {
        match self.doc["inputs"].get(INPUT) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => read_constraints(v).map(Some).map_err(input_error),
        }
    }

    /// Change the constraints: the one function the CLI and the web app set them through. A new
    /// multiplier raster is read and checked ([`read_multiplier`]), recorded by its path, hash, CRS
    /// and nodata, and attached. When every part is cleared the input is removed, so the document is
    /// as it was before any was set. The stages that read the constraints go stale. Nothing changes
    /// when it is refused.
    pub fn set_constraints(&mut self, edit: ConstraintsEdit) -> Result<Constraints, DesignError> {
        let mut c = self.constraints()?.unwrap_or_default();
        let mut attached = self.multiplier.clone();
        if let Some(no_go) = edit.no_go {
            c.no_go = no_go;
        }
        match edit.multiplier {
            None => {}
            Some(None) => {
                c.cut_rate_multiplier = None;
                attached = None;
            }
            Some(Some(f)) => {
                let raster =
                    read_multiplier(f.bytes, self.project_epsg_value()).map_err(input_error)?;
                c.cut_rate_multiplier = Some(MultiplierRaster {
                    path: f.path.to_string(),
                    sha256: super::hash::sha256_hex(f.bytes),
                    crs: super::terrain::dem_crs_name(&raster),
                    nodata: raster.nodata,
                });
                attached = Some(Arc::new(raster));
            }
        }
        c.validate().map_err(input_error)?;
        let inputs = self.doc["inputs"]
            .as_object_mut()
            .ok_or_else(|| DesignError::Input("the document has no inputs".into()))?;
        if c.is_empty() {
            inputs.shift_remove(INPUT);
        } else {
            inputs.insert(
                INPUT.into(),
                serde_json::to_value(&c).expect("constraints serialise"),
            );
        }
        self.multiplier = attached;
        Ok(c)
    }

    /// The path of the multiplier raster the document names, relative to the document.
    pub fn multiplier_path(&self) -> Option<&str> {
        self.doc["inputs"][INPUT]["cut_rate_multiplier"]["path"].as_str()
    }

    /// Attach the multiplier raster the document names, from the file the host found at
    /// [`Session::multiplier_path`]: only the file whose hash was recorded
    /// (`CONSTRAINTS_RASTER_CHANGED` otherwise). The stages that read it refuse until it is attached.
    pub fn attach_multiplier(&mut self, bytes: &[u8]) -> Result<(), DesignError> {
        let named = self.constraints()?.and_then(|c| c.cut_rate_multiplier);
        let Some(m) = named else {
            return Err(DesignError::Input(
                "the design names no cut-rate multiplier raster".into(),
            ));
        };
        if super::hash::sha256_hex(bytes) != m.sha256 {
            return Err(input_error(refuse(
                "CONSTRAINTS_RASTER_CHANGED",
                format!(
                    "the multiplier raster `{}` has changed since its hash was recorded: set it \
                     again to use the new file",
                    m.path
                ),
            )));
        }
        let raster = read_multiplier(bytes, self.project_epsg_value()).map_err(input_error)?;
        self.multiplier = Some(Arc::new(raster));
        Ok(())
    }

    /// The multiplier raster a stage reads: `Ok(None)` when the document names none, the raster when
    /// it is attached, and the path it names when it is not (the stage refuses with it).
    pub(crate) fn stage_multiplier(&self) -> Result<Option<Arc<Dem>>, String> {
        match self.multiplier_path() {
            None => Ok(None),
            Some(p) => self
                .multiplier
                .clone()
                .map(Some)
                .ok_or_else(|| p.to_string()),
        }
    }

    /// The project's EPSG code, for the multiplier sampler.
    pub(crate) fn multiplier_epsg(&self) -> Option<u32> {
        self.project_epsg_value()
    }
}
