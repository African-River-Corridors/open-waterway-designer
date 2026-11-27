//! Finding the windows on a starting line (OPT-5 3.4): forced seams (the line's ends, weir sites,
//! the ends of recorded cut-throughs and frozen reaches), then free seams between them on straights
//! of the line that hug the river, chosen by a maximin dynamic programme.

use crate::geom::overlay::{PolygonIndex, point_polygon_distance};
use crate::geom::{Polygon, Pt, linear};

/// What a seam is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamKind {
    /// An end of the line.
    End,
    /// A weir site.
    Weir,
    /// An end of a recorded cut-through or a forced off-river stretch.
    CutThroughEnd,
    /// An end of a reach the user froze.
    FrozenEnd,
    /// A free seam on a straight that hugs the river.
    Free,
    /// A free seam where no candidate in the band was admissible: placed with the offset, river
    /// and radius tests set aside, and said so.
    ForcedOffRiver,
    /// A seam of the second pass, in the middle of a first-pass window.
    Staggered,
}

impl SeamKind {
    pub fn name(self) -> &'static str {
        match self {
            SeamKind::End => "end",
            SeamKind::Weir => "weir",
            SeamKind::CutThroughEnd => "cut_through_end",
            SeamKind::FrozenEnd => "frozen_end",
            SeamKind::Free => "free",
            SeamKind::ForcedOffRiver => "forced_off_river",
            SeamKind::Staggered => "staggered",
        }
    }

    /// Whether a window may never cross it (both passes keep it).
    pub fn forced(self) -> bool {
        matches!(
            self,
            SeamKind::End | SeamKind::Weir | SeamKind::CutThroughEnd | SeamKind::FrozenEnd
        )
    }
}

/// One seam: where it is on the line, what it is, the line's quality radius there (infinite for a
/// forced seam or a straight), and how far the line is from the river centreline there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seam {
    pub at_m: f64,
    pub kind: SeamKind,
    pub quality_m: f64,
    pub offset_m: f64,
}

/// How free seams are chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct SeamRules {
    pub step_m: f64,
    pub max_offset_m: f64,
    pub scales_m: Vec<f64>,
    pub window_min_m: f64,
    pub window_max_m: f64,
    pub window_target_m: f64,
    pub r_min_m: f64,
    /// With no river centreline: a seam stands at least this far inside the river polygon.
    pub edge_clear_m: f64,
}

/// The three-point (Menger) radius of `a`, `b`, `c`; infinity on a straight.
pub fn menger(a: Pt, b: Pt, c: Pt) -> f64 {
    let x = libm::hypot(b[0] - c[0], b[1] - c[1]);
    let y = libm::hypot(a[0] - c[0], a[1] - c[1]);
    let z = libm::hypot(a[0] - b[0], a[1] - b[1]);
    let area = 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
    if area > 1e-9 {
        x * y * z / (4.0 * area)
    } else {
        f64::INFINITY
    }
}

/// The line's quality at `s`: its least three-point radius over the chords of `scales_m`, each read
/// on the points half a chord either side (a chord past an end is read to the end).
pub fn quality(line: &[Pt], len: f64, s: f64, scales_m: &[f64]) -> f64 {
    let at = |c: f64| linear::interpolate(line, c.clamp(0.0, len)).unwrap_or([f64::NAN; 2]);
    let mid = at(s);
    scales_m
        .iter()
        .map(|&c| menger(at(s - c / 2.0), mid, at(s + c / 2.0)))
        .fold(f64::INFINITY, f64::min)
}

/// One candidate: chainage, quality, offset from the river, and whether it passes every test.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    at: f64,
    q: f64,
    offset: f64,
    admissible: bool,
}

