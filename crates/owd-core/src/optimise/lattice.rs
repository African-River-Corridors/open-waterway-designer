//! Pool-by-pool lattice alignment search: the least-cost channel through each pool that never
//! bends tighter than R_min. Mirrors `optimise/lattice.py`.
//!
//! Per pool (the reach between two weirs, one water level), search an (x, y, heading) state
//! lattice with A*:
//!
//! - **State**: position snapped to `pitch_m`, heading snapped to one of `n_heading` bins, plus
//!   the turn direction that arrived (so an arc may not reverse straight into the opposite arc; a
//!   straight must sit between them).
//! - **Edges**: left and right arcs at `r_min x radius_multiples`, each turning EXACTLY one
//!   heading bin (so forward integration lands on bin headings and never drifts), plus one
//!   straight. R_min holds BY CONSTRUCTION; no radius check after the fact.
//! - **Edge cost**: the earthwork field of a trapezoidal channel swept along the primitive
//!   ([`FieldScorer`]): fairway cut to grade everywhere; side-slope cut only in sections where
//!   the fairway bites dry ground, and only on dry cells; wet and dry weighted separately; an
//!   optional deep-cut multiplier on dry cut. Weights, not prices. Plus, when
//!   `SectionSpec::length_weight` is above 0, that weight per metre of the primitive's length:
//!   water deeper than the bed costs nothing to cut, so without it length in deep water is free
//!   and a looped line can be the cheapest. Plus, when `SectionSpec::land_weight` is above 0, that
//!   weight per m2 of the channel's footprint at the water surface that lies outside the river
//!   polygon ([`FieldScorer::land_area`]), cut or no cut: ground the pool drowns deeper than the bed
//!   costs nothing to cut, so without it a line over drowned land beside the river costs the same
//!   as a line in the river.
//! - **Heuristic**: `"c_min"`: straight-line distance x the cheapest per-metre cost anywhere in
//!   the corridor (the length weight included: see [`min_cost_per_metre`]);
//!   `"reverse_dijkstra"`: a relaxed 2-D cost-to-go from the goal (tighter, but an 8-connected
//!   grid can slightly over-estimate); `"none"`: plain Dijkstra.
//! - **Endpoints**: pinned to the baseline's position and heading at both pool ends, so each
//!   result splices back into the baseline. The search ends on the goal pose EXACTLY (position and
//!   heading): from each state it expands within `close_reach_m` of the goal it tries a closing
//!   path, a Dubins curve of R_min (arc, straight, arc), costed like any edge, and the goal is a
//!   state of its own reached only that way (issue #6). Two adjacent pools then share the pose at
//!   their boundary and meet tangentially. Only when no closing path is found does the search fall
//!   back to a goal within `goal_pos_tol_m` (default half a pitch) and one heading bin; the result
//!   says so (`goal_closed` false) and `end_err_m` reports the miss.
//!
//! - **Corridor**: by default the corridor only says where the heuristic samples, and the search
//!   may go anywhere on the grid (the corridor's bounding rectangle). With
//!   `LatticeConfig::corridor_limit` the corridor is a real limit: no state, no point of a primitive
//!   and no point of a closing path may lie outside it ([`TerrainGrid::in_corridor`]). A pool that
//!   cannot be searched inside it fails as any other does: its end moves, or it keeps its baseline
//!   with the reason recorded.
//!
//! The search cost is a SEARCH objective. Re-measure the winner with
//! [`crate::volumes::sections::section_volumes`] before you quote a quantity.
//!
//! Terrain comes in through the sampler interface `terrain(xs, ys) -> z` (NaN where unknown). It
//! is sampled once onto a regular grid over the corridor ([`build_terrain_grid`]); despiking a DEM
//! is the caller's job before that.
//!
//! The array code of the Python (numpy sums, `arange`, `linspace`, `rint`) is written out with
//! the same arithmetic in the same order, so that the search takes the same decisions.

use crate::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::offset::{Cap, buffer_line};
use crate::geom::overlay::{Op, contains_xy, overlay};
use crate::geom::{Geometry, Polygon, Pt, linear};
use crate::types::{Refusal, TerrainSampler};
use crate::volumes::sections::arange;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// `(x, y, heading)`.
pub type Pose = (f64, f64, f64);

// =====================================================================================
// Small numpy and Python helpers
// =====================================================================================

/// numpy's `DOUBLE_pairwise_sum` (what `ndarray.sum()` does on a contiguous array).
fn np_sum(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &x in a {
            res += x;
        }
        return res;
    }
    if n <= 128 {
        let mut r = [0.0; 8];
        r.copy_from_slice(&a[..8]);
        let mut i = 8;
        while i < n - (n % 8) {
            for (j, rj) in r.iter_mut().enumerate() {
                *rj += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        return res;
    }
    let mut n2 = n / 2;
    n2 -= n2 % 8;
    np_sum(&a[..n2]) + np_sum(&a[n2..])
}

/// Python's `round(x)` to an int: half to even.
fn round_i(x: f64) -> i64 {
    libm::rint(x) as i64
}

/// Python's float `%`: the result takes the sign of the divisor.
fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) { m + b } else { m }
    } else {
        libm::copysign(0.0, b)
    }
}

const TAU: f64 = 2.0 * core::f64::consts::PI;

fn hypot(a: f64, b: f64) -> f64 {
    libm::hypot(a, b)
}

// =====================================================================================
// Configuration
// =====================================================================================

/// The bottom half-width: a number, or `half_width(radius_m)` so arcs are widened in bends.
pub enum HalfWidthSpec {
    Fixed(f64),
    Func(Box<dyn Fn(f64) -> f64>),
}

/// The channel the search sweeps, and how its cut is weighted.
///
/// Each arc is costed at the half-width for its own radius; the straight at `half_width(inf)`.
pub struct SectionSpec {
    pub half_width: HalfWidthSpec,
    /// Template side slope above the bottom edge.
    pub side_slope_h_per_v: f64,
    /// Added to the half-width (e.g. to match a rasterised volume integrator's footprint).
    pub footprint_corr_m: f64,
    /// Per m3 cut inside the river polygon.
    pub wet_weight: f64,
    /// Per m3 cut outside it.
    pub dry_weight: f64,
    /// Dry cut deeper than this ...
    pub deep_threshold_m: f64,
    /// ... is weighted by this as well.
    pub deep_multiplier: f64,
    /// Transverse reach of each cross-section.
    pub rake_half_m: f64,
    /// Transverse sampling step.
    pub rake_step_m: f64,
    /// Cross-section spacing along a primitive.
    pub ds_step_m: f64,
    /// Fairway samples that must be on known terrain.
    pub min_valid_fraction: f64,
    /// The cost of one metre of design line (sailing distance), in the units of the cut cost: one
    /// m3 of cut at weight 1 costs 1. Every lattice edge, closing path and baseline segment pays it
    /// for its length, so length through deep water, where cut is free, is no longer free. 0 (the
    /// default) is off and gives exactly the search without it. Must be finite and >= 0.
    pub length_weight: f64,
    /// The cost of one m2 of land taken, in the units of the cut cost (so m3 of cut per m2): every
    /// lattice edge, closing path and baseline segment pays it for the part of the channel's
    /// footprint that lies outside the river polygon ([`FieldScorer::land_area`]), whether or not
    /// any cut is needed there. A value equal to the channel depth prices land like a cut of the
    /// full depth. 0 (the default) is off and gives exactly the search without it. Must be finite
    /// and >= 0.
    pub land_weight: f64,
    /// The depth the footprint's side slopes rise through, from the bed to the water surface: the
    /// footprint's half-width is the scorer's half-width + `side_slope_h_per_v` x this (and no more
    /// than `rake_half_m`). The caller sets it to the design depth (pool level less the bed). 0 (the
    /// default) charges the bottom width only. Read only when `land_weight` is above 0. Must be
    /// finite and >= 0.
    pub land_depth_m: f64,
}

impl SectionSpec {
    /// The Python defaults for everything but the half-width.
    pub fn new(half_width: HalfWidthSpec) -> Self {
        Self {
            half_width,
            side_slope_h_per_v: 2.0,
            footprint_corr_m: 0.0,
            wet_weight: 1.0,
            dry_weight: 1.0,
            deep_threshold_m: f64::INFINITY,
            deep_multiplier: 1.0,
            rake_half_m: 250.0,
            rake_step_m: 4.0,
            ds_step_m: 8.0,
            min_valid_fraction: 0.6,
            length_weight: 0.0,
            land_weight: 0.0,
            land_depth_m: 0.0,
        }
    }

    /// Refuse a weight (or the footprint depth) that is not a finite number >= 0.
    pub fn check_weights(&self) -> Result<(), Refusal> {
        for (name, v) in [
            ("length_weight", self.length_weight),
            ("land_weight", self.land_weight),
            ("land_depth_m", self.land_depth_m),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("{name} must be a finite number >= 0"));
            }
        }
        Ok(())
    }

    pub fn half_width_at(&self, radius_m: f64) -> f64 {
        match &self.half_width {
            HalfWidthSpec::Fixed(v) => *v,
            HalfWidthSpec::Func(f) => f(radius_m),
        }
    }
}

/// How the lattice is built and searched. Every field can change the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeConfig {
    pub r_min_m: f64,
    pub radius_multiples: Vec<f64>,
    pub pitch_m: f64,
    pub n_heading: i64,
    /// Default 2 x pitch.
    pub straight_len_m: Option<f64>,
    /// Sampling step along each primitive.
    pub arc_step_m: f64,
    /// Default half a pitch.
    pub goal_pos_tol_m: Option<f64>,
    pub goal_head_tol_bins: f64,
    /// End on the goal pose exactly, through a closing Dubins path of R_min (default true).
    /// False gives the older search that stops within `goal_pos_tol_m` and one heading bin.
    pub close_goal: bool,
    /// Closing paths are tried from states this close to the goal. Default 3 x R_min.
    pub close_reach_m: Option<f64>,
    /// `"c_min"`, `"reverse_dijkstra"` or `"none"`.
    pub heuristic: String,
    /// More than 1 is weighted A*: faster, not optimal.
    pub heuristic_weight: f64,
    /// Drop states with g + h above this.
    pub prune_bound: f64,
    pub max_expand: i64,
    pub bounds_margin_m: f64,
    /// [`optimise_pools`] searches a pool longer than this (m of baseline) as consecutive pieces
    /// at the same water level ([`pool_pieces`]), so one search never has to cover tens of km and
    /// hit `max_expand` (issue #559). Default [`DEFAULT_MAX_POOL_PIECE_M`]; infinity never splits.
    pub max_pool_piece_m: f64,
    /// Hold the search to the corridor: no state, no point of a primitive and no point of a closing
    /// path may lie outside it (read on the grid's corridor mask, so to the nearest cell). False
    /// (the default) is the search as it was: bounded by the grid's rectangle only, the corridor
    /// feeding the heuristic.
    pub corridor_limit: bool,
}

/// The default `LatticeConfig::max_pool_piece_m`: 6 km. A pool of 6 to 7 km converges in seconds
/// under the default `max_expand` on a long, uneven river where a pool of tens of km hits it
/// (issue #559). On the synthetic sine river of the tests (amplitude 250 m, wavelength 1,600 m,
/// R_min 340 m, pitch 12 m, cell 5 m) a 6 km pool takes about a sixth of the default cap; see
/// `changelog.d/559.fixed.md` for the measured times.
pub const DEFAULT_MAX_POOL_PIECE_M: f64 = 6000.0;

/// How far a piece boundary may move from the even split, as a fraction of the even piece
/// length: an eighth, so no piece is longer than 1.25 x `max_pool_piece_m`.
pub const PIECE_BOUNDARY_SLACK: f64 = 0.125;

