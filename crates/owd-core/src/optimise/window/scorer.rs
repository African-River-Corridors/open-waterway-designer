//! One scorer (OPT-5, requirement 1): the window optimiser's search, its polish and its adopt rule
//! all price a line with the volumes stage's own section.
//!
//! - **One section function.** Every cross-section is cut by
//!   [`crate::cut_model::section_area_weighted`]: the bench gate, the berm, the face, the daylight
//!   limit of model B and the area sum. The search ([`MoveScorer`]) calls it directly; the line
//!   measure ([`LineMeasure`]) calls it through [`SectionCutter`], the walk's own path.
//! - **One ground.** The search and the measure of a window read the window's [`TerrainGrid`] (the
//!   ground sampled once, with the bed model under the river when the design has one).
//! - **One rule for which cut counts.** Cut counts at every sample where the ground stands above
//!   the face, wet or dry, as the volumes stage counts it.
//! - **Bend widening in the section.** A move on an arc of radius R is cut at the bottom half-width
//!   for R ([`HalfWidthRule::at_radius`]); a line is read for its radius station by station, as the
//!   volumes stage reads it ([`HalfWidthRule::along`]). Both go through
//!   [`crate::cut_model::widened_hb_at_radius`].
//! - **One set of weights** ([`Weights`]): wet, dry, land and length, applied the same way
//!   everywhere. The plain m3 are always given beside the weighted cost.
//! - **Additive.** A move's sections each stand for their share of its length; a line's stations each
//!   stand for the station step. The search's sampling (`ds_m`, `dx_m`) is a setting of the same
//!   function, not a second function.
//! - **Unknown ground** is no cut, counts as land for the land cost, and a move with less than 60 %
//!   of its bottom samples on known ground is refused.
//! - **No footprint correction.**
//!
//! **What is cached, and why it is exact.** (1) The half-width of every radius the search lays (each
//! primitive's, R_min's and the straight's), worked out once by the one rule: the same function of
//! the same radius gives the same number. (2) The offsets and the side of each sample across a
//! section, once per scorer. (3) The water level of the window when it is the same at every point
//! of it (a pool), else read at each section. (4) Scratch buffers, so a section allocates nothing.
//! (5) In the polish, each station's cost is kept with the exact bits of its inputs (place, heading,
//! half-width, level) and cut again only when one of them changes. Nothing is cached by lattice
//! state: two paths into one state reach it from different true poses, so the cost of a move from
//! there is cut again from the true pose.

use crate::cut_model::{local_radii, normalise_model, section_area_weighted, widened_hb_at_radius};
use crate::geom::Pt;
use crate::optimise::lattice::{EdgeCost, LatticeConfig, Pose, Prim, ReverseDijkstra, TerrainGrid};
use crate::types::Refusal;
use crate::volumes::sections::{CutBuffers, SectionCutter, StationAt, arange, station_frame};
use core::cell::RefCell;

/// The bottom half-width of the channel at a radius: the straight half-width, widened in a bend by
/// the sizing stage's rule (half the whole widening per side), none at or above `rmax_m`.
pub struct HalfWidthRule {
    pub straight_hb_m: f64,
    pub rmax_m: f64,
    widening: Box<dyn Fn(f64) -> f64 + Sync + Send>,
}

impl HalfWidthRule {
    /// The rule with `widening(r)` the whole bend widening at radius `r` (its floor applied).
    pub fn new(
        straight_hb_m: f64,
        rmax_m: f64,
        widening: impl Fn(f64) -> f64 + Sync + Send + 'static,
    ) -> Self {
        Self {
            straight_hb_m,
            rmax_m,
            widening: Box::new(widening),
        }
    }

    /// One half-width everywhere (the volumes setting `half_width_m`).
    pub fn fixed(hb_m: f64) -> Self {
        Self::new(hb_m, 0.0, |_| 0.0)
    }

    /// The half-width on an arc of radius `r` (infinite: a straight).
    pub fn at_radius(&self, r: f64) -> f64 {
        widened_hb_at_radius(r, self.straight_hb_m, self.rmax_m, &self.widening)
    }

    /// The half-width at each station point of a line, as the volumes stage reads it: the radius by
    /// three points two stations apart and a rolling mean ([`local_radii`]), then the rule.
    pub fn along(&self, xy: &[Pt]) -> Vec<f64> {
        let xs: Vec<f64> = xy.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = xy.iter().map(|p| p[1]).collect();
        local_radii(&xs, &ys, 2)
            .into_iter()
            .map(|r| self.at_radius(r))
            .collect()
    }
}

