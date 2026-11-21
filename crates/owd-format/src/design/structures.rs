//! The weirs, the locks and the pools of a design, for the map (MAP-4). Nothing here is stored in
//! the design file: the list is made on demand from the water level along the design line.
//!
//! A **pool** is a stretch of the design line where the water level is flat. A **weir site** is a
//! place where the level steps from one value to another: the lower pool ends there and the upper
//! one begins, and a lock lets the vessel pass. Chainage runs from the river mouth upstream along
//! the design line, so the pool upstream of a weir is the one at the higher chainage.
//!
//! The levels are read the way every stage reads them: from the water stage's answer
//! ([`water_stage::water`]), which takes the optimise stage's pools when it has run and the water
//! profile of the inputs (a pool table from `owd import --water-pools`) otherwise. A constant
//! level and a straight fall have no step, so they give no pool and no weir.
//!
//! The model of each weir and lock is a **placeholder model, not a design**: simple blocks whose
//! sizes come from the vessel by the rules in [`LockModel`], so that the map has something to
//! show where a structure will stand. No structure has been sized or checked.

use super::crs::Crs;
use super::session::{DesignError, Session};
use super::water_stage::{self, DesignWater, WaterSource};
use owd_core::geom::{Polygon, Pt, linear};
use serde::Serialize;

/// What every name of the model carries.
pub const PLACEHOLDER_NOTE: &str = "placeholder model, not a design";
/// The lock chamber is this many times the vessel's length.
pub const CHAMBER_LENGTH_FACTOR: f64 = 1.2;
/// The lock chamber is the vessel's beam and this much, in metres.
pub const CHAMBER_SIDE_ROOM_M: f64 = 2.0;
/// The thickness of a chamber wall, a gate block and the weir wall, in metres.
pub const WALL_THICKNESS_M: f64 = 3.0;
/// The top of the chamber walls and the gates is this far above the upstream pool level, in metres.
pub const WALL_FREEBOARD_M: f64 = 1.5;
/// The length of a weir where there is no river polygon to give the river's width, in metres.
pub const DEFAULT_WEIR_LENGTH_M: f64 = 100.0;
/// Two levels closer than this are one level, in metres.
const LEVEL_TOL_M: f64 = 1e-6;

/// The sizes of the placeholder lock and weir, and the rule each one comes from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LockModel {
    /// Always [`PLACEHOLDER_NOTE`].
    pub note: &'static str,
    /// The vessel's length overall, from the sizing stage. `None` before a vessel is chosen.
    pub vessel_length_m: Option<f64>,
    /// The vessel's beam, from the sizing stage.
    pub vessel_beam_m: Option<f64>,
    /// [`CHAMBER_LENGTH_FACTOR`] times the vessel's length. `None` without a vessel: no lock is drawn.
    pub chamber_length_m: Option<f64>,
    /// The vessel's beam and [`CHAMBER_SIDE_ROOM_M`].
    pub chamber_width_m: Option<f64>,
    pub wall_thickness_m: f64,
    /// How far the top of the walls is above the upstream pool level.
    pub wall_freeboard_m: f64,
    /// The weir's length where the river polygon does not give one.
    pub default_weir_length_m: f64,
    /// The rules in words, for the card.
    pub rules: &'static str,
}

impl LockModel {
    /// The model for a vessel of `(length, beam)` in metres.
    pub fn for_vessel(vessel: Option<(f64, f64)>) -> LockModel {
        let vessel = vessel.filter(|(l, b)| l.is_finite() && b.is_finite() && *l > 0.0 && *b > 0.0);
        LockModel {
            note: PLACEHOLDER_NOTE,
            vessel_length_m: vessel.map(|v| v.0),
            vessel_beam_m: vessel.map(|v| v.1),
            chamber_length_m: vessel.map(|v| CHAMBER_LENGTH_FACTOR * v.0),
            chamber_width_m: vessel.map(|v| v.1 + CHAMBER_SIDE_ROOM_M),
            wall_thickness_m: WALL_THICKNESS_M,
            wall_freeboard_m: WALL_FREEBOARD_M,
            default_weir_length_m: DEFAULT_WEIR_LENGTH_M,
            rules: "chamber length = 1.2 x vessel length; chamber width = vessel beam + 2 m; \
                    walls, gates and weir 3 m thick; top of walls 1.5 m above the upstream pool \
                    level; weir crest at the upstream pool level",
        }
    }
}