impl LatticeConfig {
    /// The Python defaults for everything but `r_min_m`.
    pub fn new(r_min_m: f64) -> Self {
        Self {
            r_min_m,
            radius_multiples: vec![1.0, 1.5, 3.0],
            pitch_m: 12.0,
            n_heading: 16,
            straight_len_m: None,
            arc_step_m: 10.0,
            goal_pos_tol_m: None,
            goal_head_tol_bins: 1.0,
            close_goal: true,
            close_reach_m: None,
            heuristic: "c_min".into(),
            heuristic_weight: 1.0,
            prune_bound: f64::INFINITY,
            max_expand: 4_000_000,
            bounds_margin_m: 20.0,
            max_pool_piece_m: DEFAULT_MAX_POOL_PIECE_M,
            corridor_limit: false,
        }
    }

    /// The Python `__post_init__`: the checks and the defaults that depend on the pitch.
    pub fn validated(mut self) -> Result<Self, Refusal> {
        if self.r_min_m <= 0.0 {
            return Err("r_min_m must be > 0".into());
        }
        let lowest = self
            .radius_multiples
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        if lowest < 1.0 {
            return Err("radius_multiples must all be >= 1 so no arc is tighter than R_min".into());
        }
        if !matches!(
            self.heuristic.as_str(),
            "c_min" | "reverse_dijkstra" | "none"
        ) {
            return Err("heuristic must be 'c_min', 'reverse_dijkstra' or 'none'".into());
        }
        if self.max_pool_piece_m.is_nan() || self.max_pool_piece_m <= 0.0 {
            return Err("max_pool_piece_m must be > 0".into());
        }
        if self.straight_len_m.is_none() {
            self.straight_len_m = Some(2.0 * self.pitch_m);
        }
        if self.goal_pos_tol_m.is_none() {
            self.goal_pos_tol_m = Some(0.5 * self.pitch_m);
        }
        if self.close_reach_m.is_none() {
            self.close_reach_m = Some(3.0 * self.r_min_m);
        }
        Ok(self)
    }

    fn straight_len(&self) -> f64 {
        self.straight_len_m.unwrap_or(2.0 * self.pitch_m)
    }

    fn goal_tol(&self) -> f64 {
        self.goal_pos_tol_m.unwrap_or(0.5 * self.pitch_m)
    }

    fn close_reach(&self) -> f64 {
        self.close_reach_m.unwrap_or(3.0 * self.r_min_m)
    }
}

// =====================================================================================
// Terrain on a grid
// =====================================================================================

/// Terrain sampled on a regular north-up grid. `(x0, y0)` is the CENTRE of the top-left cell;
/// row index grows southward. `valid` marks known ground inside the search corridor; `corridor`
/// marks every cell inside the corridor, known ground or not.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainGrid {
    pub ground: Vec<Vec<f64>>,
    pub wet: Vec<Vec<bool>>,
    pub valid: Vec<Vec<bool>>,
    /// The cells whose centre is inside the corridor: what `LatticeConfig::corridor_limit` holds
    /// the search to.
    pub corridor: Vec<Vec<bool>>,
    pub x0: f64,
    pub y0: f64,
    pub cell_m: f64,
}

impl TerrainGrid {
    fn rows(&self) -> usize {
        self.ground.len()
    }

    fn cols(&self) -> usize {
        self.ground.first().map_or(0, Vec::len)
    }

    fn rc(&self, x: f64, y: f64) -> (f64, f64) {
        ((self.y0 - y) / self.cell_m, (x - self.x0) / self.cell_m)
    }

    /// Bilinear ground level between cell centres; NaN off the grid or on unknown ground.
    pub fn sample_ground_at(&self, x: f64, y: f64) -> f64 {
        let (row, col) = self.rc(x, y);
        let (h, w) = (self.rows(), self.cols());
        let (c0, r0) = (col.floor(), row.floor());
        let ok = c0 >= 0.0 && r0 >= 0.0 && c0 < (w as f64 - 1.0) && r0 < (h as f64 - 1.0);
        if !ok {
            return f64::NAN;
        }
        let (fc, fr) = (col - c0, row - r0);
        let (cc, rr) = (c0 as usize, r0 as usize);
        let g = &self.ground;
        g[rr][cc] * (1.0 - fc) * (1.0 - fr)
            + g[rr][cc + 1] * fc * (1.0 - fr)
            + g[rr + 1][cc] * (1.0 - fc) * fr
            + g[rr + 1][cc + 1] * fc * fr
    }

    pub fn sample_ground(&self, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_ground_at(x, y))
            .collect()
    }

    /// Nearest-cell wet mask; false off the grid.
    pub fn sample_wet_at(&self, x: f64, y: f64) -> bool {
        let (row, col) = self.rc(x, y);
        let (h, w) = (self.rows() as f64, self.cols() as f64);
        let (r, c) = (libm::rint(row), libm::rint(col));
        if c >= 0.0 && r >= 0.0 && c < w && r < h {
            self.wet[r as usize][c as usize]
        } else {
            false
        }
    }

    /// Whether the nearest cell is inside the corridor; false off the grid.
    pub fn in_corridor(&self, x: f64, y: f64) -> bool {
        let (row, col) = self.rc(x, y);
        let (r, c) = (libm::rint(row), libm::rint(col));
        if c >= 0.0 && r >= 0.0 {
            self.corridor
                .get(r as usize)
                .and_then(|cells| cells.get(c as usize))
                .copied()
                .unwrap_or(false)
        } else {
            false
        }
    }

    pub fn sample_wet(&self, xs: &[f64], ys: &[f64]) -> Vec<bool> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_wet_at(x, y))
            .collect()
    }

    /// Cell centres for the given row and column indices.
    pub fn cell_centres(&self, rows: &[usize], cols: &[usize]) -> (Vec<f64>, Vec<f64>) {
        (
            cols.iter()
                .map(|&c| self.x0 + c as f64 * self.cell_m)
                .collect(),
            rows.iter()
                .map(|&r| self.y0 - r as f64 * self.cell_m)
                .collect(),
        )
    }

    /// `(xmin, ymin, xmax, ymax)` of the cell centres.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (h, w) = (self.rows(), self.cols());
        (
            self.x0,
            self.y0 - (h as f64 - 1.0) * self.cell_m,
            self.x0 + (w as f64 - 1.0) * self.cell_m,
            self.y0,
        )
    }
}

/// `(xmin, ymin, xmax, ymax)` of the polygons' vertices (shapely `.bounds`).
fn polygon_bounds(polys: &[Polygon]) -> (f64, f64, f64, f64) {
    let mut b = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in polys {
        for pt in &p.exterior {
            b.0 = b.0.min(pt[0]);
            b.1 = b.1.min(pt[1]);
            b.2 = b.2.max(pt[0]);
            b.3 = b.3.max(pt[1]);
        }
    }
    b
}

/// Sample `terrain` at cell centres over the corridor's bounds (plus `margin_m`).
///
/// Ground outside the corridor is kept (a side slope may daylight there); `valid` is known
/// ground INSIDE the corridor, which is where the heuristic samples.
pub fn build_terrain_grid(
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    corridor: &[Polygon],
    cell_m: f64,
    margin_m: f64,
) -> Result<TerrainGrid, Refusal> {
    let (mut xmin, mut ymin, mut xmax, mut ymax) = polygon_bounds(corridor);
    xmin -= margin_m;
    ymin -= margin_m;
    xmax += margin_m;
    ymax += margin_m;
    let w = ((xmax - xmin) / cell_m).ceil() as usize + 1;
    let h = ((ymax - ymin) / cell_m).ceil() as usize + 1;
    let xs: Vec<f64> = (0..w).map(|i| xmin + i as f64 * cell_m).collect();
    let ys: Vec<f64> = (0..h).map(|i| ymax - i as f64 * cell_m).collect();
    let mut px = Vec::with_capacity(h * w);
    let mut py = Vec::with_capacity(h * w);
    for &y in &ys {
        for &x in &xs {
            px.push(x);
            py.push(y);
        }
    }
    let z = terrain(&px, &py);
    if z.len() != h * w {
        return Err(format!(
            "cannot reshape array of size {} into shape ({h},{w})",
            z.len()
        ));
    }
    let mut ground = Vec::with_capacity(h);
    let mut wet = Vec::with_capacity(h);
    let mut valid = Vec::with_capacity(h);
    let mut inside = Vec::with_capacity(h);
    for r in 0..h {
        let row = &z[r * w..(r + 1) * w];
        let mut wet_row = Vec::with_capacity(w);
        let mut valid_row = Vec::with_capacity(w);
        let mut inside_row = Vec::with_capacity(w);
        for (c, &zc) in row.iter().enumerate() {
            let p = [px[r * w + c], py[r * w + c]];
            wet_row.push(contains_xy(river_polygon, p));
            let in_corridor = contains_xy(corridor, p);
            valid_row.push(zc.is_finite() && in_corridor);
            inside_row.push(in_corridor);
        }
        ground.push(row.to_vec());
        wet.push(wet_row);
        valid.push(valid_row);
        inside.push(inside_row);
    }
    Ok(TerrainGrid {
        ground,
        wet,
        valid,
        corridor: inside,
        x0: xmin,
        y0: ymax,
        cell_m,
    })
}

// =====================================================================================
// Primitives
// =====================================================================================

/// One motion primitive: an arc (or a straight) turning at most one heading bin.
#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    /// Radius; infinity for a straight.
    pub r: f64,
    /// +1 left, -1 right, 0 straight.
    pub sign: i8,
    /// `(s, x, y, heading)` along the primitive, in its own frame.
    pub pts: Vec<[f64; 4]>,
    /// `(x, y, heading)` at its end.
    pub end: (f64, f64, f64),
    pub arc_len: f64,
}

/// Left/right arcs at `r_min x radius_multiples` turning exactly one heading bin, plus one
/// straight of `straight_len_m`.
pub fn lattice_primitives(cfg: &LatticeConfig) -> Vec<Prim> {
    let mut prims = Vec::new();
    let bs = TAU / cfg.n_heading as f64;
    for &m in &cfg.radius_multiples {
        let r = cfg.r_min_m * m;
        let arc_len = r * bs;
        let n = std::cmp::max(2, round_i(arc_len / cfg.arc_step_m)) as usize;
        for sgn in [1_i8, -1] {
            let k = f64::from(sgn) / r;
            let pts: Vec<[f64; 4]> = (1..=n)
                .map(|i| {
                    let i = i as f64;
                    let nn = n as f64;
                    [
                        arc_len * i / nn,
                        libm::sin(k * arc_len * i / nn) / k,
                        (1.0 - libm::cos(k * arc_len * i / nn)) / k,
                        k * arc_len * i / nn,
                    ]
                })
                .collect();
            prims.push(Prim {
                r,
                sign: sgn,
                pts,
                end: (
                    libm::sin(k * arc_len) / k,
                    (1.0 - libm::cos(k * arc_len)) / k,
                    k * arc_len,
                ),
                arc_len,
            });
        }
    }
    let l = cfg.straight_len();
    let n = std::cmp::max(2, round_i(l / cfg.arc_step_m)) as usize;
    prims.push(straight_prim(l, n));
    prims
}

fn straight_prim(length: f64, n: usize) -> Prim {
    Prim {
        r: f64::INFINITY,
        sign: 0,
        pts: (1..=n)
            .map(|i| {
                let v = length * i as f64 / n as f64;
                [v, v, 0.0, 0.0]
            })
            .collect(),
        end: (length, 0.0, 0.0),
        arc_len: length,
    }
}

fn unit_straight(length: f64) -> Prim {
    let n = std::cmp::max(2, round_i(length / 10.0)) as usize;
    straight_prim(length, n)
}

// =====================================================================================
// Edge cost
// =====================================================================================

/// Weighted cut of a constant-width trapezoidal channel swept along one primitive.
///
/// `score(prim, pose) -> (cost, ok)`; `ok` is false when less than `min_valid_fraction` of the
/// fairway samples are on known terrain.
pub struct FieldScorer<'a> {
    grid: &'a TerrainGrid,
    grade: f64,
    spec: &'a SectionSpec,
    offs: Vec<f64>,
    tmpl: Vec<f64>,
    in_fairway: Vec<bool>,
    /// The offsets inside the footprint at the water surface: the land cost's width.
    in_footprint: Vec<bool>,
    land_half_m: f64,
}

