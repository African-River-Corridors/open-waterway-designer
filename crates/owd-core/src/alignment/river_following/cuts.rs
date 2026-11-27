//! Meander cut-throughs (spec OPT-4, section 3): short arc-straight-arc paths across a loop's
//! neck, each an explicit choice with a stable id, the length it saves, and the stretch of river it
//! replaces.
//!
//! The generator walks stations along the seed. From each, for each length band of seed stretch, it
//! tries the shortest arc-straight-arc path (OWD's own closing path, `lattice::closing_path`) to
//! each station in the band, at the cut-through radius. A path is a candidate when it saves enough
//! length, is short enough against the stretch it replaces, turns no arc too far, keeps the
//! minimum straight at both joins and crosses no other part of the line. Per start and band the
//! best ratio is kept, then only a ratio that is the best within four stations either way.

use super::elements::{Chain, Pose};
use crate::geom::{Pt, linear};
use crate::optimise::lattice::{GoalAfter, closing_path};

/// The generator's settings, each from R_min and the vessel.
#[derive(Debug, Clone, PartialEq)]
pub struct CutParams {
    /// The radius of the cut's arcs, m.
    pub r_cut: f64,
    /// Further radii tried at each site (multiples of `r_cut`).
    pub wider: Vec<f64>,
    /// A cut saves at least this length, m.
    pub min_save_m: f64,
    /// The seed stretch is at least this many times the cut.
    pub min_ratio: f64,
    /// Each arc turns at most this, rad.
    pub max_arc_turn: f64,
    /// The station step, m.
    pub station_m: f64,
    /// The bands of seed stretch length, m.
    pub bands: Vec<(f64, f64)>,
    /// The straight between opposite arcs either tighter than `tight_reverse_r`, m.
    pub min_straight_m: f64,
    pub tight_reverse_r: f64,
}

impl CutParams {
    /// The settings at R_min `r_min` (spec 3.1): stations every R_min / 7, the bands at
    /// `[0.7, 1.1, 2.5, 4.4, 7.2, 11.1] × R_min`, the wider radii 1.5 and 2 times the cut's.
    pub fn scaled(
        r_min: f64,
        r_cut: f64,
        min_save_m: f64,
        min_ratio: f64,
        max_arc_turn_deg: f64,
        min_straight_m: f64,
    ) -> CutParams {
        let edges = [0.7, 1.1, 2.5, 4.4, 7.2, 11.1];
        CutParams {
            r_cut,
            wider: vec![1.5, 2.0],
            min_save_m,
            min_ratio,
            max_arc_turn: max_arc_turn_deg.to_radians(),
            station_m: r_min / 7.0,
            bands: edges
                .windows(2)
                .map(|w| (w[0] * r_min, w[1] * r_min))
                .collect(),
            min_straight_m,
            tight_reverse_r: 2.0 * r_min,
        }
    }
}

/// One way to cut a site: a radius and the path's three pieces `(curvature, length)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CutOption {
    pub radius_m: f64,
    pub pieces: Vec<(f64, f64)>,
    pub length_m: f64,
    /// The path's points (arcs every 2 m), for drawing.
    pub line: Vec<Pt>,
}

/// A cut-through candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// A stable id from the river chainage of its two ends, to 100 m: `cut-AAAAA-BBBBB` in
    /// hectometres.
    pub id: String,
    /// Line chainage where it leaves the line and where it joins it again.
    pub s_from: f64,
    pub s_to: f64,
    /// River chainage of the two ends.
    pub river_from: f64,
    pub river_to: f64,
    /// The line's stretch it replaces, m.
    pub stay_length_m: f64,
    /// The options: at `r_cut` first, then the wider radii that also make a candidate.
    pub options: Vec<CutOption>,
}

impl Candidate {
    /// The option at the cut-through radius.
    pub fn first(&self) -> &CutOption {
        &self.options[0]
    }

    /// The length saved by option `k`, m.
    pub fn saved_m(&self, k: usize) -> f64 {
        self.stay_length_m - self.options[k].length_m
    }

    /// The stretch's length over the cut's, for option `k`.
    pub fn ratio(&self, k: usize) -> f64 {
        self.stay_length_m / self.options[k].length_m
    }

    /// True when it shares line with `other`.
    pub fn overlaps(&self, other: &Candidate) -> bool {
        self.s_from < other.s_to && other.s_from < self.s_to
    }
}

