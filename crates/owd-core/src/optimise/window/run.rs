//! The window optimiser's driver (OPT-5 3.1): seams, the first pass, the splice, the staggered
//! second pass, the end-to-end measure and the records.

use super::grid::{CellMask, window_grid};
use super::line::{
    LoopCheck, curve_near, pose_on, radius_reading, short_reversals, splice, stretch,
};
use super::polish::{PolishSettings, Polished, polish};
use super::scorer::{
    Frame, HalfWidthRule, LevelAt, LineMeasure, Measure, MoveScorer, SectionSettings, Weights,
};
use super::seams::{Seam, SeamKind, SeamRules, admissible_near, find_seams};
use crate::alignment::loops::{first_self_crossing_m, self_crossings_m};
use crate::geom::overlay::PolygonIndex;
use crate::geom::{Geometry, Polygon, Pt, linear};
use crate::optimise::lattice::{
    GoalAfter, LatticeConfig, LatticeResult, Pose, SearchGoal, SearchStart, lattice_primitives,
    line_corridor, solve_lattice_with,
};
use crate::types::{Refusal, TerrainSampler};
use crate::volumes::sections::SectionCutter;

/// A window shorter than this (m) is not searched: its ends would pin one pose (OPT-5 lesson 10).
pub const MIN_WINDOW_M: f64 = 50.0;

/// What a state of the search holds in memory, about (bytes): its cost, pose, parent and primitive
/// in four maps, and its entries in the queue. Measured on the synthetic rivers (see the change
/// log); used only to limit the windows run at once.
pub const STATE_BYTES: f64 = 600.0;

/// How the window optimiser runs. Every field can change the answer, and every field is written
/// with the answer by the stage.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowConfig {
    pub window_target_m: f64,
    pub window_min_m: f64,
    pub window_max_m: f64,
    pub seam_step_m: f64,
    pub seam_max_offset_m: f64,
    pub seam_scales_m: Vec<f64>,
    pub second_pass: bool,
    /// The lattice: pitch, headings, radii, arc length, straight, minimum straight, caps. Validated.
    pub lattice: LatticeConfig,
    pub corridor_half_m: f64,
    pub cell_m: f64,
    /// The permitted-ground rule is on (resolved from `"auto"` by the caller).
    pub stay_in_river: bool,
    pub in_river_margin_m: f64,
    pub cut_through_room_m: f64,
    pub bottom_width_m: f64,
    pub window_time_s: f64,
    pub polish: Option<PolishSettings>,
    pub adopt_threshold: f64,
    pub turn_limit_deg: f64,
    pub length_ratio_limit: f64,
    /// The search's sampling: sections along a move, samples across.
    pub search_ds_m: f64,
    pub search_dx_m: f64,
    pub weights: Weights,
    /// The volumes stage's section (its own sampling across).
    pub section: SectionSettings,
    /// The volumes stage's station step: the adopt measure's.
    pub station_m: f64,
    pub weir_straight_m: f64,
    pub threads: usize,
    pub memory_budget_mb: f64,
}

/// The radii of the `"geometric"` mode: `n` radii from `r_min_m` to `factor x r_min_m`, evenly in
/// curvature on a log scale (`R_i = r_min_m x f^i`, `f = factor^(1/(n-1))`). R_min is the first.
pub fn geometric_radii(r_min_m: f64, n: usize, factor: f64) -> Vec<f64> {
    if n <= 1 {
        return vec![r_min_m];
    }
    let f = libm::pow(factor, 1.0 / (n - 1) as f64);
    (0..n).map(|i| r_min_m * libm::pow(f, i as f64)).collect()
}

impl WindowConfig {
    /// The defaults of OPT-5 (spec 3.3) for a channel of `r_min_m`, a vessel of `loa_m`, a bottom
    /// `bottom_width_m` wide and `depth_m` deep: windows of 1,500 m in 800 to 2,500 m, 64 headings,
    /// 12 radii from R_min to 8 R_min, arcs of about 80 m, the minimum straight one vessel length,
    /// the corridor limit on, the polish on, adoption at 1 %.
    pub fn defaults(r_min_m: f64, loa_m: f64, bottom_width_m: f64, depth_m: f64) -> Self {
        let mut lat = LatticeConfig::new(r_min_m);
        lat.n_heading = 64;
        lat.radii_m = Some(geometric_radii(r_min_m, 12, 8.0));
        lat.nominal_arc_m = Some(80.0);
        lat.min_straight_m = loa_m;
        lat.corridor_limit = true;
        lat.max_expand = 2_000_000;
        lat.max_states = 2_000_000;
        lat.heuristic = "c_min".into();
        let r = r_min_m;
        Self {
            window_target_m: 1500.0,
            window_min_m: 800.0,
            window_max_m: 2500.0,
            seam_step_m: 20.0,
            seam_max_offset_m: (0.25 * bottom_width_m).max(10.0),
            seam_scales_m: vec![r / 6.0, r / 3.0, 2.0 * r / 3.0, 4.0 * r / 3.0],
            second_pass: true,
            lattice: lat,
            corridor_half_m: 350.0,
            cell_m: 2.0,
            stay_in_river: false,
            in_river_margin_m: 0.0,
            cut_through_room_m: 2.0 * bottom_width_m,
            bottom_width_m,
            window_time_s: 300.0,
            polish: Some(PolishSettings {
                spacings_m: vec![r / 3.6, r / 3.0, r / 2.4, r / 1.8, r / 1.44],
                max_shift_m: (0.5 * bottom_width_m).max(15.0),
                station_m: 20.0,
                r_min_m: r,
                radius_margin_m: (0.06 * r).max(20.0),
                max_iter: 60,
                xtol_m: 0.5,
                ftol: 1e-4,
            }),
            adopt_threshold: 0.01,
            turn_limit_deg: crate::alignment::loops::TURN_LIMIT_DEG,
            length_ratio_limit: crate::alignment::loops::LEN_RATIO_BACKSTOP,
            search_ds_m: 8.0,
            search_dx_m: 2.0,
            weights: Weights::default(),
            section: SectionSettings {
                model: crate::cut_model::CUT_MODEL.into(),
                depth_m,
                wet_slope: crate::cut_model::WET_SLOPE_H_PER_V,
                maxw_m: crate::cut_model::MAXW_M,
                dx_m: crate::cut_model::DX_M,
                daylight_run_m: None,
            },
            station_m: crate::cut_model::STATION_M,
            weir_straight_m: 2.0 * loa_m,
            threads: 1,
            memory_budget_mb: 8192.0,
        }
    }
}