/// The cross-sections of a primitive the scorer reads: every `stride`-th point, and the last.
fn sections_of(prim: &Prim, ds_step_m: f64) -> Vec<&[f64; 4]> {
    let pts = &prim.pts;
    let ds_native = prim.arc_len / pts.len() as f64;
    let stride = if ds_native > 0.0 {
        std::cmp::max(1, round_i(ds_step_m / ds_native)) as usize
    } else {
        1
    };
    let mut sel: Vec<&[f64; 4]> = pts.iter().step_by(stride).collect();
    if !core::ptr::eq(
        *sel.last().expect("a primitive has points"),
        &pts[pts.len() - 1],
    ) {
        sel.push(&pts[pts.len() - 1]);
    }
    sel
}

impl<'a> FieldScorer<'a> {
    pub fn new(grid: &'a TerrainGrid, grade_m: f64, hb_m: f64, spec: &'a SectionSpec) -> Self {
        let offs = arange(-spec.rake_half_m, spec.rake_half_m + 1e-6, spec.rake_step_m);
        let tmpl: Vec<f64> = offs
            .iter()
            .map(|o| grade_m + crate::py_max(0.0, o.abs() - hb_m) / spec.side_slope_h_per_v)
            .collect();
        let in_fairway = offs.iter().map(|o| o.abs() <= hb_m).collect();
        let land_half = hb_m + spec.side_slope_h_per_v * spec.land_depth_m;
        let in_footprint = offs.iter().map(|o| o.abs() <= land_half).collect();
        Self {
            grid,
            grade: grade_m,
            spec,
            offs,
            tmpl,
            in_fairway,
            in_footprint,
            land_half_m: land_half,
        }
    }

    /// The half-width of the footprint the land cost is charged on (m): the channel's plan
    /// half-width at the water surface, the scorer's half-width (the bottom half-width, widened on
    /// an arc, plus `footprint_corr_m`) + `side_slope_h_per_v` x `land_depth_m`.
    pub fn land_half_width_m(&self) -> f64 {
        self.land_half_m
    }

    /// The plan area (m2) of the channel's footprint along `prim` from `pose` that lies outside the
    /// river polygon: the rake samples within the footprint's half-width whose nearest grid cell is
    /// not wet, each `rake_step_m` wide and one section spacing long. Ground level plays no part: a
    /// sample on drowned land, on unknown ground or off the grid counts as land; a sample inside the
    /// river polygon never does. It is what `score` charges `land_weight` on.
    pub fn land_area(&self, prim: &Prim, pose: Pose) -> f64 {
        let (x0, y0, th0) = pose;
        let sel = sections_of(prim, self.spec.ds_step_m);
        let ds = prim.arc_len / sel.len() as f64;
        let (ct, st) = (libm::cos(th0), libm::sin(th0));
        let mut n = 0usize;
        for p in &sel {
            let wx = x0 + ct * p[1] - st * p[2];
            let wy = y0 + st * p[1] + ct * p[2];
            let hth = th0 + p[3];
            let (nx, ny) = (-libm::sin(hth), libm::cos(hth));
            for (j, &o) in self.offs.iter().enumerate() {
                if self.in_footprint[j] && !self.grid.sample_wet_at(wx + o * nx, wy + o * ny) {
                    n += 1;
                }
            }
        }
        n as f64 * self.spec.rake_step_m * ds
    }

    pub fn score(&self, prim: &Prim, pose: Pose) -> (f64, bool) {
        let (x0, y0, th0) = pose;
        let sp = self.spec;
        let sel = sections_of(prim, sp.ds_step_m);
        let n_sec = sel.len();
        let ds = prim.arc_len / n_sec as f64;
        let m = self.offs.len();

        let (ct, st) = (libm::cos(th0), libm::sin(th0));
        let mut gz = vec![0.0; n_sec * m];
        let mut wet = vec![false; n_sec * m];
        for (i, p) in sel.iter().enumerate() {
            let wx = x0 + ct * p[1] - st * p[2];
            let wy = y0 + st * p[1] + ct * p[2];
            let hth = th0 + p[3];
            let (nx, ny) = (-libm::sin(hth), libm::cos(hth));
            for (j, &o) in self.offs.iter().enumerate() {
                let gx = wx + o * nx;
                let gy = wy + o * ny;
                gz[i * m + j] = self.grid.sample_ground_at(gx, gy);
                wet[i * m + j] = self.grid.sample_wet_at(gx, gy);
            }
        }
        let n_fair = self.in_fairway.iter().filter(|&&f| f).count();
        if n_fair == 0 {
            return (f64::INFINITY, false);
        }
        let mut fair_finite = 0usize;
        for i in 0..n_sec {
            for j in 0..m {
                if self.in_fairway[j] && gz[i * m + j].is_finite() {
                    fair_finite += 1;
                }
            }
        }
        if (fair_finite as f64 / (n_sec * n_fair) as f64) < sp.min_valid_fraction {
            return (f64::INFINITY, false);
        }

        let mut cut = vec![0.0; n_sec * m];
        for i in 0..n_sec {
            let mut hits_dry = false;
            for j in 0..m {
                let z = gz[i * m + j];
                if z.is_finite() && self.in_fairway[j] {
                    let fc = crate::py_max(z - self.grade, 0.0);
                    cut[i * m + j] = fc;
                    if fc > 0.0 && !wet[i * m + j] {
                        hits_dry = true;
                    }
                }
            }
            if hits_dry {
                for j in 0..m {
                    let z = gz[i * m + j];
                    if z.is_finite() && !self.in_fairway[j] && !wet[i * m + j] {
                        cut[i * m + j] += crate::py_max(z - self.tmpl[j], 0.0);
                    }
                }
            }
        }

        let cell = sp.rake_step_m * ds;
        let dredge: Vec<f64> = cut
            .iter()
            .zip(&wet)
            .map(|(&c, &w)| c * if w { 1.0 } else { 0.0 })
            .collect();
        let excav: Vec<f64> = cut
            .iter()
            .zip(&wet)
            .map(|(&c, &w)| c * if w { 0.0 } else { 1.0 })
            .collect();
        let shallow: Vec<f64> = excav
            .iter()
            .map(|&e| if e > sp.deep_threshold_m { 0.0 } else { e })
            .collect();
        let deep: Vec<f64> = excav
            .iter()
            .map(|&e| if e > sp.deep_threshold_m { e } else { 0.0 })
            .collect();
        let mut total = (np_sum(&dredge) * sp.wet_weight
            + np_sum(&shallow) * sp.dry_weight
            + np_sum(&deep) * sp.dry_weight * sp.deep_multiplier)
            * cell;
        // Off at 0, and then not even added, so the cost is the same number as without it.
        if sp.length_weight != 0.0 {
            total += sp.length_weight * prim.arc_len;
        }
        // The land cost, off at 0 in the same way: every footprint sample outside the river polygon
        // pays, cut or no cut (the same count as `land_area`).
        if sp.land_weight != 0.0 {
            let mut n = 0usize;
            for i in 0..n_sec {
                for j in 0..m {
                    if self.in_footprint[j] && !wet[i * m + j] {
                        n += 1;
                    }
                }
            }
            total += sp.land_weight * (n as f64 * cell);
        }
        (total, true)
    }
}

// =====================================================================================
// Heuristics
// =====================================================================================

/// `(row index / s, col index / s, x, y)` of the valid cells on every `s`-th row and column.
struct ValidPoints {
    ir: Vec<usize>,
    ic: Vec<usize>,
    xs: Vec<f64>,
    ys: Vec<f64>,
}

fn valid_points(grid: &TerrainGrid, stride_m: f64, max_samples: Option<usize>) -> ValidPoints {
    let s = std::cmp::max(1, round_i(stride_m / grid.cell_m)) as usize;
    let (mut rr, mut cc) = (Vec::new(), Vec::new());
    for (r, row) in grid.valid.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            if v && r % s == 0 && c % s == 0 {
                rr.push(r);
                cc.push(c);
            }
        }
    }
    if let Some(max) = max_samples
        && rr.len() > max
    {
        // np.linspace(0, size - 1, max).astype(int)
        let (n, last) = (max, (rr.len() - 1) as f64);
        let step = last / (n - 1) as f64;
        let idx: Vec<usize> = (0..n)
            .map(|i| if i == n - 1 { last } else { i as f64 * step } as usize)
            .collect();
        rr = idx.iter().map(|&i| rr[i]).collect();
        cc = idx.iter().map(|&i| cc[i]).collect();
    }
    let (xs, ys) = grid.cell_centres(&rr, &cc);
    ValidPoints {
        ir: rr.iter().map(|r| r / s).collect(),
        ic: cc.iter().map(|c| c / s).collect(),
        xs,
        ys,
    }
}

/// The cheapest per-metre straight cost anywhere in the corridor (the length weight if none is
/// valid). Straight cost is symmetric under a half turn, so half the heading bins are swept.
///
/// The scorer's cost of a straight includes `length_weight` x its length, so this is the cheapest
/// cut per metre plus `length_weight`. The `c_min` heuristic, this times the straight-line distance
/// to the goal, stays admissible with the weight on: every edge pays `length_weight` x its length,
/// and any path to the goal is at least the straight-line distance long, so the added term
/// (weight x distance) is a lower bound of the length cost still to pay and never over-estimates.
/// The cut part is the cheapest cut per metre, as before.
///
/// The land cost (`land_weight`) is part of the straight's cost in the same way as its cut: this is
/// the cheapest cut plus land per metre found in the corridor, and it is 0 for the land part as soon
/// as one sampled straight has its whole footprint inside the river polygon. It bounds the land
/// still to pay as it bounds the cut still to pay: the cheapest metre found, times the distance.
pub fn min_cost_per_metre(
    grid: &TerrainGrid,
    scorer: &FieldScorer,
    n_heading: i64,
    length_m: f64,
    stride_m: f64,
    max_samples: usize,
) -> f64 {
    let v = valid_points(grid, stride_m, Some(max_samples));
    let unit = unit_straight(length_m);
    let bs = TAU / n_heading as f64;
    let mut best = f64::INFINITY;
    for (&x, &y) in v.xs.iter().zip(&v.ys) {
        for b in 0..std::cmp::max(1, n_heading / 2) {
            let (c, ok) = scorer.score(&unit, (x, y, b as f64 * bs));
            if ok {
                best = crate::py_min(best, c / length_m);
            }
        }
    }
    if best.is_finite() {
        best
    } else {
        scorer.spec.length_weight
    }
}

/// Defaults of `min_cost_per_metre`.
pub const DEFAULT_C_MIN_STRIDE_M: f64 = 24.0;
pub const DEFAULT_C_MIN_MAX_SAMPLES: usize = 4000;

/// A 2-D cost-to-go from the goal on a coarse grid, curvature dropped and the cheapest heading
/// taken per cell.
pub struct ReverseDijkstra {
    d: Vec<Vec<f64>>,
    s: usize,
    x0: f64,
    y0: f64,
    cell_m: f64,
}

impl ReverseDijkstra {
    /// The cost to go from `(x, y)` (a large number off the reachable grid).
    pub fn h(&self, x: f64, y: f64) -> f64 {
        let s = self.s as f64;
        let i = round_i((self.y0 - y) / self.cell_m / s);
        let j = round_i((x - self.x0) / self.cell_m / s);
        let (nr, nc) = (
            self.d.len() as i64,
            self.d.first().map_or(0, Vec::len) as i64,
        );
        if 0 <= i && i < nr && 0 <= j && j < nc && self.d[i as usize][j as usize].is_finite() {
            return self.d[i as usize][j as usize];
        }
        1e18
    }
}

