//! The polish (OPT-5 3.9): the lattice picks the route well and places the line badly, so the line
//! is slid sideways by a smooth offset that keeps its two end poses, to whatever the full measure
//! says is cheapest.
//!
//! The offset is a clamped uniform cubic B-spline of the chainage, one control point per spacing
//! (plus three, at least six), the first two and the last two held at 0: offset and slope are 0 at
//! both ends, so the polished line leaves and meets its neighbours on the same poses. Each free
//! control point is bounded by `max_shift_m`. The objective is the full measure at the polish's
//! stations (the window's [`LineMeasure`], station by station), times a penalty when the line is
//! tighter than R_min plus a margin. The search is a bounded coordinate search (deterministic): each
//! free control point is tried one step either way, the step halves when a sweep finds nothing
//! better, and it stops at `xtol_m` or after `max_iter` sweeps. Five spacings are run; the cheapest
//! is kept. A polished line that does not pass OWD's radius check at R_min (and is not at least as
//! wide as the line it came from) is not used.

use super::line::radius_reading;
use super::scorer::{LineMeasure, SectionCost, gradient_headings};
use super::seams::menger;
use crate::alignment::radius_check::default_check_chord;
use crate::geom::{Pt, linear};
use crate::volumes::sections::CutBuffers;

/// How the polish runs.
#[derive(Debug, Clone, PartialEq)]
pub struct PolishSettings {
    pub spacings_m: Vec<f64>,
    pub max_shift_m: f64,
    pub station_m: f64,
    pub r_min_m: f64,
    pub radius_margin_m: f64,
    pub max_iter: usize,
    pub xtol_m: f64,
    pub ftol: f64,
}

/// What the polish did to one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Polished {
    /// The line used: the polished one when accepted, else the line given.
    pub line: Vec<Pt>,
    pub accepted: bool,
    /// The spacing kept, and the polish measure (no penalty) of each spacing, in order.
    pub spacing_m: f64,
    pub by_spacing: Vec<(f64, f64)>,
    /// `100 x (largest - smallest) / smallest` over the spacings.
    pub spread_pct: f64,
    pub max_shift_m: f64,
    pub min_radius_before_m: f64,
    pub min_radius_after_m: f64,
    pub evaluations: usize,
}

/// A line made dense: points no further apart than `step_m`, every original point kept.
pub fn densify(line: &[Pt], step_m: f64) -> Vec<Pt> {
    let mut out = vec![line[0]];
    for w in line.windows(2) {
        let l = libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let n = std::cmp::max(1, (l / step_m - 1e-9).ceil() as usize);
        for k in 1..=n {
            let t = k as f64 / n as f64;
            out.push(if k == n {
                w[1]
            } else {
                [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ]
            });
        }
    }
    out
}

