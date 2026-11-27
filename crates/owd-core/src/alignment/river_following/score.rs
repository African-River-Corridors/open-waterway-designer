//! The cost of a stretch of line, from one scorer (spec OPT-4, 3.2 and 4.6; OPT-5, 2.2).
//!
//! The river-following line does not price a section itself. It asks a [`StationScorer`] for the
//! section at a station of the line (its position, heading and curvature, so bend widening is in
//! the section), and sums stations along a stretch. With a ground model the scorer is
//! [`OneScorer`]: the volumes stage's own section through OPT-5's one scorer
//! ([`LineMeasure::station`]). [`PlainScorer`] is a plain double for the tests, and the plan-area
//! price when there is no ground model (it then prices land only and cuts nothing).

use super::elements::{Chain, Pose};
use super::mask::RiverMask;
use crate::geom::Pt;
use crate::optimise::window::scorer::LineMeasure;
use crate::volumes::sections::CutBuffers;
use core::cell::RefCell;

/// The section of the channel at one station, per metre of line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Section {
    /// Cut below the water level (dredge), m² (m³ per metre).
    pub wet_m2: f64,
    /// Cut above it (dry), m².
    pub dry_m2: f64,
    /// The weighted cost per metre of line: the user's weights on wet and dry cut, land and
    /// length, as the scorer applies them.
    pub cost_per_m: f64,
    /// The width of the channel's water surface outside the river polygon, m (land per metre).
    pub land_m: f64,
    /// The tallest cut face above the water level, m.
    pub face_m: f64,
}

/// One scorer: the section of the channel at a station.
pub trait StationScorer {
    /// True when the scorer reads a ground model (a DEM); false when it prices plan area only.
    fn has_ground(&self) -> bool;
    /// The section at `at`, the line heading `heading` (rad) with curvature `k` (1/m, + left).
    fn section(&self, at: Pt, heading: f64, k: f64) -> Section;
}

/// The sums over a stretch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Totals {
    pub length_m: f64,
    /// Dredge (wet) cut, m³.
    pub wet_m3: f64,
    /// Dry cut, m³.
    pub dry_m3: f64,
    /// The weighted cost.
    pub cost: f64,
    /// Land: water surface outside the river polygon, m².
    pub land_m2: f64,
    /// The tallest face, m.
    pub face_max_m: f64,
    /// Stations whose face is over the face limit.
    pub stations_over_face: usize,
}

impl Totals {
    /// Plain total: wet and dry cut, m³.
    pub fn plain_m3(&self) -> f64 {
        self.wet_m3 + self.dry_m3
    }
}

/// The totals of `chain` from chainage `a` to `b`, a station every `step` metres at most, each
/// standing for its share of the stretch (half a step at the two ends).
pub fn totals(
    chain: &Chain,
    a: f64,
    b: f64,
    step: f64,
    scorer: &dyn StationScorer,
    face_limit: f64,
) -> Totals {
    let len = (b - a).max(0.0);
    let n = ((len / step).ceil() as usize).max(1);
    let h = len / n as f64;
    let mut t = Totals {
        length_m: len,
        ..Totals::default()
    };
    for i in 0..=n {
        let s = a + h * i as f64;
        let w = if i == 0 || i == n { 0.5 * h } else { h };
        let p: Pose = chain.pose_at(s);
        // At a join, the curvature of the element the station is in.
        let k = chain.curvature_at(s);
        add(&mut t, scorer.section([p.0, p.1], p.2, k), w, face_limit);
    }
    t
}

fn add(t: &mut Totals, x: Section, w: f64, face_limit: f64) {
    t.wet_m3 += x.wet_m2 * w;
    t.dry_m3 += x.dry_m2 * w;
    t.cost += x.cost_per_m * w;
    t.land_m2 += x.land_m * w;
    if x.face_m > t.face_max_m {
        t.face_max_m = x.face_m;
    }
    if x.face_m > face_limit {
        t.stations_over_face += 1;
    }
}