#[derive(PartialEq)]
struct Node(f64, usize, usize);

impl Eq for Node {}

impl Ord for Node {
    /// Reversed, so the `BinaryHeap` pops the smallest `(d, i, j)`, as Python's `heapq`.
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then(o.1.cmp(&self.1))
            .then(o.2.cmp(&self.2))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

pub fn reverse_dijkstra_heuristic(
    grid: &TerrainGrid,
    scorer: &FieldScorer,
    goal_xy: Pt,
    n_heading: i64,
    length_m: f64,
    stride_m: f64,
) -> ReverseDijkstra {
    let s = std::cmp::max(1, round_i(stride_m / grid.cell_m)) as usize;
    let (h, w) = (grid.rows(), grid.cols());
    let (nr, nc) = (h.div_ceil(s), w.div_ceil(s));
    let mut cg = vec![vec![f64::INFINITY; nc]; nr];
    let unit = unit_straight(length_m);
    let bs = TAU / n_heading as f64;
    let v = valid_points(grid, stride_m, None);
    for k in 0..v.xs.len() {
        let mut best = f64::INFINITY;
        for b in 0..std::cmp::max(1, n_heading / 2) {
            let (c, ok) = scorer.score(&unit, (v.xs[k], v.ys[k], b as f64 * bs));
            if ok {
                best = crate::py_min(best, c / length_m);
            }
        }
        cg[v.ir[k]][v.ic[k]] = best;
    }
    let step = s as f64 * grid.cell_m;
    let mut gr = round_i((grid.y0 - goal_xy[1]) / grid.cell_m / s as f64);
    let mut gc = round_i((goal_xy[0] - grid.x0) / grid.cell_m / s as f64);
    gr = gr.clamp(0, nr as i64 - 1);
    gc = gc.clamp(0, nc as i64 - 1);
    let (mut gr, mut gc) = (gr as usize, gc as usize);
    if !cg[gr][gc].is_finite() {
        // The finite cell nearest the goal in grid steps; the first one wins a tie.
        let mut best: Option<(i64, usize, usize)> = None;
        for (i, row) in cg.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                if v.is_finite() {
                    let d = (i as i64 - gr as i64).abs() + (j as i64 - gc as i64).abs();
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, i, j));
                    }
                }
            }
        }
        if let Some((_, i, j)) = best {
            gr = i;
            gc = j;
        }
    }
    let mut d = vec![vec![f64::INFINITY; nc]; nr];
    d[gr][gc] = 0.0;
    let mut pq = BinaryHeap::new();
    pq.push(Node(0.0, gr, gc));
    let diag = hypot(step, step);
    let neigh: [(i64, i64, f64); 8] = [
        (-1, 0, step),
        (1, 0, step),
        (0, -1, step),
        (0, 1, step),
        (-1, -1, diag),
        (-1, 1, diag),
        (1, -1, diag),
        (1, 1, diag),
    ];
    while let Some(Node(dist, i, j)) = pq.pop() {
        if dist > d[i][j] + 1e-9 {
            continue;
        }
        for &(di, dj, sl) in &neigh {
            let (a, b) = (i as i64 + di, j as i64 + dj);
            if 0 <= a && a < nr as i64 && 0 <= b && b < nc as i64 {
                let (a, b) = (a as usize, b as usize);
                if cg[a][b].is_finite() {
                    let nd = dist + sl * 0.5 * (cg[i][j] + cg[a][b]);
                    if nd < d[a][b] - 1e-9 {
                        d[a][b] = nd;
                        pq.push(Node(nd, a, b));
                    }
                }
            }
        }
    }
    ReverseDijkstra {
        d,
        s,
        x0: grid.x0,
        y0: grid.y0,
        cell_m: grid.cell_m,
    }
}

// =====================================================================================
// The search
// =====================================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct LatticeResult {
    pub converged: bool,
    /// Forward-integrated, tangent-continuous.
    pub line: Option<Vec<Pt>>,
    /// The search objective (weights, not prices).
    pub cost: f64,
    /// `(R or None for a straight, sign, arc_len)` per primitive.
    pub prim_seq: Vec<(Option<f64>, i8, f64)>,
    pub start_pose: Pose,
    pub goal_pose: Pose,
    /// The line ends on `goal_pose` exactly (position and heading), through a closing path. False
    /// when the search fell back to a goal within the tolerance, or did not converge.
    pub goal_closed: bool,
    pub end_err_m: f64,
    /// The heading miss at the end (rad, absolute); 0 when the goal is closed.
    pub end_heading_err_rad: f64,
    pub expansions: usize,
    pub n_states: usize,
    pub hit_cap: bool,
    pub timed_out: bool,
    /// `c_min_per_m` when the heuristic is `"c_min"`.
    pub c_min_per_m: Option<f64>,
    /// The line's minimum radius read on the chord (`min_radius_on_chord`, chord R_min/3); NaN
    /// when there is no line.
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    /// The chord reading holds R_min (within `RADIUS_TOL`); false when there is no line.
    pub radius_ok: bool,
    /// The plan area (m2) of the line's footprint outside the river polygon
    /// ([`FieldScorer::land_area`], summed over its primitives and closing path), measured when
    /// `SectionSpec::land_weight` is above 0 and there is a line; else `None`. The land part of
    /// `cost` is `land_weight` x this.
    pub land_area_m2: Option<f64>,
}

type Key = (i64, i64, usize, i8);

struct Entry(f64, f64, Key);

impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Entry {}