/// The objective's weights: per m3 of cut in the river (`wet`) and out of it (`dry`), per m2 of
/// footprint at the water surface outside the river polygon (`land`), per metre of line (`length`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub wet: f64,
    pub dry: f64,
    pub land: f64,
    pub length: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            wet: 1.0,
            dry: 1.0,
            land: 0.0,
            length: 0.0,
        }
    }
}

/// The water level over a window: one number (a pool), or read at each point.
#[derive(Clone, Copy)]
pub enum LevelAt<'a> {
    Fixed(f64),
    ByPoint(&'a (dyn Fn(Pt) -> f64 + Sync)),
}

impl LevelAt<'_> {
    pub fn at(&self, p: Pt) -> f64 {
        match self {
            LevelAt::Fixed(v) => *v,
            LevelAt::ByPoint(f) => f(p),
        }
    }
}

/// The section every cross-section of the window is cut to: the cut model, the depth, the slope
/// below water, the reach and step across, and the daylight run of model B.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionSettings {
    pub model: String,
    pub depth_m: f64,
    pub wet_slope: f64,
    pub maxw_m: f64,
    pub dx_m: f64,
    pub daylight_run_m: Option<f64>,
}

/// The cost of one cross-section, per metre of line: weighted, and the plain parts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SectionCost {
    /// The weighted cost of one metre of line here (cut and land; not length).
    pub cost: f64,
    pub dredge_m2: f64,
    pub excav_m2: f64,
    /// Footprint outside the river polygon, m2 per metre of line.
    pub land_m: f64,
    pub max_face_h_m: f64,
    /// Samples with no ground, and samples in all.
    pub unknown: usize,
    pub samples: usize,
}

/// The weighted cost of a section from its areas and its land.
fn weigh(w: &Weights, dredge: f64, excav: f64, land_m: f64) -> f64 {
    let mut c = w.wet * dredge + w.dry * excav;
    if w.land != 0.0 {
        c += w.land * land_m;
    }
    c
}

// =====================================================================================
// The search's scorer
// =====================================================================================

/// The window search's cost of a move: the volumes stage's section swept along the primitive.
pub struct MoveScorer<'a> {
    grid: &'a TerrainGrid,
    sec: SectionSettings,
    offs: Vec<f64>,
    sidewhich: Vec<i8>,
    ds_m: f64,
    level: LevelAt<'a>,
    weights: Weights,
    /// The half-width of every radius the search lays, worked out once: `(radius bits, hb)`.
    hb_by_r: Vec<(u64, f64)>,
    rule: &'a HalfWidthRule,
    mask: Option<&'a dyn Fn(f64, f64) -> bool>,
    min_valid_fraction: f64,
    scratch: RefCell<(Vec<f64>, Vec<bool>, Vec<bool>)>,
}

