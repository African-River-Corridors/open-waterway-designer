//! The river-following line (spec OPT-4): a line built from the river first, then moved by cost.
//!
//! 1. [`seed`]: arcs and straights that follow the river centreline as closely as R_min allows.
//! 2. Cut-throughs: short arc-straight-arc paths across a meander's neck, each an explicit choice.
//! 3. The sideways shift: dynamic programming on the offset from the line, the shifted line's own
//!    curvature a hard limit.

pub mod choice;
pub mod cuts;
pub mod elements;
#[doc(hidden)]
pub mod fixtures;
pub mod mask;
pub mod score;
pub mod seed;
pub mod shift;

use crate::geom::Pt;
use choice::{Choice, ChoiceParams, choose};
use cuts::{CutParams, candidates};
use elements::Chain;
use mask::RiverMask;
use score::StationScorer;
use seed::{Seed, SeedParams};
use shift::{Room, Shift, ShiftParams, Station};
use std::collections::BTreeMap;

/// Every setting of the method, each worked out from R_min and the vessel by the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub seed: SeedParams,
    pub cut: CutParams,
    pub choice: ChoiceParams,
    /// The shift; `None`: no shift.
    pub shift: Option<ShiftParams>,
    /// A stretch out of the river at least this long is a forced stretch, m.
    pub off_river_min_m: f64,
    /// The room about a forced stretch the shift may use, m.
    pub forced_room_m: f64,
}

/// What the method made, step by step.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub seed: Seed,
    pub choices: Vec<Choice>,
    /// Forced ids that matched no candidate.
    pub unmatched: Vec<String>,
    /// The seed with the accepted cut-throughs.
    pub after_cuts: Chain,
    /// The shift's stations and result, when it ran.
    pub shift: Option<(Vec<Station>, Shift)>,
    /// The delivered line: exact arcs and straights.
    pub line: Chain,
    /// The rebuild of the shifted line as arcs, when the shift moved the line.
    pub rebuild: Option<Seed>,
    /// The largest gap between the shifted target and the delivered line, m (0 with no shift).
    pub tracking_gap_m: f64,
    /// True when the delivered line follows the shifted line in exact 5 m steps. Never set by
    /// [`run`] now: the arcs are delivered and a gap over the tolerance is warned of, because a
    /// tracked line carries short reversals an engineer would not draw.
    pub tracked: bool,
}

/// The river-following line of `centre`: the seed, the cut-throughs, the shift, the rebuild.
/// `mask` is the river polygon with the in-river margin; `scorer` prices a station; `widening`
/// gives the bend widening at a radius; `force` the cut-throughs forced on or off; `river_at`
/// a point's river chainage (for the ids).
#[allow(clippy::too_many_arguments)]
pub fn run(
    centre: &[Pt],
    mask: Option<&RiverMask>,
    p: &Params,
    scorer: &dyn StationScorer,
    widening: &dyn Fn(f64) -> f64,
    force: &BTreeMap<String, bool>,
    river_at: &dyn Fn(Pt) -> f64,
) -> Outcome {
    let in_water = mask.map(|m| move |q: Pt| m.contains(q));
    let in_water_dyn: Option<&dyn Fn(Pt) -> bool> =
        in_water.as_ref().map(|f| f as &dyn Fn(Pt) -> bool);
    let sd = seed::seed(centre, &p.seed, in_water_dyn);
    let cands = candidates(&sd.chain, &p.cut, river_at);
    let (choices, after_cuts, unmatched) = choose(&sd.chain, cands, scorer, force, &p.choice);
    let mut out = Outcome {
        seed: sd,
        choices,
        unmatched,
        line: after_cuts.clone(),
        after_cuts,
        shift: None,
        rebuild: None,
        tracking_gap_m: 0.0,
        tracked: false,
    };
    let Some(sp) = &p.shift else {
        return out;
    };
    // The rooms: each accepted cut-through, each forced stretch out of the river.
    let mut rooms: Vec<Room> = out
        .choices
        .iter()
        .filter(|c| c.accepted)
        .filter_map(|c| {
            Some(Room {
                from: c.line_from?,
                to: c.line_to?,
                room_m: sp.cut_room_m,
            })
        })
        .collect();
    if let Some(m) = mask {
        for (a, b) in off_river(&out.after_cuts, m, p.off_river_min_m) {
            if !rooms.iter().any(|r| a < r.to && b > r.from) {
                rooms.push(Room {
                    from: a,
                    to: b,
                    room_m: sp.reach_m,
                });
            }
        }
    }
    let st = shift::stations(&out.after_cuts, sp, mask, &rooms);
    if st.len() < 3 {
        return out;
    }
    let t = shift::tables(&st, sp, scorer, widening);
    let mut sh = shift::search(&st, &t, sp, widening);
    if sp.refine {
        shift::refine(&st, &t, sp, &mut sh, widening);
    }
    let moved = sh.offsets.iter().any(|d| d.abs() >= 0.5 * sp.offset_step_m);
    if moved && sh.cost < sh.cost_zero {
        let pts = shift::shifted_points(&out.after_cuts, &st, &sh.offsets, p.seed.step_m);
        let target = seed::target_of(seed::resample(&pts, p.seed.step_m));
        let rp = rebuild_params(&p.seed);
        let rb = seed::seed_of_target(target, &rp, None);
        out.tracking_gap_m = rb.gap_all_m;
        out.line = rb.chain.clone();
        out.rebuild = Some(rb);
    }
    out.shift = Some((st, sh));
    out
}

/// The seed's settings for building the shifted line again as arcs: no smoothing (the shifted
/// line is smooth already), arcs at R_min at least.
fn rebuild_params(p: &SeedParams) -> SeedParams {
    let mut q = p.clone();
    q.presmooth_m = 0.0;
    // Built at R_min itself (spec 4.7): the shifted line keeps to R_dp, above it.
    q.r_clip = p.r_min;
    q.aim = seed::SeedAim::Centreline;
    q
}

/// The runs of `chain` out of the river (with the mask's margin) at least `min_m` long, read every
/// 5 m: `(from, to)` chainage.
pub fn off_river(chain: &Chain, mask: &RiverMask, min_m: f64) -> Vec<(f64, f64)> {
    let total = chain.length();
    let n = ((total / 5.0).ceil() as usize).max(1);
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    let mut last = 0.0;
    for i in 0..=n {
        let s = total * i as f64 / n as f64;
        let q = chain.pose_at(s);
        let outside = !mask.contains([q.0, q.1]);
        match (outside, start) {
            (true, None) => start = Some(s),
            (false, Some(a)) => {
                if last - a >= min_m {
                    out.push((a, last));
                }
                start = None;
            }
            _ => {}
        }
        last = s;
    }
    if let Some(a) = start
        && total - a >= min_m
    {
        out.push((a, total));
    }
    out
}
