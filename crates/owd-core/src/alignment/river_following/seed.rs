//! The seed (spec OPT-4, section 2): circular arcs and straights that follow the river centreline as
//! closely as R_min allows. Tangent at every join, no arc tighter than `r_clip`, a straight of at
//! least `min_straight_m` between two opposite arcs when either is tighter than `tight_reverse_r`.
//!
//! The construction is a circle per bend, not a relaxed polyline tracked in 5 m steps (spec 2.4
//! allows either when the tests hold):
//!
//! 1. The target is the centreline resampled, smoothed by a moving average (ends held) and
//!    resampled again. Its curvature comes from its heading.
//! 2. Each run of the target whose curvature keeps one hand above `curve_threshold` and turns at
//!    least `min_turn_rad` is a bend. Each bend gets the circle that fits its points best (least
//!    squares on the distance to the circle); a circle tighter than `r_clip` is fitted again at
//!    `r_clip` exactly, and the bend is *tight* (the river is tighter than the seed may be).
//! 3. The line is the chain of common tangents: from the start point to the first circle, circle
//!    to circle (the outer tangent for two bends of one hand, the inner for opposite hands), to the
//!    end point. Each element's geometry is fixed in absolute coordinates, so a long river does
//!    not drift.
//! 4. Two opposite circles closer than the minimum straight allows are pushed apart until it
//!    fits. Two circles of one hand with no tangent, or an arc that would wrap, are merged.
//! 5. Where the line is still more than `tolerance_m` from the target outside a tight bend, the
//!    bend there is split in two (or a new bend is fitted to a straight that misses), and the line
//!    is built again. A change that does not bring the line nearer is undone.

use super::elements::{Chain, Pose, walk_distances};
use crate::geom::Pt;
use core::f64::consts::{PI, TAU};

/// What the seed aims at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeedAim {
    /// The river centreline: the least squared distance to it (the default).
    #[default]
    Centreline,
    /// The river's water: at a tight bend, the arc of radius `r_clip` placed to keep the most of
    /// it inside the river polygon (shrunk by the in-river margin).
    InWater,
}

/// The seed's settings, each scaled from R_min and the vessel by [`SeedParams::scaled`].
#[derive(Debug, Clone, PartialEq)]
pub struct SeedParams {
    /// The minimum bend radius, m.
    pub r_min: f64,
    /// The seed's least radius, m (`seed_clip_ratio × R_min`).
    pub r_clip: f64,
    /// The straight between opposite arcs, m.
    pub min_straight_m: f64,
    /// Opposite arcs need the straight when either is tighter than this, m (`2 R_min`).
    pub tight_reverse_r: f64,
    /// The largest gap from the target the seed accepts outside a tight bend, m.
    pub tolerance_m: f64,
    /// The moving average before the fit, m.
    pub presmooth_m: f64,
    /// The target's sampling step, m.
    pub step_m: f64,
    /// The curvature of a bend, at least, 1/m (`1 / (8 R_min)`).
    pub curve_threshold: f64,
    /// The turn of a bend, at least, rad.
    pub min_turn_rad: f64,
    /// Refinement passes, at most.
    pub max_passes: usize,
    /// What the seed aims at.
    pub aim: SeedAim,
}

impl SeedParams {
    /// The settings at R_min `r_min`, a vessel `loa_m` long: every length from those two.
    pub fn scaled(r_min: f64, clip_ratio: f64, min_straight_m: f64, tolerance_m: f64) -> Self {
        SeedParams {
            r_min,
            r_clip: clip_ratio * r_min,
            min_straight_m,
            tight_reverse_r: 2.0 * r_min,
            tolerance_m,
            presmooth_m: r_min / 6.0,
            step_m: (r_min / 60.0).clamp(1.0, 5.0),
            curve_threshold: 1.0 / (8.0 * r_min),
            min_turn_rad: 3f64.to_radians(),
            max_passes: 12,
            aim: SeedAim::Centreline,
        }
    }
}

/// The line the seed fits: the centreline resampled and smoothed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Target {
    pub pts: Vec<Pt>,
    /// Chainage along the target.
    pub s: Vec<f64>,
    /// Heading, unwrapped.
    pub heading: Vec<f64>,
    /// Curvature, + left.
    pub kappa: Vec<f64>,
}