/// What the window optimiser reads.
pub struct WindowInputs<'a> {
    /// The starting line.
    pub line: &'a [Pt],
    pub river: &'a [Polygon],
    pub river_index: &'a PolygonIndex,
    pub centreline: Option<&'a [Pt]>,
    /// The ground (with the bed model under the river where the design has one). With no DEM, a
    /// flat ground below any bed, so nothing is cut (`plan_area`).
    pub terrain: TerrainSampler<'a>,
    pub plan_area: bool,
    /// The water level along the starting line, by its chainage.
    pub level: &'a (dyn Fn(f64) -> f64 + Sync),
    pub rule: &'a HalfWidthRule,
    /// Weir sites, recorded cut-throughs, forced off-river stretches and frozen reaches, as
    /// chainages of the starting line.
    pub weirs_m: Vec<f64>,
    pub cut_throughs_m: Vec<(f64, f64)>,
    pub forced_stretches_m: Vec<(f64, f64)>,
    /// Stretches of the starting line the line may stay off the river on, with the room either
    /// side (m): `(from, to, room)`. Read only with `stay_in_river`; they are not seams.
    pub permitted_m: Vec<(f64, f64, f64)>,
    pub frozen_m: Vec<(f64, f64)>,
    /// Seconds, from any origin: the time limit of each window and the records read it.
    pub clock: &'a (dyn Fn() -> f64 + Sync),
    /// True once the run should stop.
    pub cancel: &'a (dyn Fn() -> bool + Sync),
}

/// What a window took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    Found,
    PolishedStart,
    None,
}

impl Adopted {
    pub fn name(self) -> &'static str {
        match self {
            Adopted::Found => "found",
            Adopted::PolishedStart => "polished_start",
            Adopted::None => "none",
        }
    }
}

/// What the polish did, as a window records it.
#[derive(Debug, Clone, PartialEq)]
pub struct PolishRecord {
    pub accepted: bool,
    pub spacing_m: f64,
    pub by_spacing: Vec<(f64, f64)>,
    pub spread_pct: f64,
    pub max_shift_m: f64,
    pub min_radius_before_m: f64,
    pub min_radius_after_m: f64,
    pub evaluations: usize,
}

impl From<&Polished> for PolishRecord {
    fn from(p: &Polished) -> Self {
        Self {
            accepted: p.accepted,
            spacing_m: p.spacing_m,
            by_spacing: p.by_spacing.clone(),
            spread_pct: p.spread_pct,
            max_shift_m: p.max_shift_m,
            min_radius_before_m: p.min_radius_before_m,
            min_radius_after_m: p.min_radius_after_m,
            evaluations: p.evaluations,
        }
    }
}

/// The record of one window (OPT-5 3.12). Every window is recorded, adopted or not.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRecord {
    pub id: usize,
    pub pass: u8,
    /// Chainages of its seams on the line it was searched on.
    pub start_m: f64,
    pub end_m: f64,
    pub start_seam: Seam,
    pub end_seam: Seam,
    /// The search measure of the starting stretch and of the line found.
    pub search_cost_start: f64,
    pub search_cost_found: Option<f64>,
    /// The full measure, padded the same at both ends: the starting stretch as it is, polished, and
    /// the found line polished.
    pub full_start: Measure,
    pub full_start_polished: Option<Measure>,
    pub full_found: Option<Measure>,
    /// `100 x (1 - full found / full polished start)`.
    pub gain_pct: Option<f64>,
    pub adopted: Adopted,
    pub reason: &'static str,
    pub expansions: usize,
    pub states: usize,
    pub seconds: f64,
    pub polish_found: Option<PolishRecord>,
    pub polish_start: Option<PolishRecord>,
    pub loop_check: Option<LoopCheck>,
    /// The least radius on OWD's chord of the window's line as kept (with its padding).
    pub min_radius_m: f64,
    pub unknown_ground_pct: f64,
    /// Straights between opposite curves shorter than the minimum straight, on the line as kept.
    pub short_straights: usize,
    /// The found line's ends against the seams' poses (before they are put on them exactly).
    pub seam_gap_m: f64,
    pub seam_heading_gap_rad: f64,
    /// The window held a recorded cut-through and its found line went round the loop instead.
    pub cut_through_dropped: bool,
    /// A weir site at an end was not on a straight of the starting line.
    pub weir_not_on_straight: bool,
    /// The water level was one number over the window.
    pub level_fixed: bool,
}

/// What the window optimiser gives.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsResult {
    pub line: Vec<Pt>,
    /// The first pass's seams on the starting line.
    pub seams: Vec<Seam>,
    pub windows: Vec<WindowRecord>,
    /// The full measure of the whole starting line and of the result, end to end (the volumes
    /// stage's stations), on the ground given.
    pub start_total: Measure,
    pub result_total: Measure,
    /// The sum of the adopted windows' savings, and the end-to-end saving: they agree within 0.5 %
    /// of the starting line's cost, or `totals_agree` is false and a warning says so.
    pub window_saving_sum: f64,
    pub end_to_end_saving: f64,
    pub totals_agree: bool,
    pub min_radius_m: f64,
    pub radius_ok: bool,
    pub self_crossing_m: Option<f64>,
    pub short_straights: Vec<(f64, f64)>,
    pub windows_in_flight: usize,
    pub seconds: f64,
    pub cancelled: bool,
    pub warnings: Vec<String>,
}