impl Ord for Entry {
    /// Reversed `(f, g, key)`: the heap pops the smallest tuple, as Python's `heapq`.
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then(o.1.partial_cmp(&self.1).unwrap_or(Ordering::Equal))
            .then(o.2.cmp(&self.2))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn bin(theta: f64, n: i64) -> usize {
    (round_i(py_mod(theta, TAU) / TAU * n as f64) % n) as usize
}

/// The points along an arc (or straight) of `arc_len` from a pose, `n` steps.
fn arc_points(x0: f64, y0: f64, th0: f64, r: f64, sgn: i8, arc_len: f64, n: usize) -> Vec<Pose> {
    let mut out = Vec::with_capacity(n);
    if sgn == 0 || r.is_infinite() {
        for k in 1..=n {
            let s = arc_len * k as f64 / n as f64;
            out.push((x0 + s * libm::cos(th0), y0 + s * libm::sin(th0), th0));
        }
        return out;
    }
    let kap = f64::from(sgn) / r;
    let (ct, st) = (libm::cos(th0), libm::sin(th0));
    for k in 1..=n {
        let s = arc_len * k as f64 / n as f64;
        let lx = libm::sin(kap * s) / kap;
        let ly = (1.0 - libm::cos(kap * s)) / kap;
        out.push((
            x0 + ct * lx - st * ly,
            y0 + st * lx + ct * ly,
            th0 + kap * s,
        ));
    }
    out
}

/// The longest step of the returned line along an arc (m), lattice primitive or closing path.
/// A line read for its radius is a polyline: points resampled along its chords sit inside the
/// circle, so a coarse arc reads tighter than it is (an R 150 m arc in 10 m steps reads 144.5 m on
/// 25 m vertices). At 2 m the reading is within about 0.1 % of the arc's radius.
pub const LINE_ARC_STEP_M: f64 = 2.0;

/// Steps of the returned line along an arc of `len` metres (at least 10, as before).
fn line_steps(len: f64) -> usize {
    std::cmp::max(10, (len / LINE_ARC_STEP_M - 1e-9).ceil() as i64) as usize
}

/// One piece of a closing path: `(radius, sign, length)`; a straight has an infinite radius and
/// sign 0.
type Piece = (f64, i8, f64);

/// The goal state: reached only through a closing path.
const GOAL: Key = (i64::MIN, i64::MIN, usize::MAX, 0);

/// Expansions the search goes on for, after it first meets the goal within the tolerance, to find
/// a closing path before it falls back to that state.
pub const CLOSE_GRACE_EXPANSIONS: i64 = 20_000;

/// The shortest arc-straight-arc path (a Dubins CSC curve) of radius `r` from pose `a` to pose
/// `b`, as three pieces. Its total turn is at most half a turn (no loops), and its first arc does
/// not turn against `insign`, the turn the lattice arrived with (an arc never reverses straight
/// into the opposite arc). `None` when no such path exists.
fn closing_path(a: Pose, b: Pose, r: f64, insign: i8) -> Option<[Piece; 3]> {
    let left = |th: f64| [-libm::sin(th), libm::cos(th)];
    let mut best: Option<(f64, [Piece; 3])> = None;
    for (s1, s2) in [(1_i8, 1_i8), (-1, -1), (1, -1), (-1, 1)] {
        let (f1, f2) = (f64::from(s1), f64::from(s2));
        let (n0, n1) = (left(a.2), left(b.2));
        let c0 = [a.0 + f1 * r * n0[0], a.1 + f1 * r * n0[1]];
        let c1 = [b.0 + f2 * r * n1[0], b.1 + f2 * r * n1[1]];
        let d = [c1[0] - c0[0], c1[1] - c0[1]];
        let dd = hypot(d[0], d[1]);
        // The straight leaves the first circle and meets the second at heading `phi`: the outer
        // tangent for two turns the same way, the inner one for two opposite turns.
        let (len, phi) = if s1 == s2 {
            if dd < 1e-9 {
                continue;
            }
            (dd, libm::atan2(d[1], d[0]))
        } else {
            if dd < 2.0 * r {
                continue;
            }
            let l = (dd * dd - 4.0 * r * r).max(0.0).sqrt();
            (l, libm::atan2(d[1], d[0]) + libm::atan2(2.0 * f1 * r, l))
        };
        let turn = |x: f64| {
            let t = py_mod(x, TAU);
            if t > TAU - 1e-9 { 0.0 } else { t }
        };
        let (a1, a2) = (turn(f1 * (phi - a.2)), turn(f2 * (b.2 - phi)));
        if a1 + a2 > core::f64::consts::PI + 1e-9 {
            continue;
        }
        if insign != 0 && s1 == -insign && a1 > 1e-9 {
            continue;
        }
        let total = r * (a1 + a2) + len;
        if best.as_ref().is_none_or(|(t, _)| total < *t) {
            best = Some((
                total,
                [(r, s1, r * a1), (f64::INFINITY, 0, len), (r, s2, r * a2)],
            ));
        }
    }
    best.map(|(_, p)| p)
}

/// A primitive of the given radius, sign and length (a straight when the sign is 0), sampled
/// about every `step_m`, for costing.
fn piece_prim(piece: Piece, step_m: f64) -> Prim {
    let (r, sgn, len) = piece;
    let n = std::cmp::max(2, (len / step_m).ceil() as i64) as usize;
    if sgn == 0 {
        return straight_prim(len, n);
    }
    let k = f64::from(sgn) / r;
    let pts = (1..=n)
        .map(|i| {
            let s = len * i as f64 / n as f64;
            [s, libm::sin(k * s) / k, (1.0 - libm::cos(k * s)) / k, k * s]
        })
        .collect();
    Prim {
        r,
        sign: sgn,
        pts,
        end: (
            libm::sin(k * len) / k,
            (1.0 - libm::cos(k * len)) / k,
            k * len,
        ),
        arc_len: len,
    }
}

/// The poses along each piece of `path` from `from` (pieces of no length are passed): an arc in
/// steps of at most [`LINE_ARC_STEP_M`], as the lattice's own arcs; a straight in 10 steps.
fn path_poses(from: Pose, path: &[Piece]) -> Vec<Vec<Pose>> {
    let mut cur = from;
    let mut out = Vec::new();
    for &(r, sgn, len) in path {
        if len <= 1e-9 {
            continue;
        }
        let n = if sgn == 0 { 10 } else { line_steps(len) };
        let pts = arc_points(cur.0, cur.1, cur.2, r, sgn, len, n);
        cur = pts[pts.len() - 1];
        out.push(pts);
    }
    out
}

enum Heuristic {
    CMin { c_min: f64, gx: f64, gy: f64 },
    Reverse(ReverseDijkstra),
    None,
}

impl Heuristic {
    fn h(&self, x: f64, y: f64) -> f64 {
        match self {
            Heuristic::CMin { c_min, gx, gy } => c_min * hypot(x - gx, y - gy),
            Heuristic::Reverse(r) => r.h(x, y),
            Heuristic::None => 0.0,
        }
    }
}

/// A* from `start_pose` to `goal_pose` over the lattice, with no time limit. See the module docs.
pub fn solve_lattice(
    grid: &TerrainGrid,
    start_pose: Pose,
    goal_pose: Pose,
    grade_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
) -> LatticeResult {
    solve_lattice_until(
        grid,
        start_pose,
        goal_pose,
        grade_m,
        section,
        cfg,
        &mut || false,
    )
}

/// [`solve_lattice`], stopped by the caller. The core reads no clock: the search asks `stop` once
/// in every 1000 expansions (when the count is a multiple of 1000). The first time it says stop, the
/// search ends timed out (`timed_out`, not `converged`, no line) and does not ask again. A caller
/// that wants a time limit builds `stop` from its own clock.
pub fn solve_lattice_until(
    grid: &TerrainGrid,
    start_pose: Pose,
    goal_pose: Pose,
    grade_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
    stop: &mut dyn FnMut() -> bool,
) -> LatticeResult {
    let prims = lattice_primitives(cfg);
    // One scorer per distinct half-width (rounded to 0.1 mm), as Python caches them.
    let hbs: Vec<f64> = prims
        .iter()
        .map(|p| section.half_width_at(p.r) + section.footprint_corr_m)
        .collect();
    let scorers: Vec<FieldScorer> = hbs
        .iter()
        .map(|&hb| FieldScorer::new(grid, grade_m, hb, section))
        .collect();
    let straight_scorer = &scorers[prims.len() - 1];
    let close_arc_scorer = FieldScorer::new(
        grid,
        grade_m,
        section.half_width_at(cfg.r_min_m) + section.footprint_corr_m,
        section,
    );

    let (gx0, gy0, gth) = goal_pose;
    let mut c_min_per_m = None;
    let heur = match cfg.heuristic.as_str() {
        "c_min" => {
            let c_min = min_cost_per_metre(
                grid,
                straight_scorer,
                cfg.n_heading,
                cfg.pitch_m,
                DEFAULT_C_MIN_STRIDE_M,
                DEFAULT_C_MIN_MAX_SAMPLES,
            );
            c_min_per_m = Some(c_min);
            Heuristic::CMin {
                c_min,
                gx: gx0,
                gy: gy0,
            }
        }
        "reverse_dijkstra" => Heuristic::Reverse(reverse_dijkstra_heuristic(
            grid,
            straight_scorer,
            [gx0, gy0],
            cfg.n_heading,
            cfg.pitch_m,
            cfg.pitch_m,
        )),
        _ => Heuristic::None,
    };
    let w = cfg.heuristic_weight;

    let bs = TAU / cfg.n_heading as f64;
    let head_tol = cfg.goal_head_tol_bins * bs;
    let goal_tol = cfg.goal_tol();

    let key = |x: f64, y: f64, th: f64, insign: i8| -> Key {
        (
            round_i(x / cfg.pitch_m),
            round_i(y / cfg.pitch_m),
            bin(th, cfg.n_heading),
            insign,
        )
    };
    let is_goal = |x: f64, y: f64, th: f64| -> bool {
        if hypot(x - gx0, y - gy0) > goal_tol {
            return false;
        }
        (py_mod(th - gth + core::f64::consts::PI, TAU) - core::f64::consts::PI).abs() <= head_tol
    };

    let (xmin, ymin, xmax, ymax) = grid.bounds();
    let m = cfg.bounds_margin_m;
    // With `corridor_limit`, a point must also be inside the corridor: a real limit of the search.
    let limit = cfg.corridor_limit;
    let in_bounds = |x: f64, y: f64| {
        xmin - m <= x
            && x <= xmax + m
            && ymin - m <= y
            && y <= ymax + m
            && (!limit || grid.in_corridor(x, y))
    };
    // The cost of a closing path from `from`; None when it leaves the grid (or, with
    // `corridor_limit`, the corridor) or meets unknown ground.
    let closing_cost = |from: Pose, path: &[Piece; 3]| -> Option<f64> {
        let mut cur = from;
        let mut total = 0.0;
        for &piece in path {
            if piece.2 <= 1e-9 {
                continue;
            }
            let prim = piece_prim(piece, cfg.arc_step_m);
            let sc = if piece.1 == 0 {
                straight_scorer
            } else {
                &close_arc_scorer
            };
            let (c, ok) = sc.score(&prim, cur);
            if !ok {
                return None;
            }
            let (ct, st) = (libm::cos(cur.2), libm::sin(cur.2));
            for q in &prim.pts {
                if !in_bounds(cur.0 + ct * q[1] - st * q[2], cur.1 + st * q[1] + ct * q[2]) {
                    return None;
                }
            }
            total += c;
            let (ex, ey, eth) = prim.end;
            cur = (
                cur.0 + ct * ex - st * ey,
                cur.1 + st * ex + ct * ey,
                cur.2 + eth,
            );
        }
        Some(total)
    };
    let close = cfg.close_goal;
    let reach = cfg.close_reach();
    let mut closing: Option<[Piece; 3]> = None;
    let mut closing_from: Option<Pose> = None;
    let mut fallback: Option<(Key, i64)> = None;
    let sk = key(start_pose.0, start_pose.1, start_pose.2, 0);
    let mut g: HashMap<Key, f64> = HashMap::new();
    let mut pose: HashMap<Key, Pose> = HashMap::new();
    let mut prev: HashMap<Key, Option<Key>> = HashMap::new();
    let mut prev_prim: HashMap<Key, Option<usize>> = HashMap::new();
    g.insert(sk, 0.0);
    pose.insert(sk, start_pose);
    prev.insert(sk, None);
    prev_prim.insert(sk, None);
    let mut pq = BinaryHeap::new();
    pq.push(Entry(w * heur.h(start_pose.0, start_pose.1), 0.0, sk));
    let mut expanded: i64 = 0;
    let mut best: Option<Key> = None;
    let mut res = LatticeResult {
        converged: false,
        line: None,
        cost: f64::INFINITY,
        prim_seq: Vec::new(),
        start_pose,
        goal_pose,
        goal_closed: false,
        end_err_m: f64::INFINITY,
        end_heading_err_rad: f64::INFINITY,
        expansions: 0,
        n_states: 0,
        hit_cap: false,
        timed_out: false,
        c_min_per_m,
        measured_min_radius_m: f64::NAN,
        radius_check_chord_m: default_check_chord(cfg.r_min_m),
        radius_ok: false,
        land_area_m2: None,
    };
    while let Some(Entry(_, gc, k)) = pq.pop() {
        if gc > g.get(&k).copied().unwrap_or(f64::INFINITY) + 1e-9 {
            continue;
        }
        if k == GOAL {
            best = Some(k);
            break;
        }
        let (x, y, th) = pose[&k];
        if is_goal(x, y, th) {
            if !close {
                best = Some(k);
                break;
            }
            fallback.get_or_insert((k, expanded));
        }
        if fallback.is_some_and(|(_, at)| expanded - at >= CLOSE_GRACE_EXPANSIONS) {
            break;
        }
        expanded += 1;
        if expanded >= cfg.max_expand {
            res.hit_cap = true;
            break;
        }
        if expanded % 1000 == 0 && stop() {
            res.timed_out = true;
            break;
        }
        let insign = k.3;
        // A closing path to the goal pose, from a state within reach (issue #6).
        if close && hypot(x - gx0, y - gy0) <= reach {
            let g_goal = g.get(&GOAL).copied().unwrap_or(f64::INFINITY);
            if gc < g_goal - 1e-6
                && let Some(path) = closing_path((x, y, th), goal_pose, cfg.r_min_m, insign)
                && let Some(c) = closing_cost((x, y, th), &path)
                && gc + c < g_goal - 1e-6
            {
                g.insert(GOAL, gc + c);
                pose.insert(GOAL, goal_pose);
                prev.insert(GOAL, Some(k));
                prev_prim.insert(GOAL, None);
                closing = Some(path);
                closing_from = Some((x, y, th));
                pq.push(Entry(gc + c, gc + c, GOAL));
            }
        }
        let (ct, st) = (libm::cos(th), libm::sin(th));
        for (pi, p) in prims.iter().enumerate() {
            let sgn = p.sign;
            if insign != 0 && sgn != 0 && sgn == -insign {
                continue; // no arc straight into the opposite arc
            }
            let (ex, ey, eth) = p.end;
            let nx = x + ct * ex - st * ey;
            let ny = y + st * ex + ct * ey;
            if !in_bounds(nx, ny) {
                continue;
            }
            // With the limit, every point of the primitive stays inside too: a long arc must not
            // bulge out of the corridor between two states inside it.
            if limit
                && !p
                    .pts
                    .iter()
                    .all(|q| grid.in_corridor(x + ct * q[1] - st * q[2], y + st * q[1] + ct * q[2]))
            {
                continue;
            }
            let (c, ok) = scorers[pi].score(p, (x, y, th));
            if !ok {
                continue;
            }
            let ng = gc + c;
            let hn = heur.h(nx, ny);
            if ng + hn > cfg.prune_bound {
                continue;
            }
            let nk = key(nx, ny, th + eth, sgn);
            if ng < g.get(&nk).copied().unwrap_or(f64::INFINITY) - 1e-6 {
                g.insert(nk, ng);
                pose.insert(nk, (nx, ny, th + eth));
                prev.insert(nk, Some(k));
                prev_prim.insert(nk, Some(pi));
                pq.push(Entry(ng + w * hn, ng, nk));
            }
        }
    }

    res.expansions = expanded as usize;
    res.n_states = g.len();
    if best.is_none() && !res.timed_out {
        best = fallback.map(|(k, _)| k);
    }
    let Some(best) = best else {
        return res;
    };
    let closed = best == GOAL;
    let mut chain = Vec::new();
    let mut cur = if closed { prev[&GOAL] } else { Some(best) };
    while let Some(k) = cur {
        chain.push(k);
        cur = prev[&k];
    }
    chain.reverse();
    let seq: Vec<usize> = chain[1..]
        .iter()
        .map(|k| prev_prim[k].expect("a non-start state has a primitive"))
        .collect();
    // Forward-integrate the exact primitive sequence from the true start pose: tangent-continuous
    // by construction. It is the geometry the costs were computed on unless a state on the way was
    // reached again more cheaply from another pose (see below).
    let (mut cx, mut cy, mut cth) = start_pose;
    let mut dense: Vec<Pt> = vec![[cx, cy]];
    for &pi in &seq {
        let p = &prims[pi];
        let n = if p.sign == 0 {
            10
        } else {
            line_steps(p.arc_len)
        };
        let pts = arc_points(cx, cy, cth, p.r, p.sign, p.arc_len, n);
        dense.extend(pts.iter().map(|q| [q.0, q.1]));
        (cx, cy, cth) = pts[pts.len() - 1];
    }
    // A state is a snapped position and heading. When one is reached again more cheaply its pose is
    // replaced, and states already reached from its old pose keep theirs; the chain run forward
    // from the start can then end some metres from the pose the closing path was computed from. A
    // closing path appended as it was would end that far off the goal, in line with it, while the
    // result says the goal is closed. So the closing path is computed again from where the line
    // really ends, and the line is costed as it is rebuilt; when no closing path leaves from there,
    // the goal is not closed and the result says so (`end_err_m`).
    let mut closed = closed;
    let mut cost = g[&best];
    if closed
        && closing_from.is_some_and(|f| {
            hypot(cx - f.0, cy - f.1) > 1e-9
                || (py_mod(cth - f.2 + core::f64::consts::PI, TAU) - core::f64::consts::PI).abs()
                    > 1e-12
        })
    {
        let insign = seq.last().map_or(0, |&pi| prims[pi].sign);
        let end = (cx, cy, cth);
        let redone = closing_path(end, goal_pose, cfg.r_min_m, insign)
            .and_then(|path| closing_cost(end, &path).map(|c| (path, c)));
        // The chain's own cost, primitive by primitive along the line as rebuilt.
        let (mut px, mut py, mut pth) = start_pose;
        let mut chain_cost = 0.0;
        for &pi in &seq {
            let p = &prims[pi];
            chain_cost += scorers[pi].score(p, (px, py, pth)).0;
            let (ct, st) = (libm::cos(pth), libm::sin(pth));
            let (ex, ey, eth) = p.end;
            (px, py, pth) = (px + ct * ex - st * ey, py + st * ex + ct * ey, pth + eth);
        }
        match redone {
            Some((path, c)) => {
                closing = Some(path);
                cost = chain_cost + c;
            }
            None => {
                closed = false;
                closing = None;
                cost = chain_cost;
            }
        }
    }
    let close_pieces: Vec<Piece> = match (closed, closing) {
        (true, Some(path)) => path.into_iter().filter(|p| p.2 > 1e-9).collect(),
        _ => Vec::new(),
    };
    // The land the line takes, read along the line as it is built (the chain from the true start,
    // then the closing path), with the scorer of each piece: what the land cost was charged on.
    if section.land_weight != 0.0 {
        let advance = |p: Pose, e: (f64, f64, f64)| -> Pose {
            let (ct, st) = (libm::cos(p.2), libm::sin(p.2));
            (
                p.0 + ct * e.0 - st * e.1,
                p.1 + st * e.0 + ct * e.1,
                p.2 + e.2,
            )
        };
        let mut at = start_pose;
        let mut area = 0.0;
        for &pi in &seq {
            area += scorers[pi].land_area(&prims[pi], at);
            at = advance(at, prims[pi].end);
        }
        for &piece in &close_pieces {
            let prim = piece_prim(piece, cfg.arc_step_m);
            let sc = if piece.1 == 0 {
                straight_scorer
            } else {
                &close_arc_scorer
            };
            area += sc.land_area(&prim, at);
            at = advance(at, prim.end);
        }
        res.land_area_m2 = Some(area);
    }
    for pts in path_poses((cx, cy, cth), &close_pieces) {
        dense.extend(pts.iter().map(|q| [q.0, q.1]));
        (cx, cy, cth) = pts[pts.len() - 1];
    }
    res.converged = true;
    res.goal_closed = closed;
    res.cost = cost;
    if let Ok(m) = min_radius_on_chord(&dense, res.radius_check_chord_m) {
        res.measured_min_radius_m = m;
        res.radius_ok = radius_ok(m, cfg.r_min_m);
    }
    res.line = Some(dense);
    res.prim_seq = seq
        .iter()
        .map(|&pi| {
            let p = &prims[pi];
            (
                if p.r.is_infinite() { None } else { Some(p.r) },
                p.sign,
                p.arc_len,
            )
        })
        .chain(
            close_pieces
                .iter()
                .map(|&(r, sgn, len)| (r.is_finite().then_some(r), sgn, len)),
        )
        .collect();
    res.end_err_m = hypot(cx - gx0, cy - gy0);
    res.end_heading_err_rad =
        (py_mod(cth - gth + core::f64::consts::PI, TAU) - core::f64::consts::PI).abs();
    res
}

// =====================================================================================
// Front door: per pool, then the whole line
// =====================================================================================

/// Tangent bearing (rad) at chainage `s`.
pub fn heading_at(line: &[Pt], s: f64, arm_m: f64) -> f64 {
    let a = linear::interpolate(line, crate::py_max(0.0, s - arm_m)).unwrap_or([f64::NAN; 2]);
    let b = linear::interpolate(line, crate::py_min(linear::length(line), s + arm_m))
        .unwrap_or([f64::NAN; 2]);
    libm::atan2(b[1] - a[1], b[0] - a[0])
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoolAlignment {
    /// Where the search starts and ends on the baseline. These are the pool's (or piece's) own
    /// ends unless a search end was moved (`start_moved_from_m`, `end_moved_from_m`).
    pub start_m: f64,
    pub end_m: f64,
    pub water_level_m: f64,
    pub grade_m: f64,
    pub result: LatticeResult,
    /// The baseline segment under the same scorer (straight width).
    pub baseline_cost: f64,
    /// The plan area (m2) of the baseline segment's footprint outside the river polygon, under the
    /// same scorer, measured when `SectionSpec::land_weight` is above 0; else `None`. The land part
    /// of `baseline_cost` is `land_weight` x this.
    pub baseline_land_area_m2: Option<f64>,
    /// Whether the lattice line is in the joined line (set by [`optimise_pools`]; a pool that
    /// converged but kept its baseline is not used).
    pub used: bool,
    /// Why the baseline was kept, when it was: the search hit its expansion limit, the search did
    /// not converge, or the baseline is cheaper and holds R_min.
    pub kept_baseline_because: Option<&'static str>,
    /// Set when this record is one piece of a pool longer than `max_pool_piece_m` (issue #559);
    /// `start_m` and `end_m` are then the piece's.
    pub piece: Option<PoolPiece>,
    /// Set when the search could not close on the pool's own end pose and closed on a pose moved
    /// along the baseline instead (issue #565): the pool's own end, `end_m` then being the moved
    /// search end. Only the search boundary moves; the water level does not change, and neither
    /// does where the weir is.
    pub end_moved_from_m: Option<f64>,
    /// Set when the previous pool's search end moved downstream into this pool, so this search
    /// starts there (`start_m`) instead of at the pool's own start, given here.
    pub start_moved_from_m: Option<f64>,
}

/// Where a piece sits in the pool it was cut from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolPiece {
    /// 0-based, in chainage order.
    pub index: usize,
    pub count: usize,
    pub pool_start_m: f64,
    pub pool_end_m: f64,
}

/// Why a pool keeps its baseline when its search hit `max_expand`.
pub const HIT_CAP_REASON: &str = "the search hit its expansion limit";

/// Why a pool keeps its baseline when its search could not close on its end pose at any search end
/// tried, and its line would join the next piece with a step that breaks R_min or crosses itself
/// (issue #565).
pub const UNCLOSED_REASON: &str = "the search could not end on the pool's end pose, even with the \
    end moved, and its line would join the next piece with a step that breaks R_min or crosses itself";

/// Why a pool keeps its baseline when its search did not converge.
pub const NOT_CONVERGED_REASON: &str = "the search did not converge";

/// Why a pool keeps its baseline when, with `corridor_limit`, its search found no line inside the
/// corridor: the line never leaves the corridor silently.
pub const NO_LINE_IN_CORRIDOR_REASON: &str = "the search found no line inside the corridor";

/// [`UNCLOSED_REASON`] with `corridor_limit` on: the closing path had to stay inside the corridor.
pub const UNCLOSED_IN_CORRIDOR_REASON: &str = "the search could not end on the pool's end pose \
    inside the corridor, even with the end moved, and its line would join the next piece with a \
    step that breaks R_min or crosses itself";

/// Why a pool keeps its baseline when its own line crosses itself at every search end tried
/// (issue #569): a design line must never contain a loop.
pub const SELF_CROSSING_REASON: &str = "its line crosses itself, even with the end moved: a loop is \
    not a design";

/// Why a pool keeps its baseline when its line crosses the rest of the joined line (another pool's
/// line, or the baseline) (issue #569).
pub const CROSSES_LINE_REASON: &str = "its line crosses the rest of the design line: a loop is not \
    a design";

/// A pool whose search cannot close on its end pose is searched again with its end moved along the
/// baseline in steps of this fraction of R_min ...
pub const BOUNDARY_STEP_FRACTION: f64 = 0.25;

/// ... up to this many steps (so up to one R_min) downstream first, then upstream. A moved end stays
/// at least one step inside the stretch it moves into (the next pool, or the pool itself).
pub const BOUNDARY_STEPS: usize = 4;

/// The piece boundaries of the pool `start_m..end_m` on `baseline`, both ends included: just
/// `[start_m, end_m]` when the pool is no longer than `max_piece_m`.
///
/// The rule: `n = ceil(length / max_piece_m)` pieces, as even as they can be; each inner boundary
/// then moves up to [`PIECE_BOUNDARY_SLACK`] of the even piece length (in 10 m steps) to where the
/// baseline turns least over `r_min_m` either side (the sum of the absolute heading changes over four
/// steps of `r_min_m / 2`), the nearer to the even split on a tie. A boundary pins the line to the
/// baseline's pose there, and a pose on a tight bend can force a turn tighter than R_min next to it;
/// where the baseline is straight or gently curved, the pinned pose costs the line nothing.
pub fn pool_pieces(
    baseline: &[Pt],
    start_m: f64,
    end_m: f64,
    max_piece_m: f64,
    r_min_m: f64,
) -> Vec<f64> {
    let len = end_m - start_m;
    if len.is_nan() || len <= max_piece_m {
        return vec![start_m, end_m];
    }
    let n = (len / max_piece_m).ceil() as usize;
    let nominal = len / n as f64;
    let slack = PIECE_BOUNDARY_SLACK * nominal;
    let blen = linear::length(baseline);
    let turn = |s: f64| -> f64 {
        let h = |c: f64| heading_at(baseline, c.clamp(0.0, blen), 20.0);
        let mut total = 0.0;
        let mut prev = h(s - r_min_m);
        for j in -1..=2 {
            let next = h(s + f64::from(j) * r_min_m / 2.0);
            total +=
                (py_mod(next - prev + core::f64::consts::PI, TAU) - core::f64::consts::PI).abs();
            prev = next;
        }
        total
    };
    let steps = (slack / 10.0).floor() as i64;
    let mut out = vec![start_m];
    for k in 1..n {
        let even = start_m + nominal * k as f64;
        let mut best = (turn(even), even);
        // Outward from the even split, so the nearer point wins a tie.
        for i in 1..=steps {
            for s in [even - 10.0 * i as f64, even + 10.0 * i as f64] {
                let t = turn(s);
                if t < best.0 - 1e-9 {
                    best = (t, s);
                }
            }
        }
        out.push(best.1);
    }
    out.push(end_m);
    out
}

/// The cost of the baseline segment under the straight scorer, and the land its footprint takes
/// (m2; 0 when `land_weight` is 0, where it is not measured).
fn segment_cost(seg: &[Pt], grid: &TerrainGrid, grade: f64, section: &SectionSpec) -> (f64, f64) {
    let hb = section.half_width_at(f64::INFINITY) + section.footprint_corr_m;
    let sc = FieldScorer::new(grid, grade, hb, section);
    let mut total = 0.0;
    let mut land = 0.0;
    for w in seg.windows(2) {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        let l = hypot(x1 - x0, y1 - y0);
        if l <= 0.0 {
            continue;
        }
        let n = std::cmp::max(1, round_i(l / 2.0)) as usize;
        let prim = Prim {
            r: f64::INFINITY,
            sign: 0,
            pts: (1..=n)
                .map(|k| {
                    let v = l * k as f64 / n as f64;
                    [v, v, 0.0, 0.0]
                })
                .collect(),
            end: (l, 0.0, 0.0),
            arc_len: l,
        };
        let pose = (x0, y0, libm::atan2(y1 - y0, x1 - x0));
        let (c, ok) = sc.score(&prim, pose);
        let area = if section.land_weight != 0.0 {
            sc.land_area(&prim, pose)
        } else {
            0.0
        };
        land += area;
        // A segment on unknown ground still pays for its length and for the land it takes, as
        // every lattice edge does.
        if ok {
            total += c;
        } else {
            if section.length_weight != 0.0 {
                total += section.length_weight * l;
            }
            if section.land_weight != 0.0 {
                total += section.land_weight * area;
            }
        }
    }
    (total, land)
}

fn polygons_of(g: Geometry) -> Result<Vec<Polygon>, Refusal> {
    match g {
        Geometry::Polygon(p) => Ok(vec![p]),
        Geometry::MultiPolygon(ps) => Ok(ps),
        Geometry::Empty(_) => Ok(Vec::new()),
        other => Err(format!("a buffer came out as {other:?}")),
    }
}

fn line_of(g: Geometry) -> Result<Vec<Pt>, Refusal> {
    match g {
        Geometry::LineString(l) => Ok(l),
        Geometry::Point(p) => Ok(vec![p]),
        other => Err(format!("a substring came out as {other:?}")),
    }
}

/// The corridor, cell size and half-width of `optimise_pool`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoolOptions {
    pub corridor_half_m: f64,
    pub cell_m: f64,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            corridor_half_m: 350.0,
            cell_m: 2.0,
        }
    }
}