/// `line` resampled at the step nearest `step` that divides its length.
pub fn resample(line: &[Pt], step: f64) -> Vec<Pt> {
    if line.len() < 2 {
        return line.to_vec();
    }
    let mut cum = vec![0.0];
    for w in line.windows(2) {
        let d = libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        cum.push(cum[cum.len() - 1] + d);
    }
    let total = cum[cum.len() - 1];
    let n = ((total / step).ceil() as usize).max(1);
    let mut out = Vec::with_capacity(n + 1);
    let mut j = 0;
    for i in 0..=n {
        let t = total * i as f64 / n as f64;
        while j + 2 < cum.len() && cum[j + 1] < t {
            j += 1;
        }
        let seg = cum[j + 1] - cum[j];
        let f = if seg > 0.0 {
            ((t - cum[j]) / seg).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push([
            line[j][0] + f * (line[j + 1][0] - line[j][0]),
            line[j][1] + f * (line[j + 1][1] - line[j][1]),
        ]);
    }
    out
}

/// A moving average `half` points either side; near the ends the window shrinks so that it stays
/// centred, so the two end points are held.
fn moving_average(pts: &[Pt], half: usize) -> Vec<Pt> {
    let n = pts.len();
    let mut cx = vec![0.0; n + 1];
    let mut cy = vec![0.0; n + 1];
    for (i, p) in pts.iter().enumerate() {
        cx[i + 1] = cx[i] + p[0];
        cy[i + 1] = cy[i] + p[1];
    }
    (0..n)
        .map(|i| {
            let h = half.min(i).min(n - 1 - i);
            let (a, b) = (i - h, i + h + 1);
            let m = (b - a) as f64;
            [(cx[b] - cx[a]) / m, (cy[b] - cy[a]) / m]
        })
        .collect()
}

/// The target of a centreline.
pub fn target(centreline: &[Pt], p: &SeedParams) -> Target {
    let raw = resample(centreline, p.step_m);
    let half = (p.presmooth_m / 2.0 / p.step_m).round() as usize;
    let smooth = resample(&moving_average(&raw, half), p.step_m);
    target_of(smooth)
}

/// The target of points already resampled evenly.
pub fn target_of(pts: Vec<Pt>) -> Target {
    let n = pts.len();
    let mut s = vec![0.0; n];
    for i in 1..n {
        s[i] = s[i - 1] + libm::hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
    }
    let mut heading = vec![0.0; n];
    for i in 0..n {
        let (a, b) = (i.saturating_sub(1), (i + 1).min(n.saturating_sub(1)));
        heading[i] = libm::atan2(pts[b][1] - pts[a][1], pts[b][0] - pts[a][0]);
        if i > 0 {
            while heading[i] - heading[i - 1] > PI {
                heading[i] -= TAU;
            }
            while heading[i] - heading[i - 1] < -PI {
                heading[i] += TAU;
            }
        }
    }
    let step = if n > 1 {
        s[n - 1] / (n - 1) as f64
    } else {
        1.0
    };
    // Curvature over about 20 m, at least one point either side.
    let m = ((10.0 / step).round() as usize).max(1);
    let kappa = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(m), (i + m).min(n.saturating_sub(1)));
            if b > a && s[b] > s[a] {
                (heading[b] - heading[a]) / (s[b] - s[a])
            } else {
                0.0
            }
        })
        .collect();
    Target {
        pts,
        s,
        heading,
        kappa,
    }
}

/// The circle that fits `pts` best: the algebraic fit, then Gauss-Newton on the distance to the
/// circle. `None` for points on a line.
pub fn fit_circle(pts: &[Pt]) -> Option<(Pt, f64)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = (
        pts.iter().map(|p| p[0]).sum::<f64>() / n,
        pts.iter().map(|p| p[1]).sum::<f64>() / n,
    );
    // Kasa: x² + y² + D x + E y + F = 0, centred on the mean.
    let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut sz = 0.0;
    for p in pts {
        let (x, y) = (p[0] - mx, p[1] - my);
        let z = x * x + y * y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sxz += x * z;
        syz += y * z;
        sz += z;
    }
    let det = sxx * syy - sxy * sxy;
    if det.abs() < 1e-12 * (sxx * syy).max(1e-300) {
        return None;
    }
    // Solve [sxx sxy; sxy syy] [a b] = [sxz syz] / 2 for the centre (a, b) about the mean.
    let a = (syy * sxz - sxy * syz) / (2.0 * det);
    let b = (sxx * syz - sxy * sxz) / (2.0 * det);
    let r2 = a * a + b * b + sz / n;
    if !(r2.is_finite() && r2 > 0.0) {
        return None;
    }
    let mut c = [a + mx, b + my];
    let mut r = r2.sqrt();
    // Gauss-Newton on (cx, cy, r).
    for _ in 0..20 {
        let mut jtj = [[0.0; 3]; 3];
        let mut jtf = [0.0; 3];
        for p in pts {
            let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
            let d = libm::hypot(dx, dy).max(1e-12);
            let f = d - r;
            let j = [-dx / d, -dy / d, -1.0];
            for u in 0..3 {
                jtf[u] += j[u] * f;
                for v in 0..3 {
                    jtj[u][v] += j[u] * j[v];
                }
            }
        }
        let Some(dlt) = solve3(jtj, jtf) else { break };
        c[0] -= dlt[0];
        c[1] -= dlt[1];
        r -= dlt[2];
        if dlt.iter().map(|x| x.abs()).fold(0.0, f64::max) < 1e-9 * r.max(1.0) {
            break;
        }
    }
    (r.is_finite() && r > 0.0).then_some((c, r))
}

/// The centre of the circle of radius `r` that fits `pts` best, from `c0`.
pub fn fit_circle_fixed(pts: &[Pt], r: f64, c0: Pt) -> Pt {
    let mut c = c0;
    for _ in 0..30 {
        let (mut a11, mut a12, mut a22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for p in pts {
            let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
            let d = libm::hypot(dx, dy).max(1e-12);
            let f = d - r;
            let (j1, j2) = (-dx / d, -dy / d);
            a11 += j1 * j1;
            a12 += j1 * j2;
            a22 += j2 * j2;
            b1 += j1 * f;
            b2 += j2 * f;
        }
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-18 {
            break;
        }
        let d1 = (a22 * b1 - a12 * b2) / det;
        let d2 = (a11 * b2 - a12 * b1) / det;
        c[0] -= d1;
        c[1] -= d2;
        if d1.abs().max(d2.abs()) < 1e-9 * r {
            break;
        }
    }
    c
}

fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(a);
    if d.is_nan() || d.abs() <= 1e-300 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut m = a;
        for r in 0..3 {
            m[r][k] = b[r];
        }
        *o = det(m) / d;
    }
    Some(out)
}

