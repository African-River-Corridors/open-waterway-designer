//! The sideways shift (spec OPT-4, section 4): the line moved sideways by dynamic programming to cut
//! less, the shifted line's own curvature a hard limit, the line's curves and straights kept.
//!
//! The state at a station is the offset and the step from the station before (so a move sets the
//! second difference of the offset, and with it the curvature). The curvature of the shifted line
//! `r(s) = r0(s) + d(s) N(s)` is, with the line's curvature `k0` and its rate `k0'`:
//!
//! ```text
//! k = [ (1 − k0 d)(k0 (1 − k0 d) + d'') + d' (k0' d + 2 k0 d') ] / [ (1 − k0 d)² + d'² ]^{3/2}
//! ```
//!
//! (from `r' = (1 − k0 d) T + d' N`, `r'' = −(k0' d + 2 k0 d') T + (k0 (1 − k0 d) + d'') N`,
//! `k = r' × r'' / |r'|³`). The model reads `k0` smoothed over one station (the line's curvature
//! jumps at its element joins); the delivered line is built again as exact arcs and checked, and
//! the gap between the shifted target and the delivered line is recorded.

use super::elements::{Chain, Pose};
use super::mask::RiverMask;
use super::score::StationScorer;
use crate::geom::Pt;

/// The curvature of the shifted line at a station (see the module's formula).
pub fn shifted_curvature(k0: f64, dk0: f64, d: f64, d1: f64, d2: f64) -> f64 {
    let a = 1.0 - k0 * d;
    let num = a * (k0 * a + d2) + d1 * (dk0 * d + 2.0 * k0 * d1);
    let q = a * a + d1 * d1;
    let den = q * libm::sqrt(q);
    num / den
}

/// The station spacing for R_min (spec 4.8): R_min / 6, from 20 to 50 m.
pub fn station_spacing(r_min: f64) -> f64 {
    (r_min / 6.0).clamp(20.0, 50.0)
}

/// The offset step for a station spacing `h` and R_min (spec 4.8): the smallest curvature step
/// the grid can make, `δ / h²`, at most `1 / (6 R_min)`; rounded down to 0.25 m, at least 0.25 m.
pub fn offset_step(h: f64, r_min: f64) -> f64 {
    let raw = h * h / (6.0 * r_min);
    ((raw / 0.25).floor() * 0.25).max(0.25)
}

/// The shift's settings, each from R_min and the vessel.
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftParams {
    pub r_min: f64,
    /// The radius limit of the shifted line, m.
    pub r_limit: f64,
    /// Station spacing and offset step, m.
    pub station_m: f64,
    pub offset_step_m: f64,
    /// The largest offset, m.
    pub reach_m: f64,
    /// The steepest sideways slope.
    pub max_slope: f64,
    /// Keep the line's curves and straights (spec 4.5).
    pub same_curves: bool,
    /// The offsets an accepted cut-through allows either side, m.
    pub cut_room_m: f64,
    /// A stretch out of the river at least this long is a forced or chosen stretch, m.
    pub run_min_m: f64,
    /// The smoothness weight (on `(d'' R_min)²` per metre of line).
    pub w_bend: f64,
    /// A price on turning (on `|k| R_min` per metre); 0: off.
    pub w_turn: f64,
    /// The pull toward the line (per metre of offset per metre of line).
    pub pull: f64,
    /// Refine after the dynamic programme.
    pub refine: bool,
    /// The bottom-width levels the cost tables are built at, as radii (∞ for a straight).
    pub levels: Vec<f64>,
}

impl ShiftParams {
    /// The settings at R_min `r_min`.
    pub fn scaled(r_min: f64, r_limit: f64, cut_room_m: f64, run_min_m: f64) -> ShiftParams {
        let h = station_spacing(r_min);
        ShiftParams {
            r_min,
            r_limit,
            station_m: h,
            offset_step_m: offset_step(h, r_min),
            reach_m: 150f64.max(0.5 * r_min),
            max_slope: 0.30,
            same_curves: true,
            cut_room_m,
            run_min_m,
            w_bend: 5.0,
            w_turn: 0.0,
            pull: 0.002,
            refine: true,
            levels: levels(r_min, 4),
        }
    }
}

