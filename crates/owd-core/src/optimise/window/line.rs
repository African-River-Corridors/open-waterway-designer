//! Reading a design line as the window optimiser needs it (OPT-5): its pose at a chainage, what it
//! does either side of a seam, a stretch of it replaced by another line, and the checks of the
//! joined line (R_min, continuity, minimum straight, loops).

use crate::alignment::loops::{cumulative_turn_deg, first_self_crossing_m};
use crate::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use crate::geom::{Geometry, Pt, linear};
use crate::optimise::lattice::Pose;

/// The cumulative length at each vertex.
pub fn cumulative(line: &[Pt]) -> Vec<f64> {
    let mut out = Vec::with_capacity(line.len());
    let mut at = 0.0;
    out.push(0.0);
    for w in line.windows(2) {
        at += libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        out.push(at);
    }
    out
}

/// The index of the segment holding chainage `s` (the first one whose end is at or past it), and
/// skipping segments of no length.
fn segment_at(line: &[Pt], cum: &[f64], s: f64) -> usize {
    let n = line.len() - 1;
    let mut i = match cum.binary_search_by(|c| c.total_cmp(&s)) {
        Ok(i) => i.min(n - 1),
        Err(i) => i.saturating_sub(1).min(n - 1),
    };
    while i + 1 < n && cum[i + 1] - cum[i] <= 1e-12 {
        i += 1;
    }
    i
}

/// The line's pose at chainage `s`: the point there ([`linear::interpolate`]) and the heading of
/// the segment that holds it. A window starts and ends on these poses exactly.
pub fn pose_on(line: &[Pt], s: f64) -> Pose {
    let p = linear::interpolate(line, s).unwrap_or([f64::NAN; 2]);
    let cum = cumulative(line);
    let i = segment_at(line, &cum, s);
    let (a, b) = (line[i], line[i + 1]);
    (p[0], p[1], libm::atan2(b[1] - a[1], b[0] - a[0]))
}

/// The stretch of `line` from `a` to `b` (chainages, `a < b`).
pub fn stretch(line: &[Pt], a: f64, b: f64) -> Vec<Pt> {
    match linear::substring(line, a, b) {
        Geometry::LineString(l) => l,
        Geometry::Point(p) => vec![p, p],
        _ => Vec::new(),
    }
}

/// `line` with its stretch `a..b` replaced by `piece`, which starts at the line's point at `a` and
/// ends at its point at `b` (its last point is put there exactly). Returns the new line and where
/// the piece ends on it.
pub fn splice(line: &[Pt], a: f64, b: f64, piece: &[Pt]) -> (Vec<Pt>, f64) {
    let len = linear::length(line);
    let mut out: Vec<Pt> = if a > 0.0 {
        stretch(line, 0.0, a)
    } else {
        vec![line[0]]
    };
    let at_a = *out.last().expect("a stretch has a point");
    let mut body = piece.to_vec();
    body[0] = at_a;
    let tail: Vec<Pt> = if b < len {
        stretch(line, b, len)
    } else {
        vec![line[line.len() - 1]]
    };
    let at_b = tail[0];
    let last = body.len() - 1;
    body[last] = at_b;
    out.extend_from_slice(&body[1..]);
    let end = linear::length(&out);
    out.extend_from_slice(&tail[1..]);
    (dedup(out), end)
}

/// The line without repeated points.
pub fn dedup(line: Vec<Pt>) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(line.len());
    for p in line {
        if out
            .last()
            .is_none_or(|q| libm::hypot(p[0] - q[0], p[1] - q[1]) > 1e-9)
        {
            out.push(p);
        }
    }
    out
}

/// A curve tighter than this many times R_min counts as a curve in the minimum-straight check and
/// in what a seam inherits; a gentler one counts as straight (OPT-5 3.13).
pub const CURVE_FACTOR: f64 = 4.0;

/// The signed curvature at each inner vertex: the turn there over the mean of its two segments.
fn vertex_curvature(line: &[Pt]) -> Vec<f64> {
    let n = line.len();
    let mut k = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        let (a, b, c) = (line[i - 1], line[i], line[i + 1]);
        let (l1, l2) = (
            libm::hypot(b[0] - a[0], b[1] - a[1]),
            libm::hypot(c[0] - b[0], c[1] - b[1]),
        );
        if l1 <= 1e-9 || l2 <= 1e-9 {
            continue;
        }
        let h1 = libm::atan2(b[1] - a[1], b[0] - a[0]);
        let h2 = libm::atan2(c[1] - b[1], c[0] - b[0]);
        let d = crate::optimise::window::wrap(h2 - h1);
        k[i] = d / (0.5 * (l1 + l2));
    }
    k
}