/// One bend's circle.
#[derive(Debug, Clone, PartialEq)]
pub struct Circle {
    pub c: Pt,
    pub r: f64,
    /// `1` left, `-1` right.
    pub hand: i8,
    /// The target's points of the bend, first and last index.
    pub a: usize,
    pub b: usize,
    /// The radius the bend's points fit freely, m.
    pub fit_r: f64,
    /// The river is tighter here than the seed may be: the circle is held at `r_clip`.
    pub tight: bool,
    /// Pushed off its fit to make room for a straight or a tangent.
    pub moved: bool,
    /// A part of a bend split before.
    pub part: bool,
}

impl Circle {
    fn rho(&self) -> f64 {
        f64::from(self.hand) * self.r
    }
}

/// The turn of the target from index `a` to `b`, in the bend's own direction.
fn run_turn(t: &Target, a: usize, b: usize) -> f64 {
    (t.heading[b] - t.heading[a]).abs()
}

/// The circle of the target's points `a..=b` with `hand`; `None` when they do not make one.
fn circle_of(t: &Target, a: usize, b: usize, hand: i8, p: &SeedParams) -> Option<Circle> {
    if b < a + 2 {
        return None;
    }
    let pts = &t.pts[a..=b];
    let (c, r) = fit_circle(pts)?;
    // The centre must lie on the bend's own side.
    let mid = (a + b) / 2;
    let th = t.heading[mid];
    let side = -libm::sin(th) * (c[0] - t.pts[mid][0]) + libm::cos(th) * (c[1] - t.pts[mid][1]);
    if side * f64::from(hand) <= 0.0 || r > 200.0 * p.r_min {
        return None;
    }
    if r >= p.r_clip {
        return Some(Circle {
            c,
            r,
            hand,
            a,
            b,
            fit_r: r,
            tight: false,
            moved: false,
            part: false,
        });
    }
    // Tighter than the seed may be: the circle of radius `r_clip` that fits best, from the free
    // centre moved away from the bend's middle by the difference of the radii.
    let pm = t.pts[mid];
    let (ux, uy) = (c[0] - pm[0], c[1] - pm[1]);
    let d = libm::hypot(ux, uy).max(1e-9);
    let c0 = [
        c[0] + (p.r_clip - r) * ux / d,
        c[1] + (p.r_clip - r) * uy / d,
    ];
    let c1 = fit_circle_fixed(pts, p.r_clip, c0);
    Some(Circle {
        c: c1,
        r: p.r_clip,
        hand,
        a,
        b,
        fit_r: r,
        tight: true,
        moved: false,
        part: false,
    })
}

/// The bends of the target: runs of one hand above the threshold that turn enough.
fn bends(t: &Target, p: &SeedParams) -> Vec<(usize, usize, i8)> {
    let mut out = Vec::new();
    let n = t.pts.len();
    let mut i = 0;
    while i < n {
        let k = t.kappa[i];
        if k.abs() < p.curve_threshold {
            i += 1;
            continue;
        }
        let hand: i8 = if k > 0.0 { 1 } else { -1 };
        let a = i;
        while i < n && t.kappa[i] * f64::from(hand) >= p.curve_threshold {
            i += 1;
        }
        let b = i - 1;
        if run_turn(t, a, b) >= p.min_turn_rad && b >= a + 2 {
            out.push((a, b, hand));
        }
    }
    out
}

/// Why a chain of circles does not build.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Defect {
    /// Circles `i` and `i + 1` have no common tangent.
    NoTangent(usize),
    /// The arc on circle `i` would turn the wrong way round.
    Wrap(usize),
    /// Opposite circles `i` and `i + 1` leave a straight shorter than the minimum.
    Short(usize, f64),
}

/// The common tangent of two signed circles `(c, ρ)`, in the direction of travel: its heading,
/// length, and its two touch points. `None` when there is none.
fn tangent(ca: Pt, ra: f64, cb: Pt, rb: f64) -> Option<(f64, f64, Pt, Pt)> {
    let d = [cb[0] - ca[0], cb[1] - ca[1]];
    let dd = libm::hypot(d[0], d[1]);
    let q = rb - ra;
    // Two circles of one hand that touch inside: a join of two arcs with no straight.
    if dd < 1e-9 || q.abs() > dd * (1.0 + 1e-12) + 1e-9 {
        return None;
    }
    let alpha = libm::atan2(d[1], d[0]) - libm::asin((q / dd).clamp(-1.0, 1.0));
    let len = (dd * dd - q * q).max(0.0).sqrt();
    let n = [-libm::sin(alpha), libm::cos(alpha)];
    let qa = [ca[0] - ra * n[0], ca[1] - ra * n[1]];
    let qb = [cb[0] - rb * n[0], cb[1] - rb * n[1]];
    Some((alpha, len, qa, qb))
}

/// The heading of travel round a signed circle at `q`.
fn heading_on(c: Pt, rho: f64, q: Pt) -> f64 {
    let (rx, ry) = (q[0] - c[0], q[1] - c[1]);
    // Left (ρ > 0): the radius turned a quarter left; right: a quarter right.
    if rho > 0.0 {
        libm::atan2(rx, -ry)
    } else {
        libm::atan2(-rx, ry)
    }
}

/// A built seed: the chain, and each circle's arc as chainage from and to.
#[derive(Debug, Clone)]
struct Built {
    chain: Chain,
    arcs: Vec<(f64, f64)>,
}