/// Least-cost R_min-feasible alignment for one pool, pinned to `baseline` at `start_m` and
/// `end_m` (position and heading). Grade = `water_level_m - depth_m`.
///
/// The corridor is `corridor_half_m` around the baseline segment, unioned with the same buffer
/// around `corridor_line` (e.g. the natural river) between the projections of the two end points,
/// when given.
#[allow(clippy::too_many_arguments)]
pub fn optimise_pool(
    baseline: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    start_m: f64,
    end_m: f64,
    water_level_m: f64,
    depth_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
    corridor_line: Option<&[Pt]>,
    opts: PoolOptions,
) -> Result<PoolAlignment, Refusal> {
    let blen = linear::length(baseline);
    if !(0.0 <= start_m && start_m < end_m && end_m <= blen) {
        return Err("need 0 <= start_m < end_m <= baseline.length".into());
    }
    section.check_weights()?;
    let seg = line_of(linear::substring(baseline, start_m, end_m))?;
    let geo = |e: crate::geom::GeomError| e.to_string();
    let mut corridor =
        polygons_of(buffer_line(&seg, opts.corridor_half_m, 16, Cap::Round).map_err(geo)?)?;
    if let Some(cl) = corridor_line {
        let a = linear::project(cl, seg[0]);
        let b = linear::project(cl, seg[seg.len() - 1]);
        let part = line_of(linear::substring(
            cl,
            crate::py_min(a, b),
            crate::py_max(a, b),
        ))?;
        let other =
            polygons_of(buffer_line(&part, opts.corridor_half_m, 16, Cap::Round).map_err(geo)?)?;
        corridor = overlay(&corridor, &other, Op::Union);
    }
    let grid = build_terrain_grid(terrain, river_polygon, &corridor, opts.cell_m, 20.0)?;
    let grade = water_level_m - depth_m;
    let p0 = linear::interpolate(baseline, start_m).unwrap_or([f64::NAN; 2]);
    let p1 = linear::interpolate(baseline, end_m).unwrap_or([f64::NAN; 2]);
    let start = (p0[0], p0[1], heading_at(baseline, start_m, 20.0));
    let goal = (p1[0], p1[1], heading_at(baseline, end_m, 20.0));
    let result = solve_lattice(&grid, start, goal, grade, section, cfg);
    let (baseline_cost, baseline_land) = segment_cost(&seg, &grid, grade, section);
    Ok(PoolAlignment {
        start_m,
        end_m,
        water_level_m,
        grade_m: grade,
        result,
        baseline_cost,
        baseline_land_area_m2: (section.land_weight != 0.0).then_some(baseline_land),
        used: false,
        kept_baseline_because: None,
        piece: None,
        end_moved_from_m: None,
        start_moved_from_m: None,
    })
}