/// The widening levels: a straight, then `4 R_min`, `2 R_min`, `R_min` (as many as `n` asks).
pub fn levels(r_min: f64, n: usize) -> Vec<f64> {
    let all = [f64::INFINITY, 4.0 * r_min, 2.0 * r_min, r_min, 0.75 * r_min];
    all.iter().take(n.clamp(1, all.len())).copied().collect()
}

/// What a station may do.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub s: f64,
    pub pose: Pose,
    /// The line's curvature, smoothed over one station, and its rate.
    pub k0: f64,
    pub dk0: f64,
    /// The offsets allowed, as index bounds (offset = index × step): `lo..=hi`, and inside them
    /// each one's flag.
    pub lo: i32,
    pub hi: i32,
    pub allowed: Vec<bool>,
    /// Curvature bounds from the same-curves rule.
    pub kmin: f64,
    pub kmax: f64,
}

impl Station {
    fn ok(&self, j: i32) -> bool {
        j >= self.lo && j <= self.hi && self.allowed[(j - self.lo) as usize]
    }
}

/// A stretch of the line where every offset is allowed within `room` (a chosen cut-through or a
/// forced stretch): line chainage from and to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Room {
    pub from: f64,
    pub to: f64,
    pub room_m: f64,
}

/// The stations of `chain` with their limits.
pub fn stations(
    chain: &Chain,
    p: &ShiftParams,
    mask: Option<&RiverMask>,
    rooms: &[Room],
) -> Vec<Station> {
    let st = chain.stations(p.station_m);
    let n = st.len();
    let h = chain.length() / (n - 1).max(1) as f64;
    // The line's curvature averaged over ±h/2: the mean turn over the station's own length.
    let k0: Vec<f64> = st
        .iter()
        .map(|x| {
            let (a, b) = (
                (x.0 - h / 2.0).max(0.0),
                (x.0 + h / 2.0).min(chain.length()),
            );
            if b > a {
                (chain.pose_at(b).2 - chain.pose_at(a).2) / (b - a)
            } else {
                x.2
            }
        })
        .collect();
    let dk0: Vec<f64> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(1), (i + 1).min(n - 1));
            if b > a {
                (k0[b] - k0[a]) / ((b - a) as f64 * h)
            } else {
                0.0
            }
        })
        .collect();
    let step = p.offset_step_m;
    let reach = (p.reach_m / step).floor() as i32;
    // In the river or not, along the line, and the runs out of it.
    let inside: Vec<bool> = st
        .iter()
        .map(|x| mask.is_none_or(|m| m.contains([x.1.0, x.1.1])))
        .collect();
    let mut run_len = vec![0.0; n];
    let mut i = 0;
    while i < n {
        if inside[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && !inside[i] {
            i += 1;
        }
        let len = (i - a) as f64 * h;
        for x in &mut run_len[a..i] {
            *x = len;
        }
    }
    let (kmin, kmax) = same_curve_bounds(&k0, h, p);
    (0..n)
        .map(|i| {
            let (s, pose, _) = st[i];
            let nrm = [-libm::sin(pose.2), libm::cos(pose.2)];
            let at = |j: i32| {
                [
                    pose.0 + j as f64 * step * nrm[0],
                    pose.1 + j as f64 * step * nrm[1],
                ]
            };
            let room = rooms
                .iter()
                .find(|r| s >= r.from - 1e-6 && s <= r.to + 1e-6);
            let (lo, _, allowed) = if i == 0 || i == n - 1 {
                (0, 0, vec![true])
            } else if let Some(r) = room {
                let m = ((r.room_m.min(p.reach_m)) / step).floor() as i32;
                (-m, m, vec![true; (2 * m + 1) as usize])
            } else if !inside[i] && run_len[i] >= p.run_min_m {
                (-reach, reach, vec![true; (2 * reach + 1) as usize])
            } else if let Some(m) = mask {
                if inside[i] {
                    let ok: Vec<bool> = (-reach..=reach)
                        .map(|j| j == 0 || m.contains(at(j)))
                        .collect();
                    (-reach, reach, ok)
                } else {
                    // Just clipping the bank: only moves toward the river, up to the nearest
                    // offset in it.
                    let near = (1..=reach)
                        .flat_map(|k| [k, -k])
                        .find(|&j| m.contains(at(j)));
                    match near {
                        None => (0, 0, vec![true]),
                        Some(j) => {
                            let (lo, hi) = (j.min(0), j.max(0));
                            (lo, hi, vec![true; (hi - lo + 1) as usize])
                        }
                    }
                }
            } else {
                (-reach, reach, vec![true; (2 * reach + 1) as usize])
            };
            // Trim the bounds to the allowed offsets.
            let first = allowed.iter().position(|x| *x).unwrap_or(0) as i32;
            let last = allowed.iter().rposition(|x| *x).unwrap_or(0) as i32;
            Station {
                s,
                pose,
                k0: k0[i],
                dk0: dk0[i],
                lo: lo + first,
                hi: lo + last,
                allowed: allowed[first as usize..=last as usize].to_vec(),
                kmin: kmin[i],
                kmax: kmax[i],
            }
        })
        .collect()
}