/// The chain of tangents and arcs from `p0` round each circle to `p1`.
fn build(circles: &[Circle], t: &Target, p0: Pt, p1: Pt, p: &SeedParams) -> Result<Built, Defect> {
    let n = circles.len();
    if n == 0 {
        let th = libm::atan2(p1[1] - p0[1], p1[0] - p0[0]);
        let len = libm::hypot(p1[0] - p0[0], p1[1] - p0[1]);
        return Ok(Built {
            chain: Chain::from_pieces((p0[0], p0[1], th), &[(0.0, len)]),
            arcs: vec![],
        });
    }
    // The start: a tangent from p0, or on the first circle when p0 is inside it.
    let first = &circles[0];
    let last = &circles[n - 1];
    let inside = |c: &Circle, q: Pt| libm::hypot(q[0] - c.c[0], q[1] - c.c[1]) < c.r + 1e-9;
    let on = |c: &Circle, q: Pt| {
        let (dx, dy) = (q[0] - c.c[0], q[1] - c.c[1]);
        let d = libm::hypot(dx, dy).max(1e-12);
        [c.c[0] + c.r * dx / d, c.c[1] + c.r * dy / d]
    };
    let mut pieces: Vec<(f64, f64)> = Vec::with_capacity(2 * n + 2);
    let (start, mut entry) = if inside(first, p0) {
        let q = on(first, p0);
        let h = heading_on(first.c, first.rho(), q);
        ((q[0], q[1], h), h)
    } else {
        let (alpha, len, _, _) =
            tangent(p0, 0.0, first.c, first.rho()).ok_or(Defect::NoTangent(0))?;
        pieces.push((0.0, len));
        ((p0[0], p0[1], alpha), alpha)
    };
    let mut arcs = Vec::with_capacity(n);
    let mut s = pieces.iter().map(|x| x.1).sum::<f64>();
    for i in 0..n {
        let ci = &circles[i];
        let (exit, after) = if i + 1 < n {
            let cj = &circles[i + 1];
            let (alpha, len, _, _) =
                tangent(ci.c, ci.rho(), cj.c, cj.rho()).ok_or(Defect::NoTangent(i))?;
            if ci.hand != cj.hand
                && (ci.r < p.tight_reverse_r || cj.r < p.tight_reverse_r)
                && len < p.min_straight_m - 1e-6
            {
                return Err(Defect::Short(i, len));
            }
            (alpha, Some(len))
        } else if inside(last, p1) {
            (heading_on(last.c, last.rho(), on(last, p1)), None)
        } else {
            let (alpha, len, _, _) =
                tangent(ci.c, ci.rho(), p1, 0.0).ok_or(Defect::NoTangent(i))?;
            (alpha, Some(len))
        };
        let turn = (f64::from(ci.hand) * (exit - entry)).rem_euclid(TAU);
        let turn = if turn > TAU - 1e-12 { 0.0 } else { turn };
        if turn > run_turn(t, ci.a, ci.b) + PI / 2.0 {
            return Err(Defect::Wrap(i));
        }
        let len = ci.r * turn;
        arcs.push((s, s + len));
        pieces.push((f64::from(ci.hand) / ci.r, len));
        s += len;
        if let Some(l) = after {
            pieces.push((0.0, l));
            s += l;
        }
        entry = exit;
    }
    Ok(Built {
        chain: Chain::from_pieces(start, &pieces),
        arcs,
    })
}

/// Move circles `i` and `i + 1` apart along the line of their centres until their centres are
/// `want` apart.
fn push_apart(circles: &mut [Circle], i: usize, want: f64) {
    let (a, b) = (circles[i].c, circles[i + 1].c);
    let d = [b[0] - a[0], b[1] - a[1]];
    let dd = libm::hypot(d[0], d[1]).max(1e-9);
    let m = 0.5 * (want - dd).max(0.0);
    let u = [d[0] / dd, d[1] / dd];
    circles[i].c = [a[0] - m * u[0], a[1] - m * u[1]];
    circles[i + 1].c = [b[0] + m * u[0], b[1] + m * u[1]];
    circles[i].moved = true;
    circles[i + 1].moved = true;
}

/// Circles `i` and `i + 1` of one hand, one inside the other: move the smaller away from the
/// larger's centre until they touch inside. False (nothing moved) when that is more than the
/// tolerance.
fn compound(circles: &mut [Circle], i: usize, p: &SeedParams) -> bool {
    let (big, small) = if circles[i].r >= circles[i + 1].r {
        (i, i + 1)
    } else {
        (i + 1, i)
    };
    let (cb, cs) = (circles[big].c, circles[small].c);
    let d = [cs[0] - cb[0], cs[1] - cb[1]];
    let dd = libm::hypot(d[0], d[1]);
    let want = circles[big].r - circles[small].r;
    let m = want - dd;
    if dd < 1e-9 || m > p.tolerance_m {
        return false;
    }
    circles[small].c = [cs[0] + m * d[0] / dd, cs[1] + m * d[1] / dd];
    true
}

/// Merge circles `i` and `i + 1` (one hand) into one fitted to both bends' points.
fn merge(circles: &mut Vec<Circle>, i: usize, t: &Target, p: &SeedParams) {
    let (a, b, hand) = (circles[i].a, circles[i + 1].b, circles[i].hand);
    match circle_of(t, a, b, hand, p) {
        Some(c) => {
            circles[i] = c;
            circles.remove(i + 1);
        }
        None => {
            // No circle fits the two together: keep the longer turn.
            let keep_first = run_turn(t, circles[i].a, circles[i].b)
                >= run_turn(t, circles[i + 1].a, circles[i + 1].b);
            circles.remove(if keep_first { i + 1 } else { i });
        }
    }
}