/// One join of the joined line: a point where a used pool's lattice line meets the next piece
/// (another pool's line, or the baseline).
#[derive(Debug, Clone, PartialEq)]
pub struct PoolJoin {
    /// Chainage of the join on the baseline (the pool boundary).
    pub at_m: f64,
    /// How far the line arriving at the join ends from the pose the next piece starts on: position
    /// (m) and heading (rad). Both 0 when the arriving pool's goal is closed.
    pub position_gap_m: f64,
    pub heading_gap_rad: f64,
    /// The joined line's minimum radius on the check chord, read on the line one chord either
    /// side of the join.
    pub measured_min_radius_m: f64,
    /// That reading holds R_min (within `RADIUS_TOL`).
    pub radius_ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoolsResult {
    pub line: Vec<Pt>,
    pub pools: Vec<PoolAlignment>,
    /// Every join of a used pool's line inside the line, in order (issue #6).
    pub joins: Vec<PoolJoin>,
    /// The joined line's minimum radius read on the chord (R_min/3).
    pub measured_min_radius_m: f64,
    pub radius_check_chord_m: f64,
    pub radius_ok: bool,
    /// The chainage along the joined line where it first crosses itself
    /// ([`crate::alignment::loops::first_self_crossing_m`]); `None` when it does not (issue #565).
    /// A design line must never contain a loop: this is reported, never silent.
    pub self_crossing_m: Option<f64>,
}

impl PoolsResult {
    pub fn n_replaced(&self) -> usize {
        self.pools.iter().filter(|p| p.used).count()
    }

    /// The joins that do not hold R_min.
    pub fn failed_joins(&self) -> impl Iterator<Item = &PoolJoin> {
        self.joins.iter().filter(|j| !j.radius_ok)
    }

    /// The pools (or pieces) that keep their baseline because their search could not close and
    /// their line would have broken the join ([`UNCLOSED_REASON`]).
    pub fn unclosed_pools(&self) -> impl Iterator<Item = &PoolAlignment> {
        self.pools.iter().filter(|p| {
            matches!(
                p.kept_baseline_because,
                Some(UNCLOSED_REASON | UNCLOSED_IN_CORRIDOR_REASON)
            )
        })
    }

    /// The pools (or pieces) that keep their baseline because, with `corridor_limit`, their search
    /// found no line inside the corridor ([`NO_LINE_IN_CORRIDOR_REASON`]).
    pub fn no_line_in_corridor_pools(&self) -> impl Iterator<Item = &PoolAlignment> {
        self.pools
            .iter()
            .filter(|p| p.kept_baseline_because == Some(NO_LINE_IN_CORRIDOR_REASON))
    }

    /// The pools (or pieces) that keep their baseline because their line crossed itself
    /// ([`SELF_CROSSING_REASON`]) or crossed the rest of the line ([`CROSSES_LINE_REASON`]).
    pub fn looped_pools(&self) -> impl Iterator<Item = &PoolAlignment> {
        self.pools.iter().filter(|p| {
            matches!(
                p.kept_baseline_because,
                Some(SELF_CROSSING_REASON | CROSSES_LINE_REASON)
            )
        })
    }

    /// The pools whose search end was moved so that the search closes with a simple line.
    pub fn moved_boundaries(&self) -> impl Iterator<Item = &PoolAlignment> {
        self.pools.iter().filter(|p| p.end_moved_from_m.is_some())
    }