/// Everything a free seam must pass, read every `step_m` along the line.
#[allow(clippy::too_many_arguments)]
fn candidates(
    line: &[Pt],
    river: &[Polygon],
    river_index: &PolygonIndex,
    centreline: Option<&linear::ProjectIndex>,
    centre_line: Option<&[Pt]>,
    excluded: &[(f64, f64)],
    from: f64,
    to: f64,
    rules: &SeamRules,
) -> Vec<Candidate> {
    let len = linear::length(line);
    let mut out = Vec::new();
    let mut k = 1usize;
    loop {
        let s = from + k as f64 * rules.step_m;
        if s >= to - 1e-9 {
            break;
        }
        k += 1;
        if excluded
            .iter()
            .any(|&(a, b)| a - 1e-9 <= s && s <= b + 1e-9)
        {
            continue;
        }
        let p = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
        let q = quality(line, len, s, &rules.scales_m);
        let in_river = river_index.contains_xy(p);
        let offset = match (centreline, centre_line) {
            (Some(ix), Some(cl)) => {
                let c = ix.project(p);
                let on = linear::interpolate(cl, c).unwrap_or([f64::NAN; 2]);
                libm::hypot(p[0] - on[0], p[1] - on[1])
            }
            _ => f64::NAN,
        };
        let near_centre = if offset.is_finite() {
            offset <= rules.max_offset_m
        } else {
            // No centreline: inside the polygon and at least `edge_clear_m` from its edge.
            in_river && edge_distance(river, p) >= rules.edge_clear_m
        };
        let admissible = in_river && near_centre && q >= 4.0 * rules.r_min_m;
        out.push(Candidate {
            at: s,
            q,
            offset,
            admissible,
        });
    }
    out
}

/// The distance from a point inside the river polygon to its nearest edge.
fn edge_distance(river: &[Polygon], p: Pt) -> f64 {
    let mut best = f64::INFINITY;
    for poly in river {
        for ring in core::iter::once(&poly.exterior).chain(&poly.holes) {
            best = best.min(linear::point_line_distance(p, ring));
        }
    }
    let _ = point_polygon_distance;
    best
}