/// Build, mending defects until the chain builds. Each mend either moves circles apart (bounded:
/// a pair moved apart stays apart) or takes a circle away, so the loop ends.
fn build_mended(circles: &mut Vec<Circle>, t: &Target, p0: Pt, p1: Pt, p: &SeedParams) -> Built {
    let mut pushes = 0usize;
    loop {
        match build(circles, t, p0, p1, p) {
            Ok(b) => return b,
            Err(d) => {
                let limit = 20 * (circles.len() + 1);
                match d {
                    Defect::Short(i, _) if pushes < limit => {
                        let q = circles[i + 1].rho() - circles[i].rho();
                        let want = (p.min_straight_m * p.min_straight_m + q * q).sqrt() + 1e-6;
                        push_apart(circles, i, want);
                        pushes += 1;
                    }
                    Defect::NoTangent(i) if i + 1 < circles.len() => {
                        if circles[i].hand == circles[i + 1].hand {
                            // One circle inside the other: a compound curve. Move the smaller
                            // out until they touch inside (a join of two arcs of one hand with no
                            // straight); merge them when that would move it far.
                            if !compound(circles, i, p) {
                                merge(circles, i, t, p);
                            }
                        } else if pushes < limit {
                            let want = circles[i].r + circles[i + 1].r + 1e-6;
                            push_apart(circles, i, want);
                            pushes += 1;
                        } else {
                            let i_drop = smaller_turn(circles, t, i, i + 1);
                            circles.remove(i_drop);
                        }
                    }
                    Defect::Wrap(i) => mend_wrap(circles, i, t, p),
                    Defect::Short(i, _) | Defect::NoTangent(i) => {
                        // Out of pushes, or a tangent from an end point: take a circle away.
                        let j = if i + 1 < circles.len() {
                            smaller_turn(circles, t, i, i + 1)
                        } else {
                            i
                        };
                        circles.remove(j);
                    }
                }
            }
        }
    }
}

fn smaller_turn(circles: &[Circle], t: &Target, i: usize, j: usize) -> usize {
    if run_turn(t, circles[i].a, circles[i].b) <= run_turn(t, circles[j].a, circles[j].b) {
        i
    } else {
        j
    }
}

/// An arc that would wrap: merge it with a neighbour of its hand, else take it away.
fn mend_wrap(circles: &mut Vec<Circle>, i: usize, t: &Target, p: &SeedParams) {
    let n = circles.len();
    if i + 1 < n && circles[i + 1].hand == circles[i].hand {
        merge(circles, i, t, p);
    } else if i > 0 && circles[i - 1].hand == circles[i].hand {
        merge(circles, i - 1, t, p);
    } else {
        circles.remove(i);
    }
}

/// A tight bend of the seed: the river is tighter than the seed may be, or the seed was pushed off
/// the river to hold the minimum straight.
#[derive(Debug, Clone, PartialEq)]
pub struct TightBend {
    /// Seed chainage of the arc, from and to.
    pub from: f64,
    pub to: f64,
    /// Target chainage of the river's bend, from and to.
    pub river_from: f64,
    pub river_to: f64,
    /// The river's tightest radius in the bend (on the smoothed target), m.
    pub river_radius_m: f64,
    /// The seed's radius there, m.
    pub radius_m: f64,
    /// The largest distance of the seed from the target round the bend, m.
    pub gap_m: f64,
    /// `tight` (the river is tighter than the seed) or `straight` (moved to hold the minimum
    /// straight between opposite arcs).
    pub reason: &'static str,
    /// `1` left, `-1` right.
    pub hand: i8,
}

/// The seed.
#[derive(Debug, Clone)]
pub struct Seed {
    pub chain: Chain,
    pub target: Target,
    pub circles: Vec<Circle>,
    /// Each circle's arc, as seed chainage from and to.
    pub arcs: Vec<(f64, f64)>,
    /// The largest gap from the target outside the tight bends, m.
    pub gap_m: f64,
    /// The largest gap anywhere, m.
    pub gap_all_m: f64,
    /// Refinement passes run.
    pub passes: usize,
    /// True when the gap outside the tight bends is within the tolerance.
    pub settled: bool,
    pub tight: Vec<TightBend>,
}

/// The index range of the target round circle `i` that is excused from the tolerance (a tight
/// bend and its approaches: half the bend's length either side).
fn excused(c: &Circle, n: usize) -> (usize, usize) {
    let half = (c.b - c.a) / 2 + 1;
    (c.a.saturating_sub(half), (c.b + half).min(n - 1))
}

fn gaps(built: &Built, t: &Target) -> Vec<f64> {
    walk_distances(&built.chain, &t.pts)
        .into_iter()
        .map(|x| x.0)
        .collect()
}

fn excused_mask(circles: &[Circle], n: usize) -> Vec<bool> {
    let mut m = vec![false; n];
    for c in circles.iter().filter(|c| c.tight || c.moved) {
        let (a, b) = excused(c, n);
        for x in &mut m[a..=b] {
            *x = true;
        }
    }
    m
}

/// Runs of target points over the tolerance and not excused: `(first, last, index of the largest)`.
fn bad_runs(g: &[f64], excused: &[bool], tol: f64) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < g.len() {
        if g[i] <= tol || excused[i] {
            i += 1;
            continue;
        }
        let a = i;
        let mut m = i;
        while i < g.len() && g[i] > tol && !excused[i] {
            if g[i] > g[m] {
                m = i;
            }
            i += 1;
        }
        out.push((a, i - 1, m));
    }
    out
}