/// One window to search: its seams on the current line and its stretch of the starting line's
/// chainage (for the water level).
#[derive(Debug, Clone)]
struct Job {
    id: usize,
    pass: u8,
    a: f64,
    b: f64,
    start_seam: Seam,
    end_seam: Seam,
    level_lo: f64,
    level_hi: f64,
    frozen: bool,
    /// Recorded cut-throughs inside the window, on the current line.
    cut_throughs: Vec<(f64, f64)>,
}

/// What a window gives back: its record and, when it adopted a line, the line from `a` to `b`.
struct Outcome {
    rec: WindowRecord,
    piece: Option<Vec<Pt>>,
}

/// The ground and the masks of one window, read on the main thread.
struct Prepared {
    grid: crate::optimise::lattice::TerrainGrid,
    mask: Option<CellMask>,
    level_fixed: Option<f64>,
}

fn polys_of(g: Geometry) -> Vec<Polygon> {
    match g {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(ps) => ps,
        _ => Vec::new(),
    }
}

/// The permitted ground of a window (OPT-5 3.16) on its grid: the river (grown by the margin when
/// one is set), and each permitted stretch of the starting line (a recorded cut-through, a forced
/// off-river stretch, a given line's own off-river stretch) with its room either side.
fn permitted_mask(
    inp: &WindowInputs,
    cfg: &WindowConfig,
    grid: &crate::optimise::lattice::TerrainGrid,
) -> CellMask {
    let mut mask = if cfg.in_river_margin_m != 0.0 {
        let grown = crate::geom::offset::buffer_polygons(inp.river, cfg.in_river_margin_m)
            .map(polys_of)
            .unwrap_or_else(|_| inp.river.to_vec());
        CellMask::on(grid, &grown)
    } else {
        // The grid's wet cells are the river polygon at the cell centres.
        CellMask {
            cells: grid.wet.clone(),
            x0: grid.x0,
            y0: grid.y0,
            cell_m: grid.cell_m,
        }
    };
    let len = linear::length(inp.line);
    let (gx0, gy0, gx1, gy1) = grid.bounds();
    for &(a, b, room) in &inp.permitted_m {
        let s = stretch(inp.line, a.max(0.0), b.min(len));
        if s.len() < 2 {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &s {
            x0 = x0.min(p[0] - room);
            y0 = y0.min(p[1] - room);
            x1 = x1.max(p[0] + room);
            y1 = y1.max(p[1] + room);
        }
        if x1 < gx0 || x0 > gx1 || y1 < gy0 || y0 > gy1 {
            continue;
        }
        let c0 = ((x0.max(gx0) - grid.x0) / grid.cell_m).floor().max(0.0) as usize;
        let c1 = ((x1.min(gx1) - grid.x0) / grid.cell_m).ceil() as usize;
        let r0 = ((grid.y0 - y1.min(gy1)) / grid.cell_m).floor().max(0.0) as usize;
        let r1 = ((grid.y0 - y0.max(gy0)) / grid.cell_m).ceil() as usize;
        for r in r0..=r1.min(mask.cells.len().saturating_sub(1)) {
            let row = &mut mask.cells[r];
            for c in c0..=c1.min(row.len().saturating_sub(1)) {
                if !row[c] {
                    let p = [
                        grid.x0 + c as f64 * grid.cell_m,
                        grid.y0 - r as f64 * grid.cell_m,
                    ];
                    row[c] = linear::point_line_distance(p, &s) <= room;
                }
            }
        }
    }
    mask
}

/// Read the window's ground (the only terrain reads of a window): on the main thread.
fn prepare(
    job: &Job,
    cur: &[Pt],
    inp: &WindowInputs,
    cfg: &WindowConfig,
) -> Result<Prepared, Refusal> {
    let seg = stretch(cur, job.a, job.b);
    let (corridor, _) = line_corridor(&seg, cfg.corridor_half_m)?;
    let pad = cfg.section.maxw_m + 20.0;
    let grid = window_grid(inp.terrain, inp.river_index, &corridor, pad, cfg.cell_m)?;
    let mask = cfg.stay_in_river.then(|| permitted_mask(inp, cfg, &grid));
    // One level over the window when the starting line's level is the same along all of it.
    let n = std::cmp::max(2, ((job.level_hi - job.level_lo) / 5.0).ceil() as usize + 1);
    let first = (inp.level)(job.level_lo);
    let same = (0..=n).all(|k| {
        let s = job.level_lo + (job.level_hi - job.level_lo) * k as f64 / n as f64;
        (inp.level)(s).to_bits() == first.to_bits()
    });
    Ok(Prepared {
        grid,
        mask,
        level_fixed: same.then_some(first),
    })
}

/// Run `f` on each job, `threads` at a time, the answers in the jobs' order.
fn run_all<J: Sync, T: Send>(jobs: &[J], threads: usize, f: impl Fn(&J) -> T + Sync) -> Vec<T> {
    if threads <= 1 || jobs.len() <= 1 {
        return jobs.iter().map(f).collect();
    }
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let done: Vec<Mutex<Option<T>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.min(jobs.len()) {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(k) else { break };
                    let answer = f(job);
                    *done[k].lock().expect("no worker panicked") = Some(answer);
                }
            });
        }
    });
    done.into_iter()
        .map(|m| {
            m.into_inner()
                .expect("no worker panicked")
                .expect("every job ran")
        })
        .collect()
}