/// One pool: a stretch of the design line at one water level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PoolReach {
    /// Counted from the river mouth, from 1.
    pub number: usize,
    /// Design-line chainage of its downstream end, metres.
    pub from_m: f64,
    /// Design-line chainage of its upstream end, metres.
    pub to_m: f64,
    /// The water level, metres above datum.
    pub level_m: f64,
    /// Its reach of the design line, as `[lon, lat]` points from downstream to upstream.
    pub line: Vec<Pt>,
    /// The middle of the reach, as `[lon, lat]`: where its label goes.
    pub mid: Pt,
}

/// One block of the placeholder model: a flat-topped shape on the map.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelPart {
    /// `weir`, `lock_wall`, `lock_gate` or `lock_chamber` (the water in the chamber).
    pub kind: &'static str,
    /// The outline, as a closed ring of `[lon, lat]` points.
    pub ring: Vec<Pt>,
    /// The level of its top, metres above datum.
    pub top_m: f64,
}

/// One weir site: where one pool ends and the next begins.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeirSite {
    /// Counted from the river mouth, from 1.
    pub number: usize,
    /// Design-line chainage, metres.
    pub chainage_m: f64,
    /// Its place on the design line, as `[lon, lat]`.
    pub position: Pt,
    /// The bearing of the design line there, heading upstream: degrees clockwise from north.
    pub bearing_deg: f64,
    /// The level of the pool upstream: the level this weir holds.
    pub upstream_level_m: f64,
    /// The level downstream of the weir.
    pub downstream_level_m: f64,
    /// Upstream level less downstream level, metres.
    pub lift_m: f64,
    /// The number of the pool upstream, when the water there is a pool.
    pub upstream_pool: Option<usize>,
    /// The number of the pool downstream.
    pub downstream_pool: Option<usize>,
    /// The length of the weir across the river, bank to bank, metres.
    pub weir_length_m: f64,
    /// True when the length is the river polygon's width there; false for the default length.
    pub clipped_to_river: bool,
    /// The blocks of the placeholder model.
    pub parts: Vec<ModelPart>,
}

/// The pools and the weir sites of a design, with the placeholder model's sizes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MapStructures {
    pub model: LockModel,
    pub pools: Vec<PoolReach>,
    pub weirs: Vec<WeirSite>,
}

impl MapStructures {
    /// A design with no pools.
    pub fn none(vessel: Option<(f64, f64)>) -> MapStructures {
        MapStructures {
            model: LockModel::for_vessel(vessel),
            pools: Vec::new(),
            weirs: Vec::new(),
        }
    }
}

/// A flat stretch of the water level, on design-line chainage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelRun {
    pub from_m: f64,
    pub to_m: f64,
    pub level_m: f64,
}

/// A step of the water level: the level just downstream and just upstream of `chainage_m`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelStep {
    pub chainage_m: f64,
    pub downstream_m: f64,
    pub upstream_m: f64,
}

/// The flat stretches and the steps of the water along the design line. Empty when the water has
/// no step and no pool (a constant level, a fall): then the design has no pools to show.
pub fn level_runs(water: &DesignWater) -> (Vec<LevelRun>, Vec<LevelStep>) {
    let mut runs: Vec<LevelRun> = Vec::new();
    let mut steps = Vec::new();
    let mut open: Option<(f64, f64)> = None;
    let close = |open: &mut Option<(f64, f64)>, at: f64, runs: &mut Vec<LevelRun>| {
        if let Some((from, level)) = open.take()
            && at > from
        {
            runs.push(LevelRun {
                from_m: from,
                to_m: at,
                level_m: level,
            });
        }
    };
    for w in water.points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let same_level = (b.level_m - a.level_m).abs() <= LEVEL_TOL_M;
        if b.chainage.0 - a.chainage.0 <= 1e-9 {
            // Two points at one place: a step when their levels differ.
            if !same_level {
                close(&mut open, a.chainage.0, &mut runs);
                steps.push(LevelStep {
                    chainage_m: a.chainage.0,
                    downstream_m: a.level_m,
                    upstream_m: b.level_m,
                });
            }
        } else if same_level {
            if open.is_none_or(|(_, level)| (level - a.level_m).abs() > LEVEL_TOL_M) {
                close(&mut open, a.chainage.0, &mut runs);
                open = Some((a.chainage.0, a.level_m));
            }
        } else {
            // The level slopes here: no pool.
            close(&mut open, a.chainage.0, &mut runs);
        }
    }
    if let Some(last) = water.points.last() {
        close(&mut open, last.chainage.0, &mut runs);
    }
    let pooled = water
        .stretches
        .iter()
        .any(|s| s.source == WaterSource::Pool);
    if steps.is_empty() && !pooled {
        return (Vec::new(), Vec::new());
    }
    (runs, steps)
}