/// A plain scorer for tests and for a design with no ground model: a trapezoid from the bed up to
/// the ground at the line's centre, the bottom widened on a bend by `widening(radius)`.
pub struct PlainScorer<'a> {
    /// The ground level at a point; `None` for plan area only (no ground model).
    pub ground: Option<&'a dyn Fn(Pt) -> f64>,
    /// The water level at a point, m.
    pub water: &'a dyn Fn(Pt) -> f64,
    pub depth_m: f64,
    /// The bottom width on a straight, m.
    pub bottom_m: f64,
    /// Horizontal per vertical.
    pub side_slope: f64,
    /// The bend widening at a radius, m.
    pub widening: &'a dyn Fn(f64) -> f64,
    /// The river, for the land the channel's water surface takes outside it.
    pub river: Option<&'a RiverMask>,
    pub wet_weight: f64,
    pub dry_weight: f64,
    pub land_weight: f64,
}

impl StationScorer for PlainScorer<'_> {
    fn has_ground(&self) -> bool {
        self.ground.is_some()
    }

    fn section(&self, at: Pt, heading: f64, k: f64) -> Section {
        let b = self.bottom_m
            + if k != 0.0 {
                (self.widening)(1.0 / k.abs())
            } else {
                0.0
            };
        let top = b + 2.0 * self.side_slope * self.depth_m;
        // The water surface outside the river: the width of the surface whose edge is out of it.
        let land_m = match self.river {
            None => 0.0,
            Some(m) => {
                let n = [-libm::sin(heading), libm::cos(heading)];
                let steps = 8;
                let out = (0..=steps)
                    .filter(|i| {
                        let o = -top / 2.0 + top * *i as f64 / steps as f64;
                        !m.contains([at[0] + o * n[0], at[1] + o * n[1]])
                    })
                    .count();
                top * out as f64 / (steps + 1) as f64
            }
        };
        let Some(g) = self.ground else {
            return Section {
                land_m,
                cost_per_m: self.land_weight * land_m,
                ..Section::default()
            };
        };
        // The section across the line: the template is the bed over the bottom width,
        // then the side slope up; cut is the ground above it, wet below the water level, dry above.
        let level = (self.water)(at);
        let bed = level - self.depth_m;
        let n = [-libm::sin(heading), libm::cos(heading)];
        let reach = b / 2.0 + self.side_slope * (self.depth_m + 60.0);
        // Every two metres across: a test double, not the volumes stage's section.
        let steps = reach.ceil() as i64;
        let dx = 2.0 * reach / steps as f64;
        let (mut wet, mut dry, mut face) = (0.0, 0.0, 0.0f64);
        for i in 0..=steps {
            let o = -reach + i as f64 * dx;
            let z = g([at[0] + o * n[0], at[1] + o * n[1]]);
            if !z.is_finite() {
                continue;
            }
            let t = bed + (o.abs() - b / 2.0).max(0.0) / self.side_slope;
            if z <= t {
                continue;
            }
            wet += (z.min(level) - t).max(0.0) * dx;
            dry += (z - t.max(level)).max(0.0) * dx;
            face = face.max(z - level);
        }
        Section {
            wet_m2: wet,
            dry_m2: dry,
            cost_per_m: self.wet_weight * wet + self.dry_weight * dry + self.land_weight * land_m,
            land_m,
            face_m: face.max(0.0),
        }
    }
}

/// The one scorer (OPT-5 part 1) as a [`StationScorer`]: the volumes stage's own section, through
/// OPT-5's [`LineMeasure::station`], at the bottom half-width the sizing rule gives the station's
/// curvature ([`HalfWidthRule::at_radius`]). Not a second scorer: a station of the same measure.
pub struct OneScorer<'a> {
    measure: &'a LineMeasure<'a>,
    buf: RefCell<CutBuffers>,
}

impl<'a> OneScorer<'a> {
    pub fn new(measure: &'a LineMeasure<'a>) -> Self {
        OneScorer {
            measure,
            buf: RefCell::new(CutBuffers::default()),
        }
    }
}

impl StationScorer for OneScorer<'_> {
    fn has_ground(&self) -> bool {
        true
    }

    fn section(&self, at: Pt, heading: f64, k: f64) -> Section {
        let r = if k != 0.0 {
            1.0 / k.abs()
        } else {
            f64::INFINITY
        };
        let hb = self.measure.rule().at_radius(r);
        let c = self
            .measure
            .station(at, heading, hb, &mut self.buf.borrow_mut());
        Section {
            wet_m2: c.dredge_m2,
            dry_m2: c.excav_m2,
            cost_per_m: c.cost + self.measure.weights().length,
            land_m: c.land_m,
            face_m: c.max_face_h_m,
        }
    }
}