/// The adopt rule (OPT-5 3.10): a line costing `new` replaces one costing `old` when it is at least
/// `threshold` cheaper (0.01: 1 %), and cheaper at all (a stretch that costs nothing has nothing to
/// gain).
pub fn adopts(new: f64, old: f64, threshold: f64) -> bool {
    new < old && new <= (1.0 - threshold) * old
}

/// The pose `d` metres back from `p` along its heading.
fn back(p: Pose, d: f64) -> Pose {
    (p.0 - d * libm::cos(p.2), p.1 - d * libm::sin(p.2), p.2)
}

/// Points along a straight from `p` for `d` metres, every 10 m or so (the start not included).
fn straight_points(p: Pose, d: f64) -> Vec<Pt> {
    let n = std::cmp::max(1, (d / 10.0).ceil() as usize);
    (1..=n)
        .map(|k| {
            let s = d * k as f64 / n as f64;
            [p.0 + s * libm::cos(p.2), p.1 + s * libm::sin(p.2)]
        })
        .collect()
}

/// The minimum straight the search holds, in whole straight primitives (m).
fn held_min_straight(lat: &LatticeConfig) -> f64 {
    let sl = lat.straight_len_m.unwrap_or(2.0 * lat.pitch_m);
    if lat.min_straight_m <= 0.0 {
        return 0.0;
    }
    (lat.min_straight_m / sl - 1e-9).ceil() * sl
}

/// The search measure of a stretch of line that is not a lattice line (the starting stretch): its
/// sections every `ds` along it, square to it, at the half-width of its local radius.
fn search_measure_of(scorer: &MoveScorer, seg: &[Pt], ds: f64, rule: &HalfWidthRule) -> f64 {
    let len = linear::length(seg);
    let n = std::cmp::max(1, (len / ds - 1e-9).ceil() as usize);
    let step = len / n as f64;
    let mut total = 0.0;
    for i in 0..n {
        let s = step * (i as f64 + 0.5);
        let at = |c: f64| linear::interpolate(seg, c.clamp(0.0, len)).unwrap_or([f64::NAN; 2]);
        let (p, a, b) = (at(s), at(s - 25.0), at(s + 25.0));
        let th = libm::atan2(b[1] - a[1], b[0] - a[0]);
        let r = super::seams::menger(a, p, b);
        let (c, _, _) = scorer.section(p[0], p[1], th, rule.at_radius(r));
        total += c.cost * step;
    }
    total + scorer.length_weight() * len
}

/// What a worker reads of the inputs: everything but the terrain, which only the main thread reads.
struct Shared<'a> {
    river_index: &'a PolygonIndex,
    level: &'a (dyn Fn(f64) -> f64 + Sync),
    rule: &'a HalfWidthRule,
    clock: &'a (dyn Fn() -> f64 + Sync),
    cancel: &'a (dyn Fn() -> bool + Sync),
    plan_area: bool,
}