/// Where the line `c + u * n` (unit `n`) crosses the edges of `polygons`, as the reach of the
/// river around `c`: `(left, right)` with `left <= 0 <= right`, in metres. `None` when `c` is not
/// inside a polygon.
fn river_span(polygons: &[Polygon], c: Pt, n: Pt) -> Option<(f64, f64)> {
    let mut crossings: Vec<f64> = Vec::new();
    for p in polygons {
        for ring in core::iter::once(&p.exterior).chain(&p.holes) {
            for e in ring.windows(2) {
                let (a, b) = (e[0], e[1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                // c + u n = a + t d
                let det = d[0] * n[1] - d[1] * n[0];
                if det.abs() < 1e-12 {
                    continue;
                }
                let r = [a[0] - c[0], a[1] - c[1]];
                let u = (d[0] * r[1] - d[1] * r[0]) / det;
                let t = (n[0] * r[1] - n[1] * r[0]) / det;
                // Half-open, so a crossing at a shared vertex is counted once.
                if (0.0..1.0).contains(&t) {
                    crossings.push(u);
                }
            }
        }
    }
    let right: Vec<f64> = crossings.iter().copied().filter(|u| *u > 0.0).collect();
    if right.len().is_multiple_of(2) {
        return None;
    }
    let near_right = right.iter().copied().fold(f64::INFINITY, f64::min);
    let near_left = crossings
        .iter()
        .copied()
        .filter(|u| *u <= 0.0)
        .fold(f64::NEG_INFINITY, f64::max);
    (near_left.is_finite() && near_right.is_finite()).then_some((near_left, near_right))
}

/// The corners of the rectangle around `c` that runs from `a0` to `a1` along the unit `t` and
/// from `b0` to `b1` along the unit `n`, as a closed ring.
fn block(c: Pt, t: Pt, n: Pt, along: (f64, f64), across: (f64, f64)) -> Vec<Pt> {
    let at = |a: f64, b: f64| [c[0] + a * t[0] + b * n[0], c[1] + a * t[1] + b * n[1]];
    vec![
        at(along.0, across.0),
        at(along.1, across.0),
        at(along.1, across.1),
        at(along.0, across.1),
        at(along.0, across.0),
    ]
}

/// Turns points of the project CRS into longitude and latitude in degrees.
pub type ToLonLat<'a> = &'a dyn Fn(&[Pt]) -> Result<Vec<Pt>, String>;

/// What the list is made from, in the project CRS (metres).
pub struct StructuresIn<'a> {
    /// The design line, from the mouth.
    pub line: &'a [Pt],
    /// The water level along it.
    pub water: &'a DesignWater,
    /// The river polygon(s): a weir is clipped to the river's width. Empty: the default length.
    pub polygons: &'a [Polygon],
    /// The vessel's length overall and beam, metres.
    pub vessel: Option<(f64, f64)>,
}

/// The pools and the weir sites of a design line. `to_lon_lat` turns points of the project CRS
/// into longitude and latitude in degrees.
pub fn structures(input: &StructuresIn, to_lon_lat: ToLonLat) -> Result<MapStructures, String> {
    let model = LockModel::for_vessel(input.vessel);
    let (runs, steps) = level_runs(input.water);
    let line = input.line;
    if line.len() < 2 || (runs.is_empty() && steps.is_empty()) {
        return Ok(MapStructures::none(input.vessel));
    }
    let length = linear::length(line);
    let at = |s: f64| linear::interpolate(line, s.clamp(0.0, length)).unwrap_or(line[0]);
    let mut pools = Vec::with_capacity(runs.len());
    for (i, r) in runs.iter().enumerate() {
        let reach = match linear::substring(line, r.from_m, r.to_m) {
            owd_core::geom::Geometry::LineString(l) => l,
            _ => vec![at(r.from_m), at(r.to_m)],
        };
        let mut pts = to_lon_lat(&reach)?;
        let mid = to_lon_lat(&[at((r.from_m + r.to_m) / 2.0)])?[0];
        pts.dedup();
        pools.push(PoolReach {
            number: i + 1,
            from_m: r.from_m,
            to_m: r.to_m,
            level_m: r.level_m,
            line: pts,
            mid,
        });
    }
    let pool_at = |c: f64, upstream: bool| {
        runs.iter()
            .position(|r| ((if upstream { r.from_m } else { r.to_m }) - c).abs() < 1e-6)
            .map(|i| i + 1)
    };
    let mut weirs = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        let c = at(s.chainage_m);
        // The line's direction there, heading upstream: across a chord, so a vertex at the site
        // does not turn it.
        let half = model
            .chamber_length_m
            .map_or(25.0, |l| l / 2.0)
            .clamp(5.0, (length / 2.0).max(5.0));
        let (back, ahead) = (at(s.chainage_m - half), at(s.chainage_m + half));
        let (dx, dy) = (ahead[0] - back[0], ahead[1] - back[1]);
        let norm = libm::hypot(dx, dy);
        if norm < 1e-9 {
            continue;
        }
        let t = [dx / norm, dy / norm];
        // To the left of the line, looking upstream.
        let n = [-t[1], t[0]];
        let span = river_span(input.polygons, c, n);
        let (left, right) =
            span.unwrap_or((-DEFAULT_WEIR_LENGTH_M / 2.0, DEFAULT_WEIR_LENGTH_M / 2.0));
        let wall_top = s.upstream_m + WALL_FREEBOARD_M;
        let w = WALL_THICKNESS_M;
        let mut shapes: Vec<(&'static str, Vec<Pt>, f64)> = Vec::new();
        // The weir stops at the outside of the lock walls: the navigation channel passes through
        // the lock, not over the weir.
        let clear = match (model.chamber_length_m, model.chamber_width_m) {
            (Some(len), Some(wid)) => {
                let (a, b) = (len / 2.0, wid / 2.0);
                shapes.push((
                    "lock_chamber",
                    block(c, t, n, (-a + w, a - w), (-b, b)),
                    s.downstream_m,
                ));
                shapes.push(("lock_wall", block(c, t, n, (-a, a), (b, b + w)), wall_top));
                shapes.push(("lock_wall", block(c, t, n, (-a, a), (-b - w, -b)), wall_top));
                shapes.push(("lock_gate", block(c, t, n, (-a, -a + w), (-b, b)), wall_top));
                shapes.push(("lock_gate", block(c, t, n, (a - w, a), (-b, b)), wall_top));
                b + w
            }
            _ => 0.0,
        };
        if clear == 0.0 {
            shapes.push((
                "weir",
                block(c, t, n, (-w / 2.0, w / 2.0), (left, right)),
                s.upstream_m,
            ));
        } else {
            if right > clear {
                shapes.push((
                    "weir",
                    block(c, t, n, (-w / 2.0, w / 2.0), (clear, right)),
                    s.upstream_m,
                ));
            }
            if left < -clear {
                shapes.push((
                    "weir",
                    block(c, t, n, (-w / 2.0, w / 2.0), (left, -clear)),
                    s.upstream_m,
                ));
            }
        }
        let mut parts = Vec::with_capacity(shapes.len());
        for (kind, ring, top_m) in shapes {
            parts.push(ModelPart {
                kind,
                ring: to_lon_lat(&ring)?,
                top_m,
            });
        }
        let ll = to_lon_lat(&[c, back, ahead])?;
        let mid_lat = ((ll[1][1] + ll[2][1]) / 2.0).to_radians();
        let bearing = libm::atan2(
            (ll[2][0] - ll[1][0]) * libm::cos(mid_lat),
            ll[2][1] - ll[1][1],
        )
        .to_degrees()
        .rem_euclid(360.0);
        weirs.push(WeirSite {
            number: i + 1,
            chainage_m: s.chainage_m,
            position: ll[0],
            bearing_deg: bearing,
            upstream_level_m: s.upstream_m,
            downstream_level_m: s.downstream_m,
            lift_m: s.upstream_m - s.downstream_m,
            upstream_pool: pool_at(s.chainage_m, true),
            downstream_pool: pool_at(s.chainage_m, false),
            weir_length_m: right - left,
            clipped_to_river: span.is_some(),
            parts,
        });
    }
    Ok(MapStructures {
        model,
        pools,
        weirs,
    })
}

/// What the pool water (the flooded area of each pool) is made from: the design line with its
/// chainage, the pools, and the river polygon, all in longitude and latitude.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PoolWaterIn {
    /// The whole design line, as `[lon, lat]` points from the mouth.
    pub line: Vec<Pt>,
    /// The design-line chainage of each point of `line`, metres.
    pub chainage_m: Vec<f64>,
    /// The pools: `(number, from_m, to_m, level_m)`.
    pub pools: Vec<(usize, f64, f64, f64)>,
    /// The rings of the river polygon(s), as `[lon, lat]` points. Empty: no polygon.
    pub polygon: Vec<Vec<Pt>>,
}