/// A change to the circles for one bad run: split the bend at `m` (in three, or in two when `two`),
/// or add a bend round `m`.
fn mend_for(
    circles: &[Circle],
    t: &Target,
    m: usize,
    p: &SeedParams,
    two: bool,
) -> Option<Vec<Circle>> {
    let pos = circles.partition_point(|c| c.b < m);
    if let Some(c) = circles.get(pos).filter(|c| c.a <= m) {
        if c.tight || c.moved || c.b < c.a + 8 {
            return None;
        }
        let parts = if c.part || two {
            // A part already (or the second try): in two at the largest gap.
            let cut = m.clamp(c.a + 3, c.b - 3);
            vec![(c.a, cut), (cut + 1, c.b)]
        } else {
            // A whole bend: in three, its core (curvature at least 0.65 of its peak) and the two
            // flanks. A bend whose curvature rises and falls smoothly is followed far better by a
            // compound curve than by one arc (the arc's error is about four times the compound's
            // on a sine bend), and a split in two would put a straight at the apex.
            let h = f64::from(c.hand);
            let peak = (c.a..=c.b).map(|i| t.kappa[i] * h).fold(0.0, f64::max);
            let im = (c.a..=c.b)
                .find(|&i| t.kappa[i] * h >= peak)
                .unwrap_or((c.a + c.b) / 2);
            let level = 0.65 * peak;
            let mut lo = im;
            while lo > c.a && t.kappa[lo - 1] * h >= level {
                lo -= 1;
            }
            let mut hi = im;
            while hi < c.b && t.kappa[hi + 1] * h >= level {
                hi += 1;
            }
            let mut v = Vec::new();
            if lo >= c.a + 3 {
                v.push((c.a, lo - 1));
            }
            v.push((lo, hi));
            if hi + 3 <= c.b {
                v.push((hi + 1, c.b));
            }
            if v.len() < 2 {
                let cut = m.clamp(c.a + 3, c.b - 3);
                v = vec![(c.a, cut), (cut + 1, c.b)];
            }
            v
        };
        let mut new = Vec::with_capacity(parts.len());
        for (a, b) in parts {
            let mut one = circle_of(t, a, b, c.hand, p)?;
            one.part = true;
            new.push(one);
        }
        let mut out = circles.to_vec();
        out.splice(pos..=pos, new);
        return Some(out);
    }
    // Between bends: fit a new bend to the points round `m` whose curvature has the hand at `m`.
    let lo = if pos == 0 { 0 } else { circles[pos - 1].b + 1 };
    let hi = circles
        .get(pos)
        .map_or(t.pts.len() - 1, |c| c.a.saturating_sub(1));
    if hi < lo + 4 {
        return None;
    }
    // The hand of the target's turn round `m`, over a quarter of R_min either side.
    let w = ((p.r_min / 4.0) / p.step_m).round() as usize;
    let (wa, wb) = (m.saturating_sub(w).max(lo), (m + w).min(hi));
    let turn = t.heading[wb] - t.heading[wa];
    if turn.abs() < 1e-4 {
        return None;
    }
    let hand: i8 = if turn > 0.0 { 1 } else { -1 };
    let h = f64::from(hand);
    let mut a = m;
    while a > lo && t.kappa[a - 1] * h > 0.0 {
        a -= 1;
    }
    let mut b = m;
    while b < hi && t.kappa[b + 1] * h > 0.0 {
        b += 1;
    }
    if b < a + 4 {
        a = wa;
        b = wb;
    }
    let c = circle_of(t, a, b, hand, p)?;
    let mut out = circles.to_vec();
    out.insert(pos, c);
    Some(out)
}

/// The largest gap in `g[a..=b]`.
fn max_in(g: &[f64], a: usize, b: usize) -> f64 {
    g[a..=b].iter().copied().fold(0.0, f64::max)
}

/// The region of the target circle `i` answers for: from the middle of the bend before to the
/// middle of the bend after (the ends of the line at the first and last bend).
fn region(circles: &[Circle], i: usize, n: usize) -> (usize, usize) {
    let a = if i > 0 {
        (circles[i - 1].a + circles[i - 1].b) / 2
    } else {
        0
    };
    let b = if i + 1 < circles.len() {
        (circles[i + 1].a + circles[i + 1].b) / 2
    } else {
        n - 1
    };
    (a, b)
}

/// The gaps of the target's points `a..=b` from the line through circles `lo..=hi` alone, starting
/// on circle `lo` (or at the line's start) and ending on circle `hi` (or at its end).
fn local_gaps(
    circles: &[Circle],
    lo: usize,
    hi: usize,
    t: &Target,
    (a, b): (usize, usize),
    p: &SeedParams,
) -> Option<Vec<f64>> {
    let n = t.pts.len();
    let on = |c: &Circle, q: Pt| {
        let (dx, dy) = (q[0] - c.c[0], q[1] - c.c[1]);
        let d = libm::hypot(dx, dy).max(1e-12);
        [c.c[0] + c.r * dx / d, c.c[1] + c.r * dy / d]
    };
    let p0 = if a == 0 {
        t.pts[0]
    } else {
        on(&circles[lo], t.pts[a])
    };
    let p1 = if b == n - 1 {
        t.pts[n - 1]
    } else {
        on(&circles[hi], t.pts[b])
    };
    let built = build(&circles[lo..=hi], t, p0, p1, p).ok()?;
    Some(
        walk_distances(&built.chain, &t.pts[a..=b])
            .into_iter()
            .map(|x| x.0)
            .collect(),
    )
}