/// Search, polish and judge one window. Runs on a worker: it reads no terrain.
#[allow(clippy::too_many_lines)]
fn run_window(
    job: &Job,
    prep: &Prepared,
    cur: &[Pt],
    cfg: &WindowConfig,
    inp: &Shared,
    project: &linear::ProjectIndex,
) -> Outcome {
    let t0 = (inp.clock)();
    let (a, b) = (job.a, job.b);
    let seg = stretch(cur, a, b);
    let r_min = cfg.lattice.r_min_m;
    let grid = &prep.grid;
    let level_fn = |p: Pt| (inp.level)(project.project(p).clamp(job.level_lo, job.level_hi));
    let level = match prep.level_fixed {
        Some(v) => LevelAt::Fixed(v),
        None => LevelAt::ByPoint(&level_fn),
    };
    // The window's ground for the full measure: the grid, read on the side of the river's edge
    // each sample is on (the cutter tests the same polygon).
    let grid_terrain = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| grid.sample_ground_split(x, y, inp.river_index.contains_xy([x, y])))
            .collect()
    };
    let cutter = SectionCutter::new(
        inp.river_index,
        cfg.section.depth_m,
        &cfg.section.model,
        cfg.section.wet_slope,
        cfg.section.maxw_m,
        cfg.section.dx_m,
        cfg.section.daylight_run_m,
    )
    .expect("the stage checked the cut model");
    let measure = LineMeasure::new(
        cutter,
        &grid_terrain,
        inp.rule,
        level,
        cfg.weights,
        &cfg.section,
        cfg.station_m,
    );
    let pad = (0.5 * r_min).max(2.0 * cfg.station_m);
    let cur_len = linear::length(cur);
    let pre = if a > 0.0 {
        stretch(cur, (a - pad).max(0.0), a)
    } else {
        vec![cur_point(cur, 0.0)]
    };
    let post = if b < cur_len {
        stretch(cur, b, (b + pad).min(cur_len))
    } else {
        vec![cur_point(cur, cur_len)]
    };
    // The window's line with the same padding of the current line at each end.
    let local = |x: &[Pt]| -> Vec<Pt> {
        let mut out = pre[..pre.len() - 1].to_vec();
        out.extend_from_slice(x);
        out.extend_from_slice(&post[1..]);
        super::line::dedup(out)
    };
    let local_start = local(&seg);
    let full_start = measure.measure(&local_start, Frame::FromStart);
    let (r_start, _) = radius_reading(&local_start, r_min);
    let mut rec = WindowRecord {
        id: job.id,
        pass: job.pass,
        start_m: a,
        end_m: b,
        start_seam: job.start_seam,
        end_seam: job.end_seam,
        search_cost_start: f64::NAN,
        search_cost_found: None,
        full_start,
        full_start_polished: None,
        full_found: None,
        gain_pct: None,
        adopted: Adopted::None,
        reason: "under_threshold",
        expansions: 0,
        states: 0,
        seconds: 0.0,
        polish_found: None,
        polish_start: None,
        loop_check: None,
        min_radius_m: r_start,
        unknown_ground_pct: 100.0 * full_start.unknown_share,
        short_straights: 0,
        seam_gap_m: 0.0,
        seam_heading_gap_rad: 0.0,
        cut_through_dropped: false,
        weir_not_on_straight: false,
        level_fixed: prep.level_fixed.is_some(),
    };
    let keep = |mut rec: WindowRecord, reason: &'static str| -> Outcome {
        rec.reason = reason;
        rec.seconds = (inp.clock)() - t0;
        Outcome { rec, piece: None }
    };
    if b - a < MIN_WINDOW_M {
        return keep(rec, "too_short");
    }
    if job.frozen {
        return keep(rec, "frozen");
    }
    if (inp.cancel)() {
        return keep(rec, "cancelled");
    }
    if full_start.unknown_share >= 1.0 && !inp.plan_area {
        return keep(rec, "no_ground");
    }
    let prims = lattice_primitives(&cfg.lattice);
    let radii: Vec<f64> = prims.iter().map(|p| p.r).chain([r_min]).collect();
    let sec_search = SectionSettings {
        dx_m: cfg.search_dx_m,
        ..cfg.section.clone()
    };
    let mask_fn = |x: f64, y: f64| prep.mask.as_ref().is_none_or(|m| m.at(x, y));
    let mut scorer = match MoveScorer::new(
        grid,
        &sec_search,
        cfg.search_ds_m,
        level,
        cfg.weights,
        inp.rule,
        &radii,
    ) {
        Ok(s) => s,
        Err(_) => return keep(rec, "no_path"),
    };
    if prep.mask.is_some() {
        scorer = scorer.with_mask(&mask_fn);
    }
    rec.search_cost_start = search_measure_of(&scorer, &seg, cfg.search_ds_m, inp.rule);

    // The two ends: the current line's exact poses, what the line does either side, and the
    // straight through a weir site.
    let held = held_min_straight(&cfg.lattice);
    let reach = held.max(1.0);
    let p_a = pose_on(cur, a);
    let p_b = pose_on(cur, b);
    let (turn_before, run_before) = curve_near(cur, a, r_min, reach, false);
    let (turn_after, run_after) = curve_near(cur, b, r_min, reach, true);
    let half_weir = 0.5 * cfg.weir_straight_m;
    let lead_start = if job.start_seam.kind == SeamKind::Weir {
        half_weir
    } else {
        0.0
    };
    let lead_end = if job.end_seam.kind == SeamKind::Weir {
        half_weir
    } else {
        0.0
    };
    for (kind, at) in [(job.start_seam.kind, a), (job.end_seam.kind, b)] {
        if kind == SeamKind::Weir
            && super::line::straight_run_through(cur, at) < cfg.weir_straight_m - 1.0
        {
            rec.weir_not_on_straight = true;
        }
    }
    if lead_start + lead_end >= b - a - MIN_WINDOW_M {
        return keep(rec, "too_short");
    }
    let start = SearchStart {
        pose: p_a,
        last_turn: turn_before,
        straight_run_m: run_before,
        lead_straight_m: lead_start,
    };
    let goal = if lead_end > 0.0 {
        back(p_b, lead_end)
    } else {
        p_b
    };
    let after = GoalAfter {
        turn: turn_after,
        run_m: run_after + lead_end,
    };
    let mut stop = || (inp.clock)() - t0 > cfg.window_time_s || (inp.cancel)();
    let res: LatticeResult = solve_lattice_with(
        grid,
        &start,
        &SearchGoal::Pose(goal),
        after,
        &scorer,
        &cfg.lattice,
        &mut stop,
    );
    rec.expansions = res.expansions;
    rec.states = res.n_states;
    if !res.converged {
        let why = if (inp.cancel)() {
            "cancelled"
        } else if res.timed_out {
            "timeout"
        } else if res.hit_cap {
            "cap"
        } else {
            "no_path"
        };
        return keep(rec, why);
    }
    if !res.goal_closed {
        return keep(rec, "not_closed");
    }
    let mut found = res.line.clone().unwrap_or_default();
    let mut search_found = res.cost;
    if lead_end > 0.0 {
        found.extend(straight_points(res.end_pose, lead_end));
        let tail = crate::optimise::lattice::Prim {
            r: f64::INFINITY,
            sign: 0,
            pts: Vec::new(),
            end: (lead_end, 0.0, 0.0),
            arc_len: lead_end,
        };
        let (c, _, _) = scorer.move_parts(&tail, res.end_pose);
        search_found += c.cost;
    }
    rec.search_cost_found = Some(search_found);
    let end = found[found.len() - 1];
    rec.seam_gap_m = libm::hypot(found[0][0] - p_a.0, found[0][1] - p_a.1)
        .max(libm::hypot(end[0] - p_b.0, end[1] - p_b.1));
    rec.seam_heading_gap_rad = res.end_heading_err_rad;
    found[0] = [p_a.0, p_a.1];
    let last = found.len() - 1;
    found[last] = [p_b.0, p_b.1];
    let found = super::line::dedup(found);

    // The polish: the found line and the starting stretch, the same way (between the weir leads).
    let allowed = |x: f64, y: f64| grid.in_corridor(x, y) && mask_fn(x, y);
    let do_polish = |line: &[Pt]| -> Option<(Vec<Pt>, Polished)> {
        let set = cfg.polish.as_ref()?;
        let len = linear::length(line);
        let (s0, s1) = (lead_start, len - lead_end);
        if s1 - s0 < MIN_WINDOW_M {
            return None;
        }
        let inner = stretch(line, s0, s1);
        let p = polish(&inner, &measure, Some(&allowed), set);
        if !p.accepted {
            return Some((line.to_vec(), p));
        }
        let (joined, _) = splice(line, s0, s1, &p.line);
        Some((joined, p))
    };
    let (found_line, pol_found) = match do_polish(&found) {
        Some((l, p)) => (l, Some(p)),
        None => (found.clone(), None),
    };
    let (start_line, pol_start) = match do_polish(&seg) {
        Some((l, p)) => (l, Some(p)),
        None => (seg.clone(), None),
    };
    rec.polish_found = pol_found.as_ref().map(PolishRecord::from);
    rec.polish_start = pol_start.as_ref().map(PolishRecord::from);

    // The adopt rule on the full measure over identical padded ends.
    let local_found = local(&found_line);
    let m_found = measure.measure(&local_found, Frame::FromStart);
    let polished_start = pol_start.as_ref().is_some_and(|p| p.accepted);
    let (local_pstart, m_pstart) = if polished_start {
        let l = local(&start_line);
        let m = measure.measure(&l, Frame::FromStart);
        (Some(l), m)
    } else {
        (None, full_start)
    };
    rec.full_found = Some(m_found);
    rec.full_start_polished = polished_start.then_some(m_pstart);
    rec.gain_pct = (m_pstart.cost > 0.0).then(|| 100.0 * (1.0 - m_found.cost / m_pstart.cost));
    rec.unknown_ground_pct = 100.0 * m_found.unknown_share;
    let lc = LoopCheck::read(&local_found, &found_line, b - a);
    rec.loop_check = Some(lc);
    let (r_found, ok_found) = radius_reading(&local_found, r_min);
    let radius_pass = ok_found || r_found >= r_start - 1e-6;
    let inside = |l: &[Pt]| {
        prep.mask
            .as_ref()
            .is_none_or(|m| l.iter().all(|p| m.at(p[0], p[1])))
    };
    let t = cfg.adopt_threshold;
    if !job.cut_throughs.is_empty() {
        rec.cut_through_dropped = super::line::drops_cut_through(
            cur,
            &job.cut_throughs,
            &found_line,
            cfg.cut_through_room_m,
        );
    }
    let found_ok = !lc.is_loop(cfg.turn_limit_deg, cfg.length_ratio_limit)
        && radius_pass
        && inside(&found_line);
    let wins = |new: f64, old: f64| adopts(new, old, t);
    let found_wins = wins(m_found.cost, m_pstart.cost);
    let pstart_wins = polished_start && wins(m_pstart.cost, full_start.cost);
    let (adopted, piece, reason, kept_local): (Adopted, Option<Vec<Pt>>, &'static str, Vec<Pt>) =
        if found_wins && found_ok {
            (Adopted::Found, Some(found_line), "gain", local_found)
        } else if pstart_wins
            && local_pstart.as_ref().is_some_and(|l| {
                let (r, ok) = radius_reading(l, r_min);
                (ok || r >= r_start - 1e-6) && inside(&start_line)
            })
        {
            (
                Adopted::PolishedStart,
                Some(start_line),
                "polished_start",
                local_pstart.clone().unwrap_or_default(),
            )
        } else {
            let why = if found_wins {
                if lc.is_loop(cfg.turn_limit_deg, cfg.length_ratio_limit) {
                    "loop"
                } else if !radius_pass {
                    "radius"
                } else {
                    "left_river"
                }
            } else if m_found.cost > m_pstart.cost {
                "dearer"
            } else {
                "under_threshold"
            };
            (Adopted::None, None, why, local_start.clone())
        };
    rec.adopted = adopted;
    rec.reason = reason;
    rec.min_radius_m = radius_reading(&kept_local, r_min).0;
    rec.short_straights =
        short_reversals(&kept_local, r_min, held, 2.0 * cfg.lattice.pitch_m).len();
    rec.seconds = (inp.clock)() - t0;
    Outcome { rec, piece }
}