/// The line, water and polygon the list is made from.
struct Basis {
    line: Vec<Pt>,
    water: DesignWater,
    polygons: Vec<Polygon>,
    crs: super::crs::Utm,
}

impl Session {
    /// The vessel's length overall and beam as the sizing stage stored them, metres.
    fn stored_vessel(&self) -> Option<(f64, f64)> {
        let r = &self.doc["stages"]["sizing"]["results"];
        Some((r["loa_m"].as_f64()?, r["beam_m"].as_f64()?))
    }

    /// `None` when the design has no design line, no river, no water level, or a CRS in degrees.
    fn structures_basis(&self) -> Option<Basis> {
        let epsg = self.doc["project"]["crs"]["epsg"].as_u64()?;
        let Crs::Utm(crs) = Crs::from_epsg(epsg as u32).ok()? else {
            return None;
        };
        let input = self.water_input()?;
        let water = water_stage::water(&input).ok()?;
        Some(Basis {
            line: input.design_line.0,
            water,
            polygons: self.river_polygons().unwrap_or_default(),
            crs,
        })
    }

    /// The weirs, the locks and the pools of the design, for the map: made now from the water
    /// level along the design line the later stages read. Nothing is stored. A design with no
    /// pools gives an empty list.
    pub fn map_structures(&self) -> Result<MapStructures, DesignError> {
        let vessel = self.stored_vessel();
        let Some(b) = self.structures_basis() else {
            return Ok(MapStructures::none(vessel));
        };
        let crs = b.crs;
        structures(
            &StructuresIn {
                line: &b.line,
                water: &b.water,
                polygons: &b.polygons,
                vessel,
            },
            &|pts| crs.inverse_all(pts),
        )
        .map_err(DesignError::Input)
    }