/// The hand of the last arc before chainage `s` and the straight run between it and `s`.
fn arriving(chain: &Chain, s: f64) -> (i8, f64) {
    let i = chain.element_at(s);
    let e = &chain.elements[i];
    if e.is_arc() {
        return (e.hand(), 0.0);
    }
    let mut run = s - e.s0;
    for p in chain.elements[..i].iter().rev() {
        if p.is_arc() {
            return (p.hand(), run);
        }
        run += p.length;
    }
    (0, run)
}

/// The hand of the first arc after chainage `s` and the straight run from `s` to it.
fn leaving(chain: &Chain, s: f64) -> (i8, f64) {
    let i = chain.element_at(s);
    let e = &chain.elements[i];
    if e.is_arc() {
        return (e.hand(), 0.0);
    }
    let mut run = e.s1() - s;
    for p in &chain.elements[i + 1..] {
        if p.is_arc() {
            return (p.hand(), run);
        }
        run += p.length;
    }
    (0, run)
}

/// The pieces of `chain` with `[a, b]` replaced by `cut` (which runs from the pose at `a` to the
/// pose at `b`).
pub fn splice(chain: &Chain, a: f64, b: f64, cut: &[(f64, f64)]) -> Chain {
    let mut pieces = chain.slice(0.0, a).1;
    pieces.extend_from_slice(cut);
    pieces.extend(chain.slice(b, chain.length()).1);
    Chain::from_pieces(chain.start(), &pieces)
}

/// True when the joins of a cut from `a` to `b` keep the minimum straight: the line round each join
/// (one minimum straight and a little either side) has no short reversal.
fn joins_hold(chain: &Chain, a: f64, b: f64, cut: &[(f64, f64)], p: &CutParams) -> bool {
    let w = p.min_straight_m + 1.0;
    let (start, mut pieces) = chain.slice((a - w).max(0.0), a);
    pieces.extend_from_slice(cut);
    pieces.extend(chain.slice(b, (b + w).min(chain.length())).1);
    Chain::from_pieces(start, &pieces)
        .short_reversals(p.tight_reverse_r, p.min_straight_m)
        .is_empty()
}

/// The pieces `(curvature, length)` of a closing path, with its length; `None` when no path keeps
/// the rules.
fn path(
    a: Pose,
    b: Pose,
    r: f64,
    arriving: (i8, f64),
    leaving: (i8, f64),
    p: &CutParams,
) -> Option<(Vec<(f64, f64)>, f64)> {
    let (insign, run) = if arriving.1 >= p.min_straight_m {
        (0, 0.0)
    } else {
        arriving
    };
    let after = if leaving.1 >= p.min_straight_m {
        GoalAfter::default()
    } else {
        GoalAfter {
            turn: leaving.0,
            run_m: leaving.1,
        }
    };
    let pieces = closing_path(a, b, r, insign, run, p.min_straight_m, after)?;
    let mut out = Vec::with_capacity(3);
    let mut len = 0.0;
    for (rr, sgn, l) in pieces {
        if sgn != 0 && l / rr > p.max_arc_turn + 1e-9 {
            return None;
        }
        out.push((if sgn == 0 { 0.0 } else { f64::from(sgn) / rr }, l));
        len += l;
    }
    Some((out, len))
}

fn points_of(start: Pose, pieces: &[(f64, f64)]) -> Vec<Pt> {
    Chain::from_pieces(start, pieces).points()
}

/// True when the cut's points cross the line outside the stretch it replaces (within a few
/// kilometres of it).
fn crosses_line(chain: &Chain, a: f64, b: f64, cut: &[Pt], reach: f64) -> bool {
    let before = Chain::from_pieces(chain.pose_at((a - reach).max(0.0)), &{
        chain.slice((a - reach).max(0.0), (a - 1.0).max(0.0)).1
    })
    .points();
    let after = Chain::from_pieces(chain.pose_at(b + 1.0), &{
        chain.slice(b + 1.0, (b + reach).min(chain.length())).1
    })
    .points();
    let inner = if cut.len() > 2 {
        &cut[1..cut.len() - 1]
    } else {
        cut
    };
    [before, after]
        .iter()
        .filter(|l| l.len() >= 2)
        .any(|l| !linear::line_intersection(inner, l).0.is_empty())
}

/// A start, an end, the ratio, the pieces and the length of a path found.
type Found = (usize, usize, f64, Vec<(f64, f64)>, f64);
/// The best end of a band so far: the end, the ratio, the pieces and the length.
type Top = (usize, f64, Vec<(f64, f64)>, f64);