/// The clamped uniform cubic B-spline of `segments` pieces over `[0, len]`: at `s`, the first
/// control point it reads and the four weights.
fn basis(s: f64, len: f64, segments: usize) -> (usize, [f64; 4]) {
    let h = len / segments as f64;
    let u = (s / h).clamp(0.0, segments as f64);
    let span = (u.floor() as usize).min(segments - 1);
    // Knots: 0,0,0,0,1,2,...,segments-1,segments,segments,segments,segments (in units of h).
    let knot = |i: isize| -> f64 { (i - 3).clamp(0, segments as isize) as f64 };
    let i = span as isize + 3; // the knot span [knot(i), knot(i+1))
    // Cox-de Boor, degree 3, at u.
    let mut n = [0.0f64; 4];
    n[0] = 1.0;
    let mut left = [0.0f64; 4];
    let mut right = [0.0f64; 4];
    for j in 1..=3usize {
        left[j] = u - knot(i + 1 - j as isize);
        right[j] = knot(i + j as isize) - u;
        let mut saved = 0.0;
        for r in 0..j {
            let den = right[r + 1] + left[j - r];
            let temp = if den.abs() > 0.0 { n[r] / den } else { 0.0 };
            n[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        n[j] = saved;
    }
    (span, n)
}

/// The offset at each of `at` (chainages) for control points `c`.
fn offsets(c: &[f64], bases: &[(usize, [f64; 4])]) -> Vec<f64> {
    bases
        .iter()
        .map(|(span, w)| (0..4).map(|k| w[k] * c[span + k]).sum())
        .collect()
}

/// The polish's objective for one spacing, with its station cache.
struct Objective<'a> {
    measure: &'a LineMeasure<'a>,
    q: Vec<Pt>,
    normals: Vec<Pt>,
    bases: Vec<(usize, [f64; 4])>,
    k_chord: usize,
    r_pen: f64,
    allowed: Option<&'a dyn Fn(f64, f64) -> bool>,
    /// Each station's inputs (bits of x, y, heading, half-width) and its cost.
    cache: Vec<Option<([u64; 4], SectionCost)>>,
    buf: CutBuffers,
    evaluations: usize,
}

impl Objective<'_> {
    /// The polish measure (no penalty) and the penalised objective of control points `c`.
    fn eval(&mut self, c: &[f64]) -> (f64, f64) {
        self.evaluations += 1;
        let d = offsets(c, &self.bases);
        let p: Vec<Pt> = (0..self.q.len())
            .map(|i| {
                [
                    self.q[i][0] + d[i] * self.normals[i][0],
                    self.q[i][1] + d[i] * self.normals[i][1],
                ]
            })
            .collect();
        if let Some(ok) = self.allowed
            && !p.iter().all(|pt| ok(pt[0], pt[1]))
        {
            return (f64::INFINITY, f64::INFINITY);
        }
        let th = gradient_headings(&p);
        let hb = self.measure.rule().along(&p);
        let n = p.len();
        let seg: Vec<f64> = p
            .windows(2)
            .map(|w| libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .collect();
        let mut total = 0.0;
        for i in 0..n {
            let key = [
                p[i][0].to_bits(),
                p[i][1].to_bits(),
                th[i].to_bits(),
                hb[i].to_bits(),
            ];
            let cost = match &self.cache[i] {
                Some((k, c)) if *k == key => c.cost,
                _ => {
                    let c = self.measure.station(p[i], th[i], hb[i], &mut self.buf);
                    self.cache[i] = Some((key, c));
                    c.cost
                }
            };
            let ds =
                0.5 * (if i > 0 { seg[i - 1] } else { 0.0 } + if i + 1 < n { seg[i] } else { 0.0 });
            total += cost * ds;
        }
        let w = self.measure.weights();
        if w.length != 0.0 {
            total += w.length * seg.iter().sum::<f64>();
        }
        let k = self.k_chord;
        let mut r = f64::INFINITY;
        for i in k..n.saturating_sub(k) {
            r = r.min(menger(p[i - k], p[i], p[i + k]));
        }
        let pen = if r < self.r_pen {
            1.0 + 4.0 * (self.r_pen - r) / self.r_pen
        } else {
            1.0
        };
        (total, total * pen)
    }
}

/// The polished line of `base` for control points `c` of `segments` pieces, on `base`'s own dense
/// points.
fn polished_line(dense: &[Pt], c: &[f64], segments: usize) -> Vec<Pt> {
    let len = linear::length(dense);
    let cum = super::line::cumulative(dense);
    let th = gradient_headings(dense);
    let bases: Vec<(usize, [f64; 4])> = cum.iter().map(|&s| basis(s, len, segments)).collect();
    let d = offsets(c, &bases);
    let n = dense.len();
    (0..n)
        .map(|i| {
            if i == 0 || i == n - 1 {
                dense[i]
            } else {
                let (nx, ny) = (-libm::sin(th[i]), libm::cos(th[i]));
                [dense[i][0] + d[i] * nx, dense[i][1] + d[i] * ny]
            }
        })
        .collect()
}

/// Polish `line` (a window's line, its two ends on the seams) on `measure`. `allowed` holds every
/// station of a trial line to the corridor and the permitted ground.
pub fn polish(
    line: &[Pt],
    measure: &LineMeasure,
    allowed: Option<&dyn Fn(f64, f64) -> bool>,
    set: &PolishSettings,
) -> Polished {
    let (r_before, _) = radius_reading(line, set.r_min_m);
    let unpolished = |by: Vec<(f64, f64)>, evals: usize| Polished {
        line: line.to_vec(),
        accepted: false,
        spacing_m: f64::NAN,
        by_spacing: by,
        spread_pct: 0.0,
        max_shift_m: 0.0,
        min_radius_before_m: r_before,
        min_radius_after_m: r_before,
        evaluations: evals,
    };
    let len = linear::length(line);
    let n_st = std::cmp::max(2, (len / set.station_m - 1e-9).ceil() as usize);
    if len <= 1e-6 || set.spacings_m.is_empty() {
        return unpolished(Vec::new(), 0);
    }
    let h = len / n_st as f64;
    let dense = densify(line, 2.0);
    let q: Vec<Pt> = (0..=n_st)
        .map(|i| linear::interpolate(&dense, i as f64 * h).unwrap_or([f64::NAN; 2]))
        .collect();
    let normals: Vec<Pt> = gradient_headings(&q)
        .into_iter()
        .map(|t| [-libm::sin(t), libm::cos(t)])
        .collect();
    let k_chord = std::cmp::max(
        1,
        libm::rint(0.5 * default_check_chord(set.r_min_m) / h) as usize,
    );
    let r_pen = set.r_min_m + set.radius_margin_m;
    let mut best: Option<(f64, f64, Vec<f64>, usize, f64)> = None;
    let mut by = Vec::new();
    let mut evals = 0usize;
    for &spacing in &set.spacings_m {
        let segments = std::cmp::max(3, (len / spacing - 1e-9).ceil() as usize);
        let nc = segments + 3;
        let mut obj = Objective {
            measure,
            q: q.clone(),
            normals: normals.clone(),
            bases: (0..=n_st)
                .map(|i| basis(i as f64 * h, len, segments))
                .collect(),
            k_chord,
            r_pen,
            allowed,
            cache: vec![None; n_st + 1],
            buf: CutBuffers::default(),
            evaluations: 0,
        };
        let mut c = vec![0.0; nc];
        let (mut plain, mut f) = obj.eval(&c);
        let free: Vec<usize> = (2..nc - 2).collect();
        let mut step = 0.5 * set.max_shift_m;
        for _ in 0..set.max_iter {
            let before = f;
            for &j in &free {
                for dir in [1.0, -1.0] {
                    let trial = (c[j] + dir * step).clamp(-set.max_shift_m, set.max_shift_m);
                    if trial == c[j] {
                        continue;
                    }
                    let old = c[j];
                    c[j] = trial;
                    let (tp, tf) = obj.eval(&c);
                    if tf < f {
                        (plain, f) = (tp, tf);
                        break;
                    }
                    c[j] = old;
                }
            }
            let gained = before - f;
            if gained.partial_cmp(&(set.ftol * f.abs())) != Some(core::cmp::Ordering::Greater) {
                step *= 0.5;
                if step < set.xtol_m {
                    break;
                }
            }
        }
        evals += obj.evaluations;
        by.push((spacing, plain));
        if best.as_ref().is_none_or(|b| f < b.1) {
            best = Some((plain, f, c, segments, spacing));
        }
    }
    let Some((plain, _, c, segments, spacing)) = best else {
        return unpolished(by, evals);
    };
    let finite: Vec<f64> = by.iter().map(|b| b.1).filter(|v| v.is_finite()).collect();
    let lo = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let spread_pct = if lo.is_finite() && lo > 0.0 {
        100.0 * (hi - lo) / lo
    } else {
        0.0
    };
    let out = polished_line(&dense, &c, segments);
    let (r_after, ok_after) = radius_reading(&out, set.r_min_m);
    let inside = allowed.is_none_or(|ok| out.iter().all(|p| ok(p[0], p[1])));
    let accept = plain.is_finite() && inside && (ok_after || r_after >= r_before);
    if !accept {
        let mut u = unpolished(by, evals);
        u.min_radius_after_m = r_after;
        u.spread_pct = spread_pct;
        return u;
    }
    let max_shift = c.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    Polished {
        line: out,
        accepted: true,
        spacing_m: spacing,
        by_spacing: by,
        spread_pct,
        max_shift_m: max_shift,
        min_radius_before_m: r_before,
        min_radius_after_m: r_after,
        evaluations: evals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Label: property (the clamped basis sums to one and is 1 at each end on the end point).
    #[test]
    fn the_clamped_basis_is_a_partition_of_unity_with_fixed_ends() {
        for segments in [3usize, 5, 12] {
            for k in 0..=100 {
                let s = 1000.0 * k as f64 / 100.0;
                let (span, w) = basis(s, 1000.0, segments);
                let sum: f64 = w.iter().sum();
                assert!((sum - 1.0).abs() < 1e-12, "{segments} {s} {sum}");
                assert!(span + 3 < segments + 3);
            }
            let (s0, w0) = basis(0.0, 1000.0, segments);
            assert_eq!((s0, w0[0]), (0, 1.0));
            let (s1, w1) = basis(1000.0, 1000.0, segments);
            assert_eq!(s1, segments - 1);
            assert!((w1[3] - 1.0).abs() < 1e-12);
        }
    }

    /// Label: property (the first two and the last two control points held at 0 give an offset and
    /// a slope of 0 at both ends).
    #[test]
    fn end_controls_at_zero_hold_offset_and_slope() {
        let segments = 6;
        let mut c = vec![0.0; segments + 3];
        for (j, v) in c.iter_mut().enumerate().take(segments + 1).skip(2) {
            *v = libm::sin(j as f64) * 10.0;
        }
        let d = |s: f64| {
            let (span, w) = basis(s, 600.0, segments);
            (0..4).map(|k| w[k] * c[span + k]).sum::<f64>()
        };
        assert_eq!(d(0.0), 0.0);
        assert!(d(600.0).abs() < 1e-12);
        assert!((d(1e-4) - d(0.0)).abs() / 1e-4 < 1e-3);
        assert!((d(600.0) - d(600.0 - 1e-4)).abs() / 1e-4 < 1e-3);
    }
}