impl<'a> MoveScorer<'a> {
    /// The scorer of a window. `radii` are the radii the search lays (each primitive's and R_min);
    /// their half-widths are worked out now. `ds_m` is the step between sections along a move.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        grid: &'a TerrainGrid,
        sec: &SectionSettings,
        ds_m: f64,
        level: LevelAt<'a>,
        weights: Weights,
        rule: &'a HalfWidthRule,
        radii: &[f64],
    ) -> Result<Self, Refusal> {
        let model = normalise_model(&sec.model);
        if model != crate::cut_model::CUT_MODEL && model != crate::cut_model::CUT_MODEL_B {
            return Err(format!("unknown cut model {}; use 'a' or 'b'", sec.model));
        }
        if !(sec.dx_m > 0.0 && sec.maxw_m > 0.0 && ds_m > 0.0) {
            return Err("the search's sampling steps must be above 0".into());
        }
        let offs = arange(-sec.maxw_m, sec.maxw_m + 0.1 * sec.dx_m, sec.dx_m);
        let sidewhich = offs.iter().map(|&o| if o < 0.0 { -1 } else { 1 }).collect();
        let mut hb_by_r: Vec<(u64, f64)> = Vec::new();
        for &r in radii.iter().chain([f64::INFINITY].iter()) {
            if !hb_by_r.iter().any(|(b, _)| *b == r.to_bits()) {
                hb_by_r.push((r.to_bits(), rule.at_radius(r)));
            }
        }
        let m = offs.len();
        Ok(Self {
            grid,
            sec: SectionSettings {
                model,
                ..sec.clone()
            },
            offs,
            sidewhich,
            ds_m,
            level,
            weights,
            hb_by_r,
            rule,
            mask: None,
            min_valid_fraction: 0.6,
            scratch: RefCell::new((vec![0.0; m], vec![false; m], vec![false; m])),
        })
    }

    /// Hold every point of every move to `allowed` (the permitted ground, OPT-5 3.16).
    pub fn with_mask(mut self, allowed: &'a dyn Fn(f64, f64) -> bool) -> Self {
        self.mask = Some(allowed);
        self
    }

    /// The length weight: what each metre of a move pays besides its cut and land.
    pub fn length_weight(&self) -> f64 {
        self.weights.length
    }

    /// The half-width at radius `r`: from the cache, else from the rule.
    pub fn hb(&self, r: f64) -> f64 {
        self.hb_by_r
            .iter()
            .find(|(b, _)| *b == r.to_bits())
            .map_or_else(|| self.rule.at_radius(r), |(_, h)| *h)
    }

    /// The cost of one cross-section at `(x, y)`, square to `heading`, with bottom half-width `hb`,
    /// and how many of its bottom samples are on known ground (of how many).
    pub fn section(&self, x: f64, y: f64, heading: f64, hb: f64) -> (SectionCost, usize, usize) {
        let mut guard = self.scratch.borrow_mut();
        let (z, inside, wet) = &mut *guard;
        let (nx, ny) = (-libm::sin(heading), libm::cos(heading));
        let (mut fair, mut fair_known, mut unknown) = (0usize, 0usize, 0usize);
        let land_half = hb + self.sec.wet_slope * self.sec.depth_m;
        let mut land_n = 0usize;
        for (k, &o) in self.offs.iter().enumerate() {
            let (gx, gy) = (x + o * nx, y + o * ny);
            let zk = self.grid.sample_ground_at(gx, gy);
            let ins = self.grid.sample_wet_at(gx, gy);
            z[k] = zk;
            inside[k] = ins;
            wet[k] = zk.is_finite() && ins;
            if !zk.is_finite() {
                unknown += 1;
            }
            if o.abs() <= hb {
                fair += 1;
                if zk.is_finite() {
                    fair_known += 1;
                }
            }
            if o.abs() <= land_half && !ins {
                land_n += 1;
            }
        }
        let wl = self.level.at([x, y]);
        let grade = wl - self.sec.depth_m;
        let a = section_area_weighted(
            z,
            &self.offs,
            &self.sidewhich,
            hb,
            grade,
            wl,
            self.sec.depth_m,
            &self.sec.model,
            wet,
            self.sec.wet_slope,
            self.sec.dx_m,
            self.sec.daylight_run_m,
            None,
        )
        .expect("the model was checked when the scorer was made");
        let land_m = land_n as f64 * self.sec.dx_m;
        (
            SectionCost {
                cost: weigh(&self.weights, a.dredge, a.excav, land_m),
                dredge_m2: a.dredge,
                excav_m2: a.excav,
                land_m,
                max_face_h_m: a.max_face_h,
                unknown,
                samples: self.offs.len(),
            },
            fair_known,
            fair,
        )
    }

    /// The sections of a primitive laid from `pose`: `(x, y, heading)` of each, every `ds_m` along
    /// it (the last at its end), and the length each stands for.
    pub fn sections_of(&self, prim: &Prim, pose: Pose) -> (Vec<Pose>, f64) {
        let n = std::cmp::max(1, (prim.arc_len / self.ds_m - 1e-9).ceil() as usize);
        let ds = prim.arc_len / n as f64;
        let (ct, st) = (libm::cos(pose.2), libm::sin(pose.2));
        let k = if prim.r.is_finite() && prim.sign != 0 {
            f64::from(prim.sign) / prim.r
        } else {
            0.0
        };
        let out = (0..n)
            .map(|i| {
                // Each section at the middle of the length it stands for.
                let s = ds * (i as f64 + 0.5);
                let (lx, ly, lth) = if k == 0.0 {
                    (s, 0.0, 0.0)
                } else {
                    (libm::sin(k * s) / k, (1.0 - libm::cos(k * s)) / k, k * s)
                };
                (
                    pose.0 + ct * lx - st * ly,
                    pose.1 + st * lx + ct * ly,
                    pose.2 + lth,
                )
            })
            .collect();
        (out, ds)
    }

    /// The plain parts of a move: weighted cost (with length), dredge m3, dry cut m3, land m2, its
    /// tallest face, and whether it may be laid.
    pub fn move_parts(&self, prim: &Prim, pose: Pose) -> (SectionCost, f64, bool) {
        let hb = self.hb(prim.r);
        let (secs, ds) = self.sections_of(prim, pose);
        let (mut total, mut known, mut fair) = (SectionCost::default(), 0usize, 0usize);
        for &(x, y, th) in &secs {
            let (c, k, f) = self.section(x, y, th, hb);
            total.cost += c.cost * ds;
            total.dredge_m2 += c.dredge_m2 * ds;
            total.excav_m2 += c.excav_m2 * ds;
            total.land_m += c.land_m * ds;
            total.max_face_h_m = total.max_face_h_m.max(c.max_face_h_m);
            total.unknown += c.unknown;
            total.samples += c.samples;
            known += k;
            fair += f;
        }
        if self.weights.length != 0.0 {
            total.cost += self.weights.length * prim.arc_len;
        }
        let ok = fair > 0 && (known as f64 / fair as f64) >= self.min_valid_fraction;
        (total, ds, ok)
    }
}