    /// The pools (or pieces) whose search hit `max_expand` before it found a line, so they keep
    /// their baseline unsearched.
    pub fn capped_pools(&self) -> impl Iterator<Item = &PoolAlignment> {
        self.pools
            .iter()
            .filter(|p| p.result.hit_cap && !p.result.converged)
    }
}

/// Run [`optimise_pool`] on each `(start_m, end_m, water_level_m)` and splice the results into
/// `baseline`. A pool longer than `cfg.max_pool_piece_m` is searched as the consecutive pieces of
/// [`pool_pieces`] at its water level, one record each (`piece`); a piece is a pool in all that
/// follows. A pool keeps its baseline segment when the search does not converge (or hits
/// `max_expand`: [`HIT_CAP_REASON`]), or
/// (`keep_if_dearer`) when the result costs more than the baseline under the same scorer and the
/// baseline holds R_min (or the lattice line does not).
///
/// Each pool's line starts on the baseline's pose at its start and, when its goal is closed, ends
/// on the baseline's pose at its end, so it meets the next piece tangentially. Every join is read
/// for R_min on the joined line and reported (`joins`).
///
/// A pool whose search cannot close on its end pose (with `cfg.close_goal`; issue #565) is searched
/// again with its end moved along the baseline: in steps of [`BOUNDARY_STEP_FRACTION`] x R_min, up
/// to [`BOUNDARY_STEPS`] steps downstream first, then upstream, each moved end staying at least one
/// step inside the stretch it moves into. The first moved end that closes is kept and recorded
/// (`end_moved_from_m`). Moved downstream, the next pool's search starts there (`start_moved_from_m`);
/// moved upstream, the baseline keeps the stretch between. Only the search boundary moves: the water
/// levels do not change. When no end closes, the pool's fallback line (within `cfg.goal_pos_tol_m`
/// and one heading bin of its end pose) is spliced only if its join holds R_min and the joined line
/// does not cross itself there; otherwise the pool keeps its baseline ([`UNCLOSED_REASON`]). With
/// `close_goal` false (the older search) fallback lines are spliced as they are, and joins that
/// break R_min are reported.
///
/// With `cfg.corridor_limit` every search is held to its corridor (`opts.corridor_half_m` round the
/// baseline segment, and round `corridor_line` when given). A pool whose closing path would leave
/// the corridor is unclosed and goes the same way: moved ends first, then the baseline
/// ([`UNCLOSED_IN_CORRIDOR_REASON`]). A pool with no line inside the corridor at all keeps its
/// baseline ([`NO_LINE_IN_CORRIDOR_REASON`]). The baseline is the corridor's own axis, so the joined
/// line never leaves the corridor.
///
/// A design line must never contain a loop (issue #569). A pool whose own line crosses itself is
/// searched again on moved ends in the same way, and keeps its baseline
/// ([`SELF_CROSSING_REASON`]) when no end gives a simple line. After the join, a pool whose line
/// crosses the rest of the joined line keeps its baseline ([`CROSSES_LINE_REASON`]): at each
/// crossing the later pool of the two passes, one at a time, until the line is simple or only
/// baseline crosses baseline. In every case a joined line that still crosses itself says where
/// (`self_crossing_m`).
#[allow(clippy::too_many_arguments)]
pub fn optimise_pools(
    baseline: &[Pt],
    terrain: TerrainSampler,
    river_polygon: &[Polygon],
    pools: &[(f64, f64, f64)],
    depth_m: f64,
    section: &SectionSpec,
    cfg: &LatticeConfig,
    corridor_line: Option<&[Pt]>,
    opts: PoolOptions,
    keep_if_dearer: bool,
) -> Result<PoolsResult, Refusal> {
    let mut sorted = pools.to_vec();
    sorted.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .then(a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
    });
    let blen = linear::length(baseline);
    let mut out_pools = Vec::new();
    let mut split: Vec<(f64, f64, f64, Option<PoolPiece>)> = Vec::new();
    for (a, b, wl) in sorted {
        let bounds = pool_pieces(baseline, a, b, cfg.max_pool_piece_m, cfg.r_min_m);
        let count = bounds.len() - 1;
        for (index, w) in bounds.windows(2).enumerate() {
            let piece = (count > 1).then_some(PoolPiece {
                index,
                count,
                pool_start_m: a,
                pool_end_m: b,
            });
            split.push((w[0], w[1], wl, piece));
        }
    }
    let step = BOUNDARY_STEP_FRACTION * cfg.r_min_m;
    // `(search start, the pool's own start)` when the previous pool's search end moved into this one.
    let mut start_override: Option<(f64, f64)> = None;
    for (idx, &(a0, b0, wl, piece)) in split.iter().enumerate() {
        let (a, start_moved_from) = match start_override.take() {
            Some((s, own)) if (own - a0).abs() <= 1e-9 => (s, Some(own)),
            _ => (a0, None),
        };
        let search = |b: f64| {
            optimise_pool(
                baseline,
                terrain,
                river_polygon,
                a,
                b,
                wl,
                depth_m,
                section,
                cfg,
                corridor_line,
                opts,
            )
        };
        let mut pa = search(b0)?;
        // A pool is searched again with a moved end when its search could not close (with
        // `close_goal`), or when its own line crosses itself (issue #569): a loop is never spliced.
        let unclosed = |r: &LatticeResult| cfg.close_goal && !r.goal_closed;
        if pa.result.converged && (unclosed(&pa.result) || crosses_itself(&pa.result)) {
            // The next pool, when it starts where this one ends.
            let next = split
                .get(idx + 1)
                .filter(|n| (n.0 - b0).abs() <= 1e-9)
                .copied();
            let far = match (next, split.get(idx + 1)) {
                (Some(n), _) => n.1,
                (None, Some(n)) => n.0,
                (None, None) => blen,
            };
            let ends: Vec<f64> = (1..=BOUNDARY_STEPS)
                .map(|k| b0 + step * k as f64)
                .take_while(|&e| e <= far - step)
                .chain(
                    (1..=BOUNDARY_STEPS)
                        .map(|k| b0 - step * k as f64)
                        .take_while(|&e| e >= a + step),
                )
                .collect();
            for e in ends {
                let t = search(e)?;
                if t.result.converged && !unclosed(&t.result) && !crosses_itself(&t.result) {
                    if e > b0 && next.is_some() {
                        start_override = Some((e, b0));
                    }
                    pa = t;
                    pa.end_moved_from_m = Some(b0);
                    break;
                }
            }
        }
        let converged = pa.result.converged;
        let dearer = pa.result.cost > pa.baseline_cost;
        // The baseline is kept when the search did not converge, or when it is cheaper and either
        // holds R_min or the lattice line does not hold it either (issue #547): a baseline that
        // breaks R_min is never kept in place of a lattice line that holds it.
        let (use_it, because) = if !converged && pa.result.hit_cap {
            (false, Some(HIT_CAP_REASON))
        } else if !converged && cfg.corridor_limit && !pa.result.timed_out {
            (false, Some(NO_LINE_IN_CORRIDOR_REASON))
        } else if !converged {
            (false, Some(NOT_CONVERGED_REASON))
        } else if crosses_itself(&pa.result) {
            (false, Some(SELF_CROSSING_REASON))
        } else if keep_if_dearer && dearer {
            let base_ok = line_of(linear::substring(baseline, pa.start_m, pa.end_m))
                .ok()
                .and_then(|seg| min_radius_on_chord(&seg, default_check_chord(cfg.r_min_m)).ok())
                .is_some_and(|r| radius_ok(r, cfg.r_min_m));
            if base_ok || !pa.result.radius_ok {
                (
                    false,
                    Some("the baseline is cheaper and holds R_min (or the lattice line does not)"),
                )
            } else {
                (true, None)
            }
        } else {
            (true, None)
        };
        pa.used = use_it;
        pa.kept_baseline_because = because;
        pa.piece = piece;
        pa.start_moved_from_m = start_moved_from;
        out_pools.push(pa);
    }
    let chord = default_check_chord(cfg.r_min_m);
    // A pool that still cannot close keeps its baseline when its line breaks the join; keeping it
    // changes the line, so join again until nothing more is dropped (at most once per pool). Then a
    // pool whose line crosses the rest of the joined line keeps its baseline too (issue #569): at each
    // crossing, the later pool's line of the two passes (or the one pool's, when the other pass is
    // baseline), one pool per join, until the line is simple or no pool's line is in a crossing.
    let (coords, joins) = loop {
        let (coords, joins, spans) = join_line(baseline, blen, &out_pools, chord, cfg.r_min_m)?;
        let mut dropped = false;
        if cfg.close_goal {
            for p in out_pools
                .iter_mut()
                .filter(|p| p.used && !p.result.goal_closed)
            {
                let Some((j, s)) = joins.iter().find(|(j, _)| (j.at_m - p.end_m).abs() <= 1e-9)
                else {
                    continue; // the line ends where this pool ends: nothing joins it there
                };
                let total = linear::length(&coords);
                let window = line_of(linear::substring(
                    &coords,
                    crate::py_max(0.0, s - 2.0 * chord),
                    crate::py_min(total, s + 2.0 * chord),
                ))?;
                if !j.radius_ok || crate::alignment::loops::is_self_intersecting(&window) {
                    p.used = false;
                    p.kept_baseline_because = Some(if cfg.corridor_limit {
                        UNCLOSED_IN_CORRIDOR_REASON
                    } else {
                        UNCLOSED_REASON
                    });
                    dropped = true;
                }
            }
        }
        if !dropped && let Some(pi) = pool_in_a_crossing(&coords, &spans) {
            out_pools[pi].used = false;
            out_pools[pi].kept_baseline_because = Some(CROSSES_LINE_REASON);
            dropped = true;
        }
        if !dropped {
            break (coords, joins);
        }
    };
    let measured = min_radius_on_chord(&coords, chord).unwrap_or(f64::NAN);
    Ok(PoolsResult {
        radius_ok: radius_ok(measured, cfg.r_min_m),
        measured_min_radius_m: measured,
        radius_check_chord_m: chord,
        self_crossing_m: crate::alignment::loops::first_self_crossing_m(&coords),
        line: coords,
        pools: out_pools,
        joins: joins.into_iter().map(|(j, _)| j).collect(),
    })
}

/// Whether a converged search's own line crosses itself (issue #569).
fn crosses_itself(r: &LatticeResult) -> bool {
    r.line
        .as_deref()
        .is_some_and(crate::alignment::loops::is_self_intersecting)
}

/// The pool to keep as baseline for the first crossing of the joined line that a pool's line is in:
/// the pool on the later pass, else the pool on the earlier pass. `None` when the line does not
/// cross itself, or only baseline crosses baseline.
fn pool_in_a_crossing(coords: &[Pt], spans: &[Span]) -> Option<usize> {
    let pool_at = |s: f64| -> Option<usize> {
        let last = spans.len().checked_sub(1)?;
        spans
            .iter()
            .enumerate()
            .find(|(i, (a, b, _))| *a <= s && (s < *b || *i == last))
            .and_then(|(_, sp)| sp.2)
    };
    let mut crossings = crate::alignment::loops::self_crossings_m(coords);
    crossings.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    crossings
        .into_iter()
        .find_map(|(a, b)| pool_at(b).or_else(|| pool_at(a)))
}

/// One piece of the joined line: where it starts and ends along the line, and the index of the pool
/// whose line it is (`None` for the baseline).
type Span = (f64, f64, Option<usize>);

/// The joined line, each join with its chainage along that line, and each piece of the line: where
/// it starts and ends along the line, and the index of the pool whose line it is (`None` for the
/// baseline).
type JoinedLine = (Vec<Pt>, Vec<(PoolJoin, f64)>, Vec<Span>);

/// Splice the used pools' lines into `baseline`, and read every join of a used pool's line: each
/// with its chainage along the joined line.
fn join_line(
    baseline: &[Pt],
    blen: f64,
    pools: &[PoolAlignment],
    chord: f64,
    r_min_m: f64,
) -> Result<JoinedLine, Refusal> {
    // Each piece, with the baseline chainage it starts at and the pool whose line it is.
    let mut pieces: Vec<(Vec<Pt>, f64, Option<usize>)> = Vec::new();
    // The miss (position, heading) of each used pool at its far end, by its end chainage.
    let mut misses: Vec<(f64, f64, f64)> = Vec::new();
    let mut cursor = 0.0;
    for (pi, p) in pools.iter().enumerate().filter(|(_, p)| p.used) {
        misses.push((p.end_m, p.result.end_err_m, p.result.end_heading_err_rad));
        if p.start_m > cursor {
            pieces.push((
                line_of(linear::substring(baseline, cursor, p.start_m))?,
                cursor,
                None,
            ));
        }
        pieces.push((
            p.result.line.clone().unwrap_or_default(),
            p.start_m,
            Some(pi),
        ));
        cursor = p.end_m;
    }
    if cursor < blen {
        pieces.push((
            line_of(linear::substring(baseline, cursor, blen))?,
            cursor,
            None,
        ));
    }
    let mut coords: Vec<Pt> = Vec::new();
    // The index in `coords` where each piece starts.
    let mut piece_at: Vec<usize> = Vec::new();
    // The index in `coords` where each piece after the first starts, when a pool's line is on
    // either side of it.
    let mut join_at: Vec<(usize, f64)> = Vec::new();
    for (i, (pc, at, lattice)) in pieces.iter().enumerate() {
        if i > 0 && (lattice.is_some() || pieces[i - 1].2.is_some()) {
            join_at.push((coords.len().saturating_sub(1), *at));
        }
        piece_at.push(coords.len().saturating_sub(1));
        for xy in pc {
            let far = coords
                .last()
                .is_none_or(|l| hypot(xy[0] - l[0], xy[1] - l[1]) > 1e-9);
            if far {
                coords.push(*xy);
            }
        }
    }
    let mut cum = vec![0.0];
    for w in coords.windows(2) {
        cum.push(cum[cum.len() - 1] + hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]));
    }
    let total = cum[cum.len() - 1];
    let mut joins = Vec::new();
    for (idx, at) in join_at {
        let s = cum[idx.min(cum.len() - 1)];
        let window = line_of(linear::substring(
            &coords,
            crate::py_max(0.0, s - chord),
            crate::py_min(total, s + chord),
        ))?;
        let r = min_radius_on_chord(&window, chord).unwrap_or(f64::NAN);
        let (gap, head) = misses
            .iter()
            .find(|m| (m.0 - at).abs() <= 1e-9)
            .map_or((0.0, 0.0), |m| (m.1, m.2));
        joins.push((
            PoolJoin {
                at_m: at,
                position_gap_m: gap,
                heading_gap_rad: head,
                measured_min_radius_m: r,
                radius_ok: radius_ok(r, r_min_m),
            },
            s,
        ));
    }
    let spans = pieces
        .iter()
        .enumerate()
        .map(|(i, pc)| {
            let end = piece_at
                .get(i + 1)
                .map_or(total, |&k| cum[k.min(cum.len() - 1)]);
            (cum[piece_at[i].min(cum.len() - 1)], end, pc.2)
        })
        .collect();
    Ok((coords, joins, spans))
}

#[cfg(test)]
mod crossing_tests {
    //! Label: property (which pool keeps its baseline for a crossing of the joined line, #569).
    use super::*;

    /// East along y = 0 (baseline, then pool 0), then pool 1 turns back west across the baseline.
    fn line() -> (Vec<Pt>, Vec<Span>) {
        let coords = vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [200.0, 0.0],
            [200.0, 50.0],
            [50.0, 50.0],
            [50.0, -50.0],
        ];
        // Baseline 0..100 m, pool 0 from 100 to 250 m, pool 1 from 250 m to the end (500 m).
        (
            coords,
            vec![
                (0.0, 100.0, None),
                (100.0, 250.0, Some(0)),
                (250.0, 500.0, Some(1)),
            ],
        )
    }

    #[test]
    fn the_later_pool_of_a_crossing_keeps_its_baseline() {
        let (coords, spans) = line();
        // Pool 1 crosses the baseline at x = 50: pool 1 is on the later pass.
        assert_eq!(pool_in_a_crossing(&coords, &spans), Some(1));
        // With pool 1 baseline, the earlier pass's pool is named when the later pass is baseline.
        let spans = vec![(0.0, 100.0, Some(0)), (100.0, 500.0, None)];
        assert_eq!(pool_in_a_crossing(&coords, &spans), Some(0));
        // Baseline crossing baseline names no pool; a simple line names none.
        let spans = vec![(0.0, 500.0, None)];
        assert_eq!(pool_in_a_crossing(&coords, &spans), None);
        let simple = vec![[0.0, 0.0], [100.0, 0.0], [200.0, 50.0]];
        assert_eq!(pool_in_a_crossing(&simple, &[(0.0, 211.8, Some(0))]), None);
    }
}