    /// What the pool water is made from. `None` when the design has no pools.
    pub fn pool_water_input(&self) -> Result<Option<PoolWaterIn>, DesignError> {
        let Some(b) = self.structures_basis() else {
            return Ok(None);
        };
        let (runs, _) = level_runs(&b.water);
        if runs.is_empty() || b.line.len() < 2 {
            return Ok(None);
        }
        let err = DesignError::Input;
        let mut chainage_m = Vec::with_capacity(b.line.len());
        let mut s = 0.0;
        for (i, p) in b.line.iter().enumerate() {
            if i > 0 {
                s += libm::hypot(p[0] - b.line[i - 1][0], p[1] - b.line[i - 1][1]);
            }
            chainage_m.push(s);
        }
        let mut polygon = Vec::new();
        for p in &b.polygons {
            for ring in core::iter::once(&p.exterior).chain(&p.holes) {
                polygon.push(b.crs.inverse_all(ring).map_err(err)?);
            }
        }
        Ok(Some(PoolWaterIn {
            line: b.crs.inverse_all(&b.line).map_err(err)?,
            chainage_m,
            pools: runs
                .iter()
                .enumerate()
                .map(|(i, r)| (i + 1, r.from_m, r.to_m, r.level_m))
                .collect(),
            polygon,
        }))
    }
}