/// The same-curves bounds (spec 4.5), thresholds from R_min: a curve where `|k0| > 1/(8 R_min)`;
/// in a curve no curvature against its hand beyond `1/(10 R_min)`; in the straight between
/// opposite curves `|k| ≤ max(1/(10 R_min), |k0|)`; near a reversal (`2 R_min / 3` either side)
/// `|k| ≤ max(|k0|, 1/(2 R_min))`. Off: only the radius limit.
fn same_curve_bounds(k0: &[f64], h: f64, p: &ShiftParams) -> (Vec<f64>, Vec<f64>) {
    let n = k0.len();
    let big = f64::INFINITY;
    if !p.same_curves {
        return (vec![-big; n], vec![big; n]);
    }
    let curve = 1.0 / (8.0 * p.r_min);
    let flat = 1.0 / (10.0 * p.r_min);
    let gentle = 1.0 / (2.0 * p.r_min);
    let near = ((2.0 * p.r_min / 3.0) / h).ceil() as usize;
    let class: Vec<i8> = k0
        .iter()
        .map(|k| {
            if *k > curve {
                1
            } else if *k < -curve {
                -1
            } else {
                0
            }
        })
        .collect();
    // Each run of 0 takes the hand of its neighbours when they agree, or is a gap between
    // opposite curves.
    let mut hand = class.clone();
    let mut gap = vec![false; n];
    let mut i = 0;
    while i < n {
        if class[i] != 0 {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && class[i] == 0 {
            i += 1;
        }
        let before = if a > 0 { class[a - 1] } else { 0 };
        let after = if i < n { class[i] } else { 0 };
        let take = match (before, after) {
            (0, x) | (x, 0) => x,
            (x, y) if x == y => x,
            _ => {
                for g in &mut gap[a..i] {
                    *g = true;
                }
                0
            }
        };
        for x in &mut hand[a..i] {
            *x = take;
        }
    }
    let mut kmin = vec![-big; n];
    let mut kmax = vec![big; n];
    for i in 0..n {
        let k = k0[i];
        if gap[i] {
            let m = flat.max(k.abs());
            kmin[i] = -m;
            kmax[i] = m;
        } else if hand[i] > 0 {
            kmin[i] = (-flat).min(k);
        } else if hand[i] < 0 {
            kmax[i] = flat.max(k);
        }
    }
    // Near every reversal of the line's hand: a gentle S stays gentle, and the straight keeps its
    // length (two stations either side of the gap).
    for i in 1..n {
        let (a, b) = (hand[i - 1], hand[i]);
        let reversal = (a != 0 && b != 0 && a != b) || (gap[i] != gap[i - 1]);
        if !reversal {
            continue;
        }
        for j in i.saturating_sub(near)..(i + near).min(n) {
            let m = gentle.max(k0[j].abs());
            kmin[j] = kmin[j].max(-m);
            kmax[j] = kmax[j].min(m);
        }
        if gap[i] != gap[i - 1] {
            for j in i.saturating_sub(2)..(i + 2).min(n) {
                let m = flat.max(k0[j].abs());
                kmin[j] = kmin[j].max(-m);
                kmax[j] = kmax[j].min(m);
            }
        }
    }
    // Zero shift always stays allowed.
    for i in 0..n {
        kmin[i] = kmin[i].min(k0[i]);
        kmax[i] = kmax[i].max(k0[i]);
    }
    (kmin, kmax)
}

/// The cost tables: at each station and allowed offset, the scorer's cost per metre at each
/// widening level (the station's own hand).
pub struct Tables {
    /// `[station][offset - lo][level]`.
    pub cost: Vec<Vec<Vec<f64>>>,
    /// The widening at each level, m (for the interpolation).
    pub widening: Vec<f64>,
}

/// Build the cost tables.
pub fn tables(
    st: &[Station],
    p: &ShiftParams,
    scorer: &dyn StationScorer,
    widening: &dyn Fn(f64) -> f64,
) -> Tables {
    // On plan area the widening moves only the edge of the water surface: one level (the
    // straight) is priced, so the tables cost a quarter.
    let one = [f64::INFINITY];
    let lv: &[f64] = if scorer.has_ground() { &p.levels } else { &one };
    let wid: Vec<f64> = lv
        .iter()
        .map(|r| if r.is_finite() { widening(*r) } else { 0.0 })
        .collect();
    let cost = st
        .iter()
        .map(|x| {
            let hand = if x.k0 < 0.0 { -1.0 } else { 1.0 };
            let nrm = [-libm::sin(x.pose.2), libm::cos(x.pose.2)];
            (x.lo..=x.hi)
                .map(|j| {
                    let d = j as f64 * p.offset_step_m;
                    let at = [x.pose.0 + d * nrm[0], x.pose.1 + d * nrm[1]];
                    lv.iter()
                        .map(|r| {
                            let k = if r.is_finite() { hand / r } else { 0.0 };
                            scorer.section(at, x.pose.2, k).cost_per_m
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    Tables {
        cost,
        widening: wid,
    }
}

impl Tables {
    /// The cost per metre at station `i`, offset index `j`, for a line of curvature `k`.
    fn at(&self, st: &Station, i: usize, j: i32, k: f64, widening: &dyn Fn(f64) -> f64) -> f64 {
        let row = &self.cost[i][(j - st.lo) as usize];
        if row.len() == 1 {
            return row[0];
        }
        let w = if k != 0.0 {
            widening(1.0 / k.abs())
        } else {
            0.0
        };
        // Between the two levels whose widening holds `w` (clamped at the ends).
        let n = row.len();
        if w <= self.widening[0] {
            return row[0];
        }
        for l in 1..n {
            let (a, b) = (self.widening[l - 1], self.widening[l]);
            if w <= b || l == n - 1 {
                if b <= a {
                    return row[l];
                }
                let t = ((w - a) / (b - a)).clamp(0.0, 1.0);
                return row[l - 1] + t * (row[l] - row[l - 1]);
            }
        }
        row[n - 1]
    }
}

/// The shift's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Shift {
    /// The offset at each station, m.
    pub offsets: Vec<f64>,
    /// The search cost with no shift, and with the shift.
    pub cost_zero: f64,
    pub cost: f64,
    /// True when the end could be closed exactly at zero offset and zero step.
    pub closed: bool,
    /// The refinement's passes kept.
    pub refined: usize,
}

/// The cost of one station's move: the section over the station's length (on the shifted line's
/// own length), the pull to the line, smoothness and the turning price.
#[allow(clippy::too_many_arguments)]
fn move_cost(
    t: &Tables,
    st: &Station,
    i: usize,
    j: i32,
    d: f64,
    d2: f64,
    k: f64,
    h: f64,
    p: &ShiftParams,
    widening: &dyn Fn(f64) -> f64,
) -> f64 {
    t.at(st, i, j, k, widening) * h * (1.0 - st.k0 * d)
        + p.pull * d.abs() * h
        + p.w_bend * (d2 * p.r_min) * (d2 * p.r_min) * h
        + p.w_turn * k.abs() * p.r_min * h
}

/// True when the curvature `k` at a station at offset `d` is allowed.
fn k_ok(st: &Station, d: f64, k: f64, p: &ShiftParams) -> bool {
    if 1.0 - st.k0 * d < 0.25 {
        return false;
    }
    let lim = (1.0 / p.r_limit).max(st.k0.abs()) + 1e-12;
    k.abs() <= lim && k >= st.kmin - 1e-12 && k <= st.kmax + 1e-12
}

/// The dynamic programme.
pub fn search(st: &[Station], t: &Tables, p: &ShiftParams, widening: &dyn Fn(f64) -> f64) -> Shift {
    let n = st.len();
    let h = if n > 1 {
        (st[n - 1].s - st[0].s) / (n - 1) as f64
    } else {
        1.0
    };
    let dl = p.offset_step_m;
    let mmax = ((p.max_slope * h / dl).floor() as i32).max(1);
    let amax = ((2.2 * h * h / (p.r_min * dl)).ceil() as i32).max(1);
    let nm = (2 * mmax + 1) as usize;
    let idx = |x: &Station, j: i32, m: i32| (j - x.lo) as usize * nm + (m + mmax) as usize;
    // The cost of zero shift.
    let zero_cost: f64 = (0..n)
        .map(|i| move_cost(t, &st[i], i, 0, 0.0, 0.0, st[i].k0, h, p, widening))
        .sum();
    let mut val: Vec<f64> = vec![f64::INFINITY; (st[0].hi - st[0].lo + 1) as usize * nm];
    val[idx(&st[0], 0, 0)] = 0.0;
    let mut back: Vec<Vec<i8>> = Vec::with_capacity(n);
    back.push(vec![0; val.len()]);
    for i in 0..n - 1 {
        let (a, b) = (&st[i], &st[i + 1]);
        let mut next = vec![f64::INFINITY; (b.hi - b.lo + 1) as usize * nm];
        let mut bp = vec![0i8; next.len()];
        for j in a.lo..=a.hi {
            if !a.ok(j) {
                continue;
            }
            let d = j as f64 * dl;
            for m in -mmax..=mmax {
                let v = val[idx(a, j, m)];
                if !v.is_finite() {
                    continue;
                }
                // At the first station the line leaves with zero slope.
                let (m_lo, m_hi) = if i == 0 {
                    (0, 0)
                } else {
                    ((m - amax).max(-mmax), (m + amax).min(mmax))
                };
                for m2 in m_lo..=m_hi {
                    let j2 = j + m2;
                    if !b.ok(j2) {
                        continue;
                    }
                    let d1 = (m + m2) as f64 * dl / (2.0 * h);
                    let d2 = (m2 - m) as f64 * dl / (h * h);
                    let k = shifted_curvature(a.k0, a.dk0, d, d1, d2);
                    if !k_ok(a, d, k, p) {
                        continue;
                    }
                    let c = v + move_cost(t, a, i, j, d, d2, k, h, p, widening);
                    let x = idx(b, j2, m2);
                    if c < next[x] {
                        next[x] = c;
                        bp[x] = m as i8;
                    }
                }
            }
        }
        val = next;
        back.push(bp);
    }
    // The end: zero offset, zero step; else the best end state.
    let last = &st[n - 1];
    let end_cost = |j: i32| {
        move_cost(
            t,
            last,
            n - 1,
            j,
            j as f64 * dl,
            0.0,
            last.k0,
            h,
            p,
            widening,
        )
    };
    let mut closed = true;
    let mut end = (0i32, 0i32);
    let mut best = if last.ok(0) {
        val[idx(last, 0, 0)] + end_cost(0)
    } else {
        f64::INFINITY
    };
    if !best.is_finite() {
        closed = false;
        for j in last.lo..=last.hi {
            for m in -mmax..=mmax {
                let v = val[idx(last, j, m)] + end_cost(j);
                if v < best {
                    best = v;
                    end = (j, m);
                }
            }
        }
    }
    if !best.is_finite() {
        return Shift {
            offsets: vec![0.0; n],
            cost_zero: zero_cost,
            cost: zero_cost,
            closed: false,
            refined: 0,
        };
    }
    // Back through the pointers.
    let mut js = vec![0i32; n];
    let (mut j, mut m) = end;
    for i in (0..n).rev() {
        js[i] = j;
        if i == 0 {
            break;
        }
        let pm = back[i][idx(&st[i], j, m)] as i32;
        j -= m;
        m = pm;
    }
    let offsets: Vec<f64> = js.iter().map(|j| *j as f64 * dl).collect();
    Shift {
        cost: line_cost(st, t, p, &offsets, widening).unwrap_or(best),
        offsets,
        cost_zero: zero_cost,
        closed,
        refined: 0,
    }
}

/// The cost of station `i` with continuous offsets `d` (its offset read between its two grid
/// neighbours); `None` when a hard limit fails there.
fn station_cost(
    st: &[Station],
    t: &Tables,
    p: &ShiftParams,
    d: &[f64],
    i: usize,
    h: f64,
    widening: &dyn Fn(f64) -> f64,
) -> Option<f64> {
    let n = st.len();
    let dl = p.offset_step_m;
    let (dp, dn) = (
        if i > 0 { d[i - 1] } else { d[i] },
        if i + 1 < n { d[i + 1] } else { d[i] },
    );
    let d1 = (dn - dp) / (2.0 * h);
    let d2 = if i > 0 && i + 1 < n {
        (dn - 2.0 * d[i] + dp) / (h * h)
    } else {
        0.0
    };
    let k = shifted_curvature(st[i].k0, st[i].dk0, d[i], d1, d2);
    if i > 0 && i + 1 < n && !k_ok(&st[i], d[i], k, p) {
        return None;
    }
    let x = d[i] / dl;
    let (j0, f) = (x.floor() as i32, x - x.floor());
    let ok = |j: i32| st[i].ok(j);
    if f < 1e-9 && ok(j0) {
        Some(move_cost(t, &st[i], i, j0, d[i], d2, k, h, p, widening))
    } else if f > 1.0 - 1e-9 && ok(j0 + 1) {
        Some(move_cost(t, &st[i], i, j0 + 1, d[i], d2, k, h, p, widening))
    } else if ok(j0) && ok(j0 + 1) {
        Some(
            (1.0 - f) * move_cost(t, &st[i], i, j0, d[i], d2, k, h, p, widening)
                + f * move_cost(t, &st[i], i, j0 + 1, d[i], d2, k, h, p, widening),
        )
    } else {
        None
    }
}

fn spacing(st: &[Station]) -> f64 {
    let n = st.len();
    if n > 1 {
        (st[n - 1].s - st[0].s) / (n - 1) as f64
    } else {
        1.0
    }
}

/// The search cost of continuous offsets; `None` when a hard limit fails.
pub fn line_cost(
    st: &[Station],
    t: &Tables,
    p: &ShiftParams,
    d: &[f64],
    widening: &dyn Fn(f64) -> f64,
) -> Option<f64> {
    let h = spacing(st);
    (0..st.len())
        .map(|i| station_cost(st, t, p, d, i, h, widening))
        .sum()
}

/// The continuous refinement after the programme: each station's offset moved by a fraction of
/// the grid step, kept when the cost of the three stations it changes falls and every hard limit
/// there holds, sweeping the line three times at a half, a quarter and an eighth of the step. It
/// takes the grid's staircase out at little cost.
pub fn refine(
    st: &[Station],
    t: &Tables,
    p: &ShiftParams,
    shift: &mut Shift,
    widening: &dyn Fn(f64) -> f64,
) {
    let n = st.len();
    let h = spacing(st);
    if line_cost(st, t, p, &shift.offsets, widening).is_none() {
        return;
    }
    let local = |d: &[f64], i: usize| -> Option<f64> {
        (i.saturating_sub(1)..=(i + 1).min(n - 1))
            .map(|k| station_cost(st, t, p, d, k, h, widening))
            .sum()
    };
    let dl = p.offset_step_m;
    let mut kept = 0;
    for pass in 0..3 {
        let step = dl / f64::from(2u32 << pass);
        let mut any = false;
        for i in 1..n.saturating_sub(1) {
            for sgn in [-1.0, 1.0] {
                let Some(before) = local(&shift.offsets, i) else {
                    continue;
                };
                let old = shift.offsets[i];
                shift.offsets[i] = old + sgn * step;
                match local(&shift.offsets, i) {
                    Some(c) if c < before - 1e-9 => any = true,
                    _ => shift.offsets[i] = old,
                }
            }
        }
        if any {
            kept += 1;
        }
    }
    shift.cost = line_cost(st, t, p, &shift.offsets, widening).unwrap_or(shift.cost);
    shift.refined = kept;
}

/// The second derivatives of the natural cubic spline through `y` at even spacing `h`.
fn spline_m(y: &[f64], h: f64) -> Vec<f64> {
    let n = y.len();
    let mut m = vec![0.0; n];
    if n < 3 {
        return m;
    }
    // Tridiagonal: m[i-1] + 4 m[i] + m[i+1] = 6 (y[i-1] - 2 y[i] + y[i+1]) / h², m[0] = m[n-1] = 0.
    let k = n - 2;
    let mut c = vec![0.0; k];
    let mut d = vec![0.0; k];
    for i in 0..k {
        let rhs = 6.0 * (y[i] - 2.0 * y[i + 1] + y[i + 2]) / (h * h);
        let (prev_c, prev_d) = if i == 0 {
            (0.0, 0.0)
        } else {
            (c[i - 1], d[i - 1])
        };
        let den = 4.0 - prev_c;
        c[i] = 1.0 / den;
        d[i] = (rhs - prev_d) / den;
    }
    for i in (0..k).rev() {
        let next = if i + 1 < k { m[i + 2] } else { 0.0 };
        m[i + 1] = d[i] - c[i] * next;
    }
    m
}

/// The shifted line's points: the offsets carried to every `step` metres along `chain` by the
/// natural cubic spline through the stations (its second derivative, and so the curvature it adds,
/// runs on smoothly), each point moved along the line's normal.
pub fn shifted_points(chain: &Chain, st: &[Station], offsets: &[f64], step: f64) -> Vec<Pt> {
    let total = chain.length();
    let n = st.len();
    let h = if n > 1 { total / (n - 1) as f64 } else { total };
    let mm = spline_m(offsets, h);
    let m = ((total / step).ceil() as usize).max(1);
    (0..=m)
        .map(|k| {
            let s = total * k as f64 / m as f64;
            let x = (s / h).min((n - 1) as f64);
            let i = (x.floor() as usize).min(n.saturating_sub(2));
            let t = (x - i as f64) * h;
            let d = if n < 2 {
                offsets[0]
            } else {
                let (a, b) = (h - t, t);
                mm[i] * a * a * a / (6.0 * h)
                    + mm[i + 1] * b * b * b / (6.0 * h)
                    + (offsets[i] / h - mm[i] * h / 6.0) * a
                    + (offsets[i + 1] / h - mm[i + 1] * h / 6.0) * b
            };
            let q = chain.pose_at(s);
            [q.0 - d * libm::sin(q.2), q.1 + d * libm::cos(q.2)]
        })
        .collect()
}