/// The point of `line` at `s`.
fn cur_point(line: &[Pt], s: f64) -> Pt {
    linear::interpolate(line, s).unwrap_or([f64::NAN; 2])
}

/// Splice every adopted piece into `line` (pieces sorted by start, not overlapping): the new line
/// and each window's new span.
fn splice_all(line: &[Pt], spans: &[(f64, f64, Option<&Vec<Pt>>)]) -> (Vec<Pt>, Vec<(f64, f64)>) {
    let mut out = line.to_vec();
    // From the last to the first, so the chainages still to splice do not move.
    for &(a, b, piece) in spans.iter().rev() {
        if let Some(p) = piece {
            out = splice(&out, a, b, p).0;
        }
    }
    let mut shift = 0.0;
    let mut new_spans = Vec::with_capacity(spans.len());
    for &(a, b, piece) in spans {
        let len = piece.map_or(b - a, |p| linear::length(p));
        new_spans.push((a + shift, a + shift + len));
        shift += len - (b - a);
    }
    (out, new_spans)
}

/// One pass: prepare each window's ground (main thread, `in_flight` at a time), search the batch
/// in parallel, splice, and undo the later of two windows whose lines cross, one at a time.
fn run_pass(
    cur: &[Pt],
    jobs: &[Job],
    cfg: &WindowConfig,
    inp: &WindowInputs,
    project: &linear::ProjectIndex,
    in_flight: usize,
    warnings: &mut Vec<String>,
) -> (Vec<Pt>, Vec<WindowRecord>, Vec<(f64, f64)>) {
    let mut outcomes: Vec<Outcome> = Vec::with_capacity(jobs.len());
    let shared = Shared {
        river_index: inp.river_index,
        level: inp.level,
        rule: inp.rule,
        clock: inp.clock,
        cancel: inp.cancel,
        plan_area: inp.plan_area,
    };
    for batch in jobs.chunks(in_flight.max(1)) {
        let prepared: Vec<(Job, Result<Prepared, Refusal>)> = batch
            .iter()
            .map(|j| {
                let p = if (inp.cancel)() {
                    Err("cancelled".into())
                } else {
                    prepare(j, cur, inp, cfg)
                };
                (j.clone(), p)
            })
            .collect();
        let outs = run_all(&prepared, in_flight, |(job, prep)| match prep {
            Ok(prep) => run_window(job, prep, cur, cfg, &shared, project),
            Err(e) => {
                let measure_free = Measure::default();
                Outcome {
                    rec: WindowRecord {
                        id: job.id,
                        pass: job.pass,
                        start_m: job.a,
                        end_m: job.b,
                        start_seam: job.start_seam,
                        end_seam: job.end_seam,
                        search_cost_start: f64::NAN,
                        search_cost_found: None,
                        full_start: measure_free,
                        full_start_polished: None,
                        full_found: None,
                        gain_pct: None,
                        adopted: Adopted::None,
                        reason: if e == "cancelled" {
                            "cancelled"
                        } else {
                            "no_ground"
                        },
                        expansions: 0,
                        states: 0,
                        seconds: 0.0,
                        polish_found: None,
                        polish_start: None,
                        loop_check: None,
                        min_radius_m: f64::NAN,
                        unknown_ground_pct: 100.0,
                        short_straights: 0,
                        seam_gap_m: 0.0,
                        seam_heading_gap_rad: 0.0,
                        cut_through_dropped: false,
                        weir_not_on_straight: false,
                        level_fixed: false,
                    },
                    piece: None,
                }
            }
        });
        outcomes.extend(outs);
    }
    // The splice, and no window that makes the line cross itself.
    loop {
        let spans: Vec<(f64, f64, Option<&Vec<Pt>>)> = outcomes
            .iter()
            .map(|o| (o.rec.start_m, o.rec.end_m, o.piece.as_ref()))
            .collect();
        let (joined, new_spans) = splice_all(cur, &spans);
        let crossings = self_crossings_m(&joined);
        let start_crossings = self_crossings_m(cur);
        // A crossing the starting line has too is not a window's doing.
        let new_crossing = crossings.iter().find(|c| {
            !start_crossings
                .iter()
                .any(|s| (s.0 - c.0).abs() < 1.0 && (s.1 - c.1).abs() < 1.0)
        });
        let Some(&(s1, s2)) = new_crossing else {
            let recs = outcomes.into_iter().map(|o| o.rec).collect();
            return (joined, recs, new_spans);
        };
        // The later adopted window of the pair is undone.
        let holder = |s: f64| {
            new_spans
                .iter()
                .enumerate()
                .rev()
                .find(|(k, sp)| {
                    outcomes[*k].piece.is_some() && sp.0 - 1e-6 <= s && s <= sp.1 + 1e-6
                })
                .map(|(k, _)| k)
        };
        let Some(k) = holder(s2).or_else(|| holder(s1)) else {
            warnings.push(format!(
                "the joined line crosses itself at {s1:.0} m and {s2:.0} m and no adopted window holds \
                 either point"
            ));
            let recs = outcomes.into_iter().map(|o| o.rec).collect();
            return (joined, recs, new_spans);
        };
        outcomes[k].piece = None;
        outcomes[k].rec.adopted = Adopted::None;
        outcomes[k].rec.reason = "crosses_line";
    }
}