/// The cut-through candidates of `chain`. `river_at` gives the river chainage of a point (for the
/// ids).
pub fn candidates(chain: &Chain, p: &CutParams, river_at: &dyn Fn(Pt) -> f64) -> Vec<Candidate> {
    let total = chain.length();
    if total < 2.0 * p.station_m {
        return Vec::new();
    }
    let st = chain.stations(p.station_m);
    let h = total / (st.len() - 1) as f64;
    let arr: Vec<(i8, f64)> = st.iter().map(|x| arriving(chain, x.0)).collect();
    let lea: Vec<(i8, f64)> = st.iter().map(|x| leaving(chain, x.0)).collect();
    // Per start and band, the end with the best ratio.
    let mut best: Vec<Found> = Vec::new();
    for i in 0..st.len() {
        let (si, pi, _) = st[i];
        for &(lo, hi) in &p.bands {
            let j0 = i + ((lo / h).ceil() as usize).max(1);
            let j1 = (i + (hi / h).floor() as usize).min(st.len() - 1);
            let mut top: Option<Top> = None;
            for j in j0..=j1 {
                if j >= st.len() {
                    break;
                }
                let (sj, pj, _) = st[j];
                let stay = sj - si;
                let d = libm::hypot(pj.0 - pi.0, pj.1 - pi.1);
                if stay < p.min_ratio * d || stay - d < p.min_save_m {
                    continue;
                }
                let Some((pieces, len)) = path(pi, pj, p.r_cut, arr[i], lea[j], p) else {
                    continue;
                };
                let ratio = stay / len;
                if stay - len < p.min_save_m || ratio < p.min_ratio {
                    continue;
                }
                if top.as_ref().is_some_and(|t| t.1 >= ratio) {
                    continue;
                }
                if !joins_hold(chain, si, sj, &pieces, p) {
                    continue;
                }
                top = Some((j, ratio, pieces, len));
            }
            if let Some((j, ratio, pieces, len)) = top {
                best.push((i, j, ratio, pieces, len));
            }
        }
    }
    // Only a ratio that is the best within four stations either way.
    // `best` is in start order: look only at the starts within four stations.
    let mut keep: Vec<&Found> = best
        .iter()
        .filter(|c| {
            let lo = best.partition_point(|o| o.0 + 4 < c.0);
            !best[lo..]
                .iter()
                .take_while(|o| o.0 <= c.0 + 4)
                .any(|o| o.2 > c.2 || (o.2 == c.2 && o.0 < c.0))
        })
        .collect();
    keep.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut out: Vec<Candidate> = Vec::new();
    for &(i, j, _, ref pieces, len) in keep {
        let (si, pi, _) = st[i];
        let (sj, pj, _) = st[j];
        let line = points_of(pi, pieces);
        if crosses_line(chain, si, sj, &line, 4.0 * (sj - si)) {
            continue;
        }
        let mut options = vec![CutOption {
            radius_m: p.r_cut,
            pieces: pieces.clone(),
            length_m: len,
            line,
        }];
        for f in &p.wider {
            let r = f * p.r_cut;
            if let Some((pc, l)) = path(pi, pj, r, arr[i], lea[j], p)
                && sj - si - l >= p.min_save_m
                && joins_hold(chain, si, sj, &pc, p)
            {
                let line = points_of(pi, &pc);
                if !crosses_line(chain, si, sj, &line, 4.0 * (sj - si)) {
                    options.push(CutOption {
                        radius_m: r,
                        pieces: pc,
                        length_m: l,
                        line,
                    });
                }
            }
        }
        let (ra, rb) = (river_at([pi.0, pi.1]), river_at([pj.0, pj.1]));
        let id = format!(
            "cut-{:05}-{:05}",
            (ra.min(rb) / 100.0).round().max(0.0) as i64,
            (ra.max(rb) / 100.0).round().max(0.0) as i64
        );
        let c = Candidate {
            id,
            s_from: si,
            s_to: sj,
            river_from: ra.min(rb),
            river_to: ra.max(rb),
            stay_length_m: sj - si,
            options,
        };
        // Two candidates with one id: the better ratio stays.
        match out.iter_mut().find(|o| o.id == c.id) {
            Some(o) if o.ratio(0) >= c.ratio(0) => {}
            Some(o) => *o = c,
            None => out.push(c),
        }
    }
    out
}