/// The maximin chain from `from` to `to` through `nodes` (sorted by chainage, both ends counted as
/// infinite quality): every gap in the band, the least quality as large as it can be, then the sum
/// of `(gap - target)^2` as small as it can be. `None` when no chain fits the band.
fn chain(nodes: &[Candidate], from: f64, to: f64, rules: &SeamRules) -> Option<Vec<usize>> {
    // Node 0 is `from`, the last is `to`; the candidates between.
    let mut at = vec![from];
    let mut q = vec![f64::INFINITY];
    for c in nodes {
        at.push(c.at);
        q.push(c.q);
    }
    at.push(to);
    q.push(f64::INFINITY);
    let n = at.len();
    let fits = |i: usize, j: usize| {
        let gap = at[j] - at[i];
        gap >= rules.window_min_m - 1e-9 && gap <= rules.window_max_m + 1e-9
    };
    // Phase 1: the best least quality reachable at each node (maximin is exact by DP).
    let mut best = vec![f64::NEG_INFINITY; n];
    best[0] = f64::INFINITY;
    for j in 1..n {
        for i in 0..j {
            if best[i] > f64::NEG_INFINITY && fits(i, j) {
                best[j] = best[j].max(best[i].min(q[j]));
            }
        }
    }
    let qstar = best[n - 1];
    if qstar == f64::NEG_INFINITY {
        return None;
    }
    // Phase 2: among nodes of quality at least Q*, the chain nearest the target length.
    let mut cost = vec![f64::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    cost[0] = 0.0;
    for j in 1..n {
        if q[j] < qstar {
            continue;
        }
        for i in 0..j {
            if cost[i].is_finite() && fits(i, j) {
                let g = at[j] - at[i] - rules.window_target_m;
                let c = cost[i] + g * g;
                if c < cost[j] {
                    cost[j] = c;
                    prev[j] = i;
                }
            }
        }
    }
    if !cost[n - 1].is_finite() {
        return None;
    }
    let mut out = Vec::new();
    let mut j = prev[n - 1];
    while j != 0 && j != usize::MAX {
        out.push(j - 1);
        j = prev[j];
    }
    out.reverse();
    Some(out)
}

/// The seams of the line: the forced ones (each with its kind) and the free ones between them.
#[allow(clippy::too_many_arguments)]
pub fn find_seams(
    line: &[Pt],
    river: &[Polygon],
    river_index: &PolygonIndex,
    centreline: Option<&[Pt]>,
    forced: &[(f64, SeamKind)],
    excluded: &[(f64, f64)],
    rules: &SeamRules,
) -> Vec<Seam> {
    let len = linear::length(line);
    let ix = centreline.map(linear::ProjectIndex::new);
    let mut fixed: Vec<(f64, SeamKind)> = vec![(0.0, SeamKind::End), (len, SeamKind::End)];
    for &(s, k) in forced {
        if s > 1e-6 && s < len - 1e-6 {
            fixed.push((s, k));
        }
    }
    fixed.sort_by(|a, b| a.0.total_cmp(&b.0));
    fixed.dedup_by(|a, b| (a.0 - b.0).abs() <= 1e-6);
    let offset_at = |s: f64| match (&ix, centreline) {
        (Some(ix), Some(cl)) => {
            let p = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
            let on = linear::interpolate(cl, ix.project(p)).unwrap_or([f64::NAN; 2]);
            libm::hypot(p[0] - on[0], p[1] - on[1])
        }
        _ => f64::NAN,
    };
    let mut out = Vec::new();
    for w in fixed.windows(2) {
        let ((a, ka), (b, _)) = (w[0], w[1]);
        out.push(Seam {
            at_m: a,
            kind: ka,
            quality_m: f64::INFINITY,
            offset_m: offset_at(a),
        });
        if b - a <= rules.window_max_m {
            continue;
        }
        let all = candidates(
            line,
            river,
            river_index,
            ix.as_ref(),
            centreline,
            excluded,
            a,
            b,
            rules,
        );
        let strict: Vec<Candidate> = all.iter().copied().filter(|c| c.admissible).collect();
        let (picked, kind_of): (Vec<Candidate>, bool) = match chain(&strict, a, b, rules) {
            Some(ix) => (ix.into_iter().map(|i| strict[i]).collect(), false),
            None => match chain(&all, a, b, rules) {
                Some(ix) => (ix.into_iter().map(|i| all[i]).collect(), true),
                None => {
                    // Nothing in the band anywhere (every candidate excluded): even pieces.
                    let n = ((b - a) / rules.window_target_m).round().max(1.0) as usize;
                    let n = n.max(((b - a) / rules.window_max_m).ceil() as usize);
                    let picked = (1..n)
                        .map(|k| {
                            let s = a + (b - a) * k as f64 / n as f64;
                            Candidate {
                                at: s,
                                q: quality(line, len, s, &rules.scales_m),
                                offset: offset_at(s),
                                admissible: false,
                            }
                        })
                        .collect();
                    (picked, true)
                }
            },
        };
        for c in picked {
            out.push(Seam {
                at_m: c.at,
                kind: if kind_of && !c.admissible {
                    SeamKind::ForcedOffRiver
                } else {
                    SeamKind::Free
                },
                quality_m: c.q,
                offset_m: c.offset,
            });
        }
    }
    let (last, kl) = fixed[fixed.len() - 1];
    out.push(Seam {
        at_m: last,
        kind: kl,
        quality_m: f64::INFINITY,
        offset_m: offset_at(last),
    });
    out
}

/// Whether a free seam could stand at `s` on `line` (pass 2 looks for one near a window's middle).
#[allow(clippy::too_many_arguments)]
pub fn admissible_near(
    line: &[Pt],
    river: &[Polygon],
    river_index: &PolygonIndex,
    centreline: Option<&[Pt]>,
    excluded: &[(f64, f64)],
    target: f64,
    reach_m: f64,
    rules: &SeamRules,
) -> Option<Seam> {
    let ix = centreline.map(linear::ProjectIndex::new);
    let all = candidates(
        line,
        river,
        river_index,
        ix.as_ref(),
        centreline,
        excluded,
        target - reach_m,
        target + reach_m,
        rules,
    );
    all.into_iter()
        .filter(|c| c.admissible)
        .min_by(|a, b| {
            (a.at - target)
                .abs()
                .total_cmp(&(b.at - target).abs())
                .then(a.at.total_cmp(&b.at))
        })
        .map(|c| Seam {
            at_m: c.at,
            kind: SeamKind::Staggered,
            quality_m: c.q,
            offset_m: c.offset,
        })
}