/// Move circle `i` (its centre and radius, the radius at least `r_clip`) to bring the line through
/// it and its two neighbours nearest the target over its region: Levenberg-Marquardt on the gaps.
/// A tight or moved circle stays where it is.
fn polish(circles: &mut [Circle], i: usize, t: &Target, p: &SeedParams) {
    if circles[i].tight || circles[i].moved {
        return;
    }
    let n = t.pts.len();
    let reg = region(circles, i, n);
    if reg.1 < reg.0 + 3 {
        return;
    }
    let lo = i.saturating_sub(1);
    let hi = (i + 1).min(circles.len() - 1);
    let mut work: Vec<Circle> = circles[lo..=hi].to_vec();
    let k = i - lo;
    let eval = |w: &[Circle]| local_gaps(w, 0, w.len() - 1, t, reg, p);
    let Some(mut f) = eval(&work) else { return };
    let ss = |f: &[f64]| f.iter().map(|x| x * x).sum::<f64>();
    let mut cost = ss(&f);
    let mut lambda = 1e-3;
    for _ in 0..30 {
        let x = [work[k].c[0], work[k].c[1], work[k].r];
        let set = |w: &mut [Circle], x: [f64; 3]| {
            w[k].c = [x[0], x[1]];
            w[k].r = x[2].max(p.r_clip);
        };
        // The Jacobian by forward differences.
        let h = [0.05, 0.05, 0.05];
        let mut cols: Vec<Vec<f64>> = Vec::with_capacity(3);
        for u in 0..3 {
            let mut xx = x;
            xx[u] += h[u];
            set(&mut work, xx);
            let Some(fu) = eval(&work) else {
                set(&mut work, x);
                return circles[lo..=hi].clone_from_slice(&work);
            };
            cols.push(fu.iter().zip(&f).map(|(a, b)| (a - b) / h[u]).collect());
        }
        set(&mut work, x);
        let mut jtj = [[0.0; 3]; 3];
        let mut jtf = [0.0; 3];
        for (row, fr) in f.iter().enumerate() {
            for u in 0..3 {
                jtf[u] += cols[u][row] * fr;
                for v in 0..3 {
                    jtj[u][v] += cols[u][row] * cols[v][row];
                }
            }
        }
        let mut improved = false;
        for _ in 0..8 {
            let mut m = jtj;
            for (u, row) in m.iter_mut().enumerate() {
                row[u] += lambda * (jtj[u][u] + 1e-9);
            }
            let Some(d) = solve3(m, jtf) else { break };
            let xx = [x[0] - d[0], x[1] - d[1], x[2] - d[2]];
            set(&mut work, xx);
            match eval(&work) {
                Some(fn_) if ss(&fn_) < cost => {
                    f = fn_;
                    cost = ss(&f);
                    lambda = (lambda / 3.0).max(1e-9);
                    improved = true;
                    break;
                }
                _ => {
                    set(&mut work, x);
                    lambda *= 4.0;
                }
            }
        }
        if !improved {
            break;
        }
    }
    // Held at the least radius: the river wants a tighter arc here than the seed may have.
    if work[k].r <= p.r_clip * (1.0 + 1e-9) {
        work[k].tight = true;
    }
    circles[lo..=hi].clone_from_slice(&work);
}

/// The seed of `centreline` (at least two points). `in_water`, when given, says whether a point is
/// in the river (the polygon shrunk by the margin); it is read only with [`SeedAim::InWater`].
pub fn seed(centreline: &[Pt], p: &SeedParams, in_water: Option<&dyn Fn(Pt) -> bool>) -> Seed {
    let t = target(centreline, p);
    seed_of_target(t, p, in_water)
}

/// The seed of a target already made.
pub fn seed_of_target(t: Target, p: &SeedParams, in_water: Option<&dyn Fn(Pt) -> bool>) -> Seed {
    let n = t.pts.len();
    let (p0, p1) = (t.pts[0], t.pts[n - 1]);
    let mut circles: Vec<Circle> = bends(&t, p)
        .into_iter()
        .filter_map(|(a, b, hand)| circle_of(&t, a, b, hand, p))
        .collect();
    if p.aim == SeedAim::InWater
        && let Some(w) = in_water
    {
        for c in circles.iter_mut().filter(|c| c.tight) {
            place_in_water(c, &t, w);
        }
    }
    build_mended(&mut circles, &t, p0, p1, p);
    for i in 0..circles.len() {
        polish(&mut circles, i, &t, p);
    }
    let mut built = build_mended(&mut circles, &t, p0, p1, p);
    let mut g = gaps(&built, &t);
    let excess = |g: &[f64], circles: &[Circle]| -> f64 {
        g.iter()
            .zip(excused_mask(circles, n))
            .filter(|(_, e)| !*e)
            .map(|(x, _)| (x - p.tolerance_m).max(0.0))
            .sum()
    };
    let mut now = excess(&g, &circles);
    let mut stuck: Vec<usize> = Vec::new();
    let mut passes = 0;
    let mut progress = false;
    while passes < p.max_passes && now > 0.0 {
        // A pass that changed the line may have made a stuck place mendable: try them again.
        if progress {
            stuck.clear();
        }
        progress = false;
        let ex = excused_mask(&circles, n);
        let runs: Vec<(usize, usize, usize)> = bad_runs(&g, &ex, p.tolerance_m)
            .into_iter()
            .filter(|r| !stuck.iter().any(|&s| s >= r.0 && s <= r.1))
            .collect();
        if runs.is_empty() {
            break;
        }
        passes += 1;
        // Each bad run in turn: mend it, build, and keep the mend when it brings the line nearer.
        for r in runs {
            let mut kept = false;
            for two in [false, true] {
                let Some(mut next) = mend_for(&circles, &t, r.2, p, two) else {
                    continue;
                };
                // Polish the circles round the mend.
                let pos = next.partition_point(|c| c.b < r.0).saturating_sub(1);
                let end = (pos + 5).min(next.len());
                build_mended(&mut next, &t, p0, p1, p);
                for i in pos..end.min(next.len()) {
                    polish(&mut next, i, &t, p);
                }
                let next_built = build_mended(&mut next, &t, p0, p1, p);
                let ng = gaps(&next_built, &t);
                let e = excess(&ng, &next);
                if e < now - 1e-9 {
                    circles = next;
                    built = next_built;
                    g = ng;
                    now = e;
                    kept = true;
                    progress = true;
                    break;
                }
            }
            if !kept {
                stuck.push(r.2);
            }
        }
    }
    let ex = excused_mask(&circles, n);
    let gap_m = g
        .iter()
        .zip(&ex)
        .filter(|(_, e)| !**e)
        .map(|(x, _)| *x)
        .fold(0.0, f64::max);
    let gap_all_m = g.iter().copied().fold(0.0, f64::max);
    let tight = circles
        .iter()
        .zip(&built.arcs)
        .filter(|(c, _)| c.tight || c.moved)
        .map(|(c, arc)| {
            let (a, b) = excused(c, n);
            TightBend {
                from: arc.0,
                to: arc.1,
                river_from: t.s[c.a],
                river_to: t.s[c.b],
                river_radius_m: (c.a..=c.b)
                    .map(|i| 1.0 / t.kappa[i].abs())
                    .fold(f64::INFINITY, f64::min),
                radius_m: c.r,
                gap_m: max_in(&g, a, b),
                reason: if c.tight { "tight" } else { "straight" },
                hand: c.hand,
            }
        })
        .collect();
    Seed {
        chain: built.chain,
        arcs: built.arcs,
        circles,
        gap_m,
        gap_all_m,
        passes,
        settled: gap_m <= p.tolerance_m,
        tight,
        target: t,
    }
}