/// The window optimiser: see the module docs.
pub fn optimise_windows(inp: &WindowInputs, cfg: &WindowConfig) -> Result<WindowsResult, Refusal> {
    let t0 = (inp.clock)();
    let line = inp.line;
    if line.len() < 2 {
        return Err("the starting line needs at least two points".into());
    }
    let len = linear::length(line);
    let project = linear::ProjectIndex::new(line);
    let mut warnings: Vec<String> = Vec::new();

    // Forced seams and the stretches no free seam may stand in.
    let mut forced: Vec<(f64, SeamKind)> = Vec::new();
    let mut excluded: Vec<(f64, f64)> = Vec::new();
    for &w in &inp.weirs_m {
        forced.push((w, SeamKind::Weir));
        excluded.push((w - cfg.weir_straight_m, w + cfg.weir_straight_m));
    }
    for &(a, b) in inp.cut_throughs_m.iter().chain(&inp.forced_stretches_m) {
        forced.push((a, SeamKind::CutThroughEnd));
        forced.push((b, SeamKind::CutThroughEnd));
        excluded.push((a, b));
    }
    for &(a, b) in &inp.frozen_m {
        forced.push((a, SeamKind::FrozenEnd));
        forced.push((b, SeamKind::FrozenEnd));
        excluded.push((a, b));
    }
    let rules = SeamRules {
        step_m: cfg.seam_step_m,
        max_offset_m: cfg.seam_max_offset_m,
        scales_m: cfg.seam_scales_m.clone(),
        window_min_m: cfg.window_min_m,
        window_max_m: cfg.window_max_m,
        window_target_m: cfg.window_target_m,
        r_min_m: cfg.lattice.r_min_m,
        edge_clear_m: 0.5 * cfg.bottom_width_m,
    };
    let seams = find_seams(
        line,
        inp.river,
        inp.river_index,
        inp.centreline,
        &forced,
        &excluded,
        &rules,
    );
    for s in seams.iter().filter(|s| s.kind == SeamKind::ForcedOffRiver) {
        warnings.push(format!(
            "the seam at {:.0} m stands where no seam in the band ({:.0} to {:.0} m) hugs the river \
             on a straight (the line's radius there is {:.0} m)",
            s.at_m, cfg.window_min_m, cfg.window_max_m, s.quality_m
        ));
    }
    let in_band = |a: f64, b: f64, list: &[(f64, f64)]| {
        list.iter().any(|&(x, y)| x - 1e-6 <= a && b <= y + 1e-6)
    };
    let jobs: Vec<Job> = seams
        .windows(2)
        .enumerate()
        .map(|(k, w)| Job {
            id: k,
            pass: 1,
            a: w[0].at_m,
            b: w[1].at_m,
            start_seam: w[0],
            end_seam: w[1],
            level_lo: w[0].at_m,
            level_hi: w[1].at_m,
            frozen: in_band(w[0].at_m, w[1].at_m, &inp.frozen_m),
            cut_throughs: inp
                .cut_throughs_m
                .iter()
                .copied()
                .filter(|&(x, y)| w[0].at_m - 1e-6 <= x && y <= w[1].at_m + 1e-6)
                .collect(),
        })
        .collect();
    let per_window_mb = cfg.lattice.max_states.min(1usize << 40) as f64 * STATE_BYTES / 1.0e6;
    let in_flight = ((cfg.memory_budget_mb / per_window_mb.max(1.0)).floor() as usize)
        .clamp(1, cfg.threads.max(1));
    if in_flight < cfg.threads {
        warnings.push(format!(
            "{in_flight} window(s) at a time, not {}: a window may hold {} states (about {:.0} MB) and \
             the memory budget is {:.0} MB",
            cfg.threads, cfg.lattice.max_states, per_window_mb, cfg.memory_budget_mb
        ));
    }
    let (line1, mut recs, spans1) =
        run_pass(line, &jobs, cfg, inp, &project, in_flight, &mut warnings);
    let mut result = line1;
    let mut cancelled = (inp.cancel)();

    // Pass 2: a window over each free seam, from the middle of the window before it to the middle
    // of the window after it, on the first pass's line.
    if cfg.second_pass && jobs.len() >= 2 && !cancelled {
        let mids: Vec<Option<Seam>> = spans1
            .iter()
            .map(|&(a, b)| {
                let mid = 0.5 * (a + b);
                let found = admissible_near(
                    &result,
                    inp.river,
                    inp.river_index,
                    inp.centreline,
                    &[],
                    mid,
                    0.25 * (b - a),
                    &rules,
                );
                Some(found.unwrap_or(Seam {
                    at_m: mid,
                    kind: SeamKind::Staggered,
                    quality_m: f64::NAN,
                    offset_m: f64::NAN,
                }))
            })
            .collect();
        let mut jobs2 = Vec::new();
        for k in 0..jobs.len() - 1 {
            let seam = jobs[k].end_seam;
            if seam.kind.forced() || jobs[k].frozen || jobs[k + 1].frozen {
                continue;
            }
            let (Some(m0), Some(m1)) = (mids[k], mids[k + 1]) else {
                continue;
            };
            if m0.quality_m.is_nan() {
                warnings.push(format!(
                    "the second pass's seam at {:.0} m is the middle of a window: no seam there hugs \
                     the river on a straight",
                    m0.at_m
                ));
            }
            jobs2.push(Job {
                id: jobs.len() + jobs2.len(),
                pass: 2,
                a: m0.at_m,
                b: m1.at_m,
                start_seam: m0,
                end_seam: m1,
                level_lo: jobs[k].level_lo,
                level_hi: jobs[k + 1].level_hi,
                frozen: false,
                cut_throughs: Vec::new(),
            });
        }
        if !jobs2.is_empty() {
            let (line2, recs2, _) = run_pass(
                &result,
                &jobs2,
                cfg,
                inp,
                &project,
                in_flight,
                &mut warnings,
            );
            result = line2;
            recs.extend(recs2);
        }
        cancelled = (inp.cancel)();
    }

    // The end-to-end measure on the ground given, and the checks of the joined line.
    let level_by_point = |p: Pt| (inp.level)(project.project(p));
    let cutter = SectionCutter::new(
        inp.river_index,
        cfg.section.depth_m,
        &cfg.section.model,
        cfg.section.wet_slope,
        cfg.section.maxw_m,
        cfg.section.dx_m,
        cfg.section.daylight_run_m,
    )?;
    let measure = LineMeasure::new(
        cutter,
        inp.terrain,
        inp.rule,
        LevelAt::ByPoint(&level_by_point),
        cfg.weights,
        &cfg.section,
        cfg.station_m,
    );
    let start_total = measure.measure(line, Frame::Volumes);
    let result_total = measure.measure(&result, Frame::Volumes);
    let window_saving_sum: f64 = recs
        .iter()
        .map(|r| match r.adopted {
            Adopted::Found => {
                r.full_start.cost - r.full_found.map_or(r.full_start.cost, |m| m.cost)
            }
            Adopted::PolishedStart => {
                r.full_start.cost - r.full_start_polished.map_or(r.full_start.cost, |m| m.cost)
            }
            Adopted::None => 0.0,
        })
        .sum();
    let end_to_end_saving = start_total.cost - result_total.cost;
    let totals_agree =
        (window_saving_sum - end_to_end_saving).abs() <= 0.005 * start_total.cost.abs().max(1e-9);
    if !totals_agree {
        warnings.push(format!(
            "the windows' savings add up to {window_saving_sum:.0} but the line measured end to end \
             saves {end_to_end_saving:.0}: more than 0.5 % of the starting line's cost ({:.0}) apart",
            start_total.cost
        ));
    }
    if result_total.cost > start_total.cost * 1.005 {
        warnings.push(format!(
            "the result costs {:.0} end to end against {:.0} for the starting line",
            result_total.cost, start_total.cost
        ));
    }
    let (min_radius_m, radius_ok) = radius_reading(&result, cfg.lattice.r_min_m);
    let short = short_reversals(
        &result,
        cfg.lattice.r_min_m,
        held_min_straight(&cfg.lattice),
        2.0 * cfg.lattice.pitch_m,
    );
    let _ = len;
    Ok(WindowsResult {
        self_crossing_m: first_self_crossing_m(&result),
        line: result,
        seams,
        windows: recs,
        start_total,
        result_total,
        window_saving_sum,
        end_to_end_saving,
        totals_agree,
        min_radius_m,
        radius_ok,
        short_straights: short,
        windows_in_flight: in_flight,
        seconds: (inp.clock)() - t0,
        cancelled,
        warnings,
    })
}