impl EdgeCost for MoveScorer<'_> {
    fn score(&self, prim: &Prim, pose: Pose) -> (f64, bool) {
        let (c, _, ok) = self.move_parts(prim, pose);
        if ok {
            (c.cost, true)
        } else {
            (f64::INFINITY, false)
        }
    }

    fn land_area(&self, prim: &Prim, pose: Pose) -> Option<f64> {
        (self.weights.land != 0.0).then(|| self.move_parts(prim, pose).0.land_m)
    }

    /// Every metre of every move pays the length weight, and cut and land are never below 0: the
    /// length weight is a lower bound of the cost of any metre, so `c_min` with it is admissible
    /// (and exact, unlike a sampled cheapest metre).
    fn c_min_per_m(&self, _grid: &TerrainGrid, _cfg: &LatticeConfig) -> f64 {
        self.weights.length
    }

    fn reverse_heuristic(
        &self,
        _: &TerrainGrid,
        _: Pt,
        _: &LatticeConfig,
    ) -> Option<ReverseDijkstra> {
        None
    }

    fn allowed(&self, x: f64, y: f64) -> bool {
        self.mask.is_none_or(|m| m(x, y))
    }

    fn has_mask(&self) -> bool {
        self.mask.is_some()
    }
}

// =====================================================================================
// The measure of a line
// =====================================================================================

/// Where the stations of a line stand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// As the volumes stage stands them: `floor(L / step) + 1` stations spread evenly from end to
    /// end, each standing for `step` ([`station_frame`]).
    Volumes,
    /// Every `step` from the line's first point (the last at or before its end), each standing for
    /// `step`, so two lines that share their start share their stations there.
    FromStart,
}

/// The measure of a line or a stretch: weighted cost and the plain parts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Measure {
    /// The objective: weighted cut, land and length.
    pub cost: f64,
    pub dredge_m3: f64,
    pub excav_m3: f64,
    pub land_m2: f64,
    pub length_m: f64,
    pub max_face_h_m: f64,
    pub stations: usize,
    /// Share of the section samples with no ground (0 to 1).
    pub unknown_share: f64,
}

impl Measure {
    pub fn total_m3(&self) -> f64 {
        self.dredge_m3 + self.excav_m3
    }
}

/// The full measure (OPT-5): the volumes stage's walk of a line, cut by the walk's own section
/// ([`SectionCutter`]), widened by the line's own radius ([`HalfWidthRule::along`]), plus the land
/// and the length the weights price.
pub struct LineMeasure<'a> {
    cutter: SectionCutter<'a>,
    terrain: &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>,
    rule: &'a HalfWidthRule,
    level: LevelAt<'a>,
    weights: Weights,
    sec: SectionSettings,
    pub station_m: f64,
}