/// A tight bend's circle moved along the line through its centre and the bend's middle, up to
/// half its distance from the target either way, to the place that keeps the most of the arc's
/// points in the water (the least-squares place first on a tie).
fn place_in_water(c: &mut Circle, t: &Target, in_water: &dyn Fn(Pt) -> bool) {
    let mid = (c.a + c.b) / 2;
    let pm = t.pts[mid];
    let (ux, uy) = (c.c[0] - pm[0], c.c[1] - pm[1]);
    let d = libm::hypot(ux, uy).max(1e-9);
    let u = [ux / d, uy / d];
    let score = |cc: Pt| -> usize {
        t.pts[c.a..=c.b]
            .iter()
            .filter(|q| {
                let (dx, dy) = (q[0] - cc[0], q[1] - cc[1]);
                let dd = libm::hypot(dx, dy).max(1e-9);
                in_water([cc[0] + c.r * dx / dd, cc[1] + c.r * dy / dd])
            })
            .count()
    };
    let reach = 0.5 * (c.r - c.fit_r).abs().max(10.0);
    let steps = 20;
    let mut best = (score(c.c), 0usize, c.c);
    for k in 1..=steps {
        for sgn in [-1.0, 1.0] {
            let m = sgn * reach * k as f64 / steps as f64;
            let cc = [c.c[0] + m * u[0], c.c[1] + m * u[1]];
            let s = score(cc);
            if s > best.0 {
                best = (s, k, cc);
            }
        }
    }
    if best.1 > 0 {
        c.c = best.2;
    }
}

/// The pose of the target at index `i`.
pub fn target_pose(t: &Target, i: usize) -> Pose {
    (t.pts[i][0], t.pts[i][1], t.heading[i])
}

/// The target followed in exact steps of `step` metres (spec 2.1, `track`): each step an arc of
/// curvature `k_T − w² e − 2 w ψ` (the target's curvature, the lateral error `e`, the heading error
/// `ψ`, `w` = 1 / 40 m, critically damped), clipped to `1 / r_clip`. Tangent everywhere and never
/// tighter than `r_clip` by construction; many short elements, so the method uses it only when the
/// arcs of [`seed_of_target`] miss the target by more than the tolerance and this does better.
pub fn track(t: &Target, r_clip: f64, step: f64) -> Chain {
    let n = t.pts.len();
    if n < 2 {
        return Chain::default();
    }
    let w = 1.0 / 40.0;
    let kmax = 1.0 / r_clip;
    let total = t.s[n - 1];
    let mut pose = target_pose(t, 0);
    let mut pieces = Vec::new();
    let mut j = 0usize;
    let mut s = 0.0;
    while s < total - 1e-9 {
        // The nearest target point, moving on only.
        while j + 1 < n {
            let d0 = libm::hypot(t.pts[j][0] - pose.0, t.pts[j][1] - pose.1);
            let d1 = libm::hypot(t.pts[j + 1][0] - pose.0, t.pts[j + 1][1] - pose.1);
            if d1 <= d0 { j += 1 } else { break }
        }
        let th = t.heading[j];
        let e = -libm::sin(th) * (pose.0 - t.pts[j][0]) + libm::cos(th) * (pose.1 - t.pts[j][1]);
        let mut psi = pose.2 - th;
        while psi > PI {
            psi -= TAU;
        }
        while psi < -PI {
            psi += TAU;
        }
        let k = (t.kappa[j] - w * w * e - 2.0 * w * psi).clamp(-kmax, kmax);
        let l = step.min(total - s);
        pieces.push((k, l));
        pose = super::elements::advance(pose, k, l);
        s += l;
    }
    Chain::from_pieces(target_pose(t, 0), &pieces)
}