/// What the line does before chainage `s` (`forward` false) or after it (`forward` true): the
/// direction of the nearest curve tighter than [`CURVE_FACTOR`] x R_min (+1 left, -1 right) and the
/// straight run to it (m). `(0, run)` when none comes within `limit_m`.
pub fn curve_near(line: &[Pt], s: f64, r_min: f64, limit_m: f64, forward: bool) -> (i8, f64) {
    let cum = cumulative(line);
    let k = vertex_curvature(line);
    let kmin = 1.0 / (CURVE_FACTOR * r_min);
    let n = line.len();
    let idx: Vec<usize> = if forward {
        (0..n).filter(|&i| cum[i] > s + 1e-9).collect()
    } else {
        (0..n).rev().filter(|&i| cum[i] < s - 1e-9).collect()
    };
    for i in idx {
        let run = (cum[i] - s).abs();
        if run > limit_m {
            break;
        }
        if k[i].abs() > kmin {
            return (if k[i] > 0.0 { 1 } else { -1 }, run);
        }
    }
    (0, limit_m)
}

/// The straights between opposite curves tighter than [`CURVE_FACTOR`] x R_min that are shorter
/// than `min_m - tol_m`: `(chainage of the straight's start, its length)`.
pub fn short_reversals(line: &[Pt], r_min: f64, min_m: f64, tol_m: f64) -> Vec<(f64, f64)> {
    if min_m <= 0.0 {
        return Vec::new();
    }
    let cum = cumulative(line);
    let k = vertex_curvature(line);
    let kmin = 1.0 / (CURVE_FACTOR * r_min);
    let mut out = Vec::new();
    let mut last: Option<(i8, f64)> = None;
    for i in 0..line.len() {
        if k[i].abs() <= kmin {
            continue;
        }
        let sgn: i8 = if k[i] > 0.0 { 1 } else { -1 };
        if let Some((ls, at)) = last
            && ls == -sgn
        {
            let run = cum[i] - at;
            if run < min_m - tol_m {
                out.push((at, run));
            }
        }
        last = Some((sgn, cum[i]));
    }
    out
}

/// The joined line's checks around a stretch, as the window records them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopCheck {
    /// The line crosses itself (where, m along it).
    pub self_crossing_m: Option<f64>,
    /// The cumulative absolute turn of the window's own line (degrees).
    pub turn_deg: f64,
    /// The window's line length over the window's length on the line it replaces.
    pub length_ratio: f64,
}

impl LoopCheck {
    pub fn read(joined: &[Pt], piece: &[Pt], replaced_len_m: f64) -> Self {
        Self {
            self_crossing_m: first_self_crossing_m(joined),
            turn_deg: cumulative_turn_deg(piece),
            length_ratio: linear::length(piece) / replaced_len_m.max(1e-9),
        }
    }

    pub fn is_loop(&self, turn_limit_deg: f64, length_ratio_limit: f64) -> bool {
        self.self_crossing_m.is_some()
            || self.turn_deg > turn_limit_deg
            || self.length_ratio > length_ratio_limit
    }
}

/// The least radius of a line on OWD's check chord (R_min / 3), and whether it holds R_min.
pub fn radius_reading(line: &[Pt], r_min: f64) -> (f64, bool) {
    let m = min_radius_on_chord(line, default_check_chord(r_min)).unwrap_or(f64::NAN);
    (m, radius_ok(m, r_min))
}

/// The stretch of `line` from `a - pad` to `b + pad`, clipped to the line.
pub fn padded(line: &[Pt], a: f64, b: f64, pad: f64) -> Vec<Pt> {
    let len = linear::length(line);
    stretch(line, (a - pad).max(0.0), (b + pad).min(len))
}

/// The straight the line has through chainage `s` (m): the stretch round `s` with no vertex whose
/// curvature is above 1 / (1000 R)... read as the run of vertices turning less than 1e-4 rad.
pub fn straight_run_through(line: &[Pt], s: f64) -> f64 {
    crate::optimise::lattice::straight_through_m(line, s)
}

/// Whether `found` goes round instead of through any of `cut_throughs` (stretches of `line`): the
/// middle of the cut-through is further than `room_m` from the found line.
pub fn drops_cut_through(
    line: &[Pt],
    cut_throughs: &[(f64, f64)],
    found: &[Pt],
    room_m: f64,
) -> bool {
    cut_throughs.iter().any(|&(a, b)| {
        let mid = linear::interpolate(line, 0.5 * (a + b)).unwrap_or([f64::NAN; 2]);
        linear::point_line_distance(mid, found) > room_m
    })
}