impl<'a> LineMeasure<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cutter: SectionCutter<'a>,
        terrain: &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>,
        rule: &'a HalfWidthRule,
        level: LevelAt<'a>,
        weights: Weights,
        sec: &SectionSettings,
        station_m: f64,
    ) -> Self {
        Self {
            cutter,
            terrain,
            rule,
            level,
            weights,
            sec: sec.clone(),
            station_m,
        }
    }

    pub fn weights(&self) -> Weights {
        self.weights
    }

    pub fn rule(&self) -> &HalfWidthRule {
        self.rule
    }

    /// The cost of one station: its section at `xy`, square to `heading`, half-width `hb`, per metre
    /// of line.
    pub fn station(&self, xy: Pt, heading: f64, hb: f64, buf: &mut CutBuffers) -> SectionCost {
        let m = self.cutter.offsets().len();
        let (mut ax, mut ay) = (Vec::with_capacity(m), Vec::with_capacity(m));
        self.cutter.sample_points(xy, heading, &mut ax, &mut ay);
        let z = (self.terrain)(&ax, &ay);
        let st = StationAt {
            chainage_m: 0.0,
            xy,
            heading_rad: heading,
            hb,
            wl: self.level.at(xy),
        };
        let a = self
            .cutter
            .areas(&st, &ax, &ay, &z, None, buf)
            .expect("the cutter's model was checked when it was made");
        let land_half = hb + self.sec.wet_slope * self.sec.depth_m;
        let land_n = self
            .cutter
            .offsets()
            .iter()
            .zip(&buf.inside)
            .filter(|(o, ins)| o.abs() <= land_half && !**ins)
            .count();
        let land_m = land_n as f64 * self.sec.dx_m;
        SectionCost {
            cost: weigh(&self.weights, a.areas.dredge, a.areas.excav, land_m),
            dredge_m2: a.areas.dredge,
            excav_m2: a.areas.excav,
            land_m,
            max_face_h_m: a.areas.max_face_h,
            unknown: buf.z.iter().filter(|z| !z.is_finite()).count(),
            samples: m,
        }
    }

    /// The stations of `line` in `frame`: points and headings (central differences of the station
    /// points, one-sided at the ends, as the volumes stage reads them).
    pub fn frame(&self, line: &[Pt], frame: Frame) -> Option<(Vec<Pt>, Vec<f64>)> {
        match frame {
            Frame::Volumes => station_frame(line, self.station_m)
                .ok()
                .map(|(_, xy, th)| (xy, th)),
            Frame::FromStart => {
                let len = crate::geom::linear::length(line);
                let n = (len / self.station_m + 1e-9).floor() as usize + 1;
                if n < 2 {
                    return None;
                }
                let xy: Vec<Pt> = (0..n)
                    .map(|i| {
                        crate::geom::linear::interpolate(line, i as f64 * self.station_m)
                            .unwrap_or([f64::NAN; 2])
                    })
                    .collect();
                Some((xy.clone(), gradient_headings(&xy)))
            }
        }
    }

    /// The full measure of `line` with its stations in `frame`.
    pub fn measure(&self, line: &[Pt], frame: Frame) -> Measure {
        let length_m = crate::geom::linear::length(line);
        let Some((xy, th)) = self.frame(line, frame) else {
            return Measure {
                cost: self.weights.length * length_m,
                length_m,
                ..Measure::default()
            };
        };
        let hb = self.rule.along(&xy);
        let mut buf = CutBuffers::default();
        let mut out = Measure {
            length_m,
            stations: xy.len(),
            ..Measure::default()
        };
        let (mut unknown, mut samples) = (0usize, 0usize);
        // Plain sums station by station, in order, each times the step: the volumes stage's sum.
        let (mut cost, mut dr, mut ex, mut land) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..xy.len() {
            let c = self.station(xy[i], th[i], hb[i], &mut buf);
            cost += c.cost;
            dr += c.dredge_m2;
            ex += c.excav_m2;
            land += c.land_m;
            out.max_face_h_m = out.max_face_h_m.max(c.max_face_h_m);
            unknown += c.unknown;
            samples += c.samples;
        }
        out.cost = cost * self.station_m;
        out.dredge_m3 = dr * self.station_m;
        out.excav_m3 = ex * self.station_m;
        out.land_m2 = land * self.station_m;
        if self.weights.length != 0.0 {
            out.cost += self.weights.length * length_m;
        }
        out.unknown_share = if samples == 0 {
            0.0
        } else {
            unknown as f64 / samples as f64
        };
        out
    }
}

/// Headings of a run of points by central differences, one-sided at the ends (`np.gradient`).
pub fn gradient_headings(xy: &[Pt]) -> Vec<f64> {
    let m = xy.len();
    (0..m)
        .map(|i| {
            let (a, b, d) = if m < 2 {
                return 0.0;
            } else if i == 0 {
                (xy[0], xy[1], 1.0)
            } else if i == m - 1 {
                (xy[m - 2], xy[m - 1], 1.0)
            } else {
                (xy[i - 1], xy[i + 1], 2.0)
            };
            libm::atan2((b[1] - a[1]) / d, (b[0] - a[0]) / d)
        })
        .collect()
}
