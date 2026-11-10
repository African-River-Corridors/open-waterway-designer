//! The bank checks (spec 0013, ticket T1), moved to the core (ticket B3a) so they apply to any
//! banks, whatever made them: each bank on its own side, no crossing, a plausible width, no jump and
//! no excursion. They read two bank lines and a rough centreline, in metres, and change nothing. The
//! repairs stay with the bank extraction (`owd_reach::bankcheck`).
//!
//! Label: property

use super::stations::{SegIndex, arc, bank_offsets, median_width, resample};

type P2 = [f64; 2];

/// The settings of the checks. Every value here is a provisional default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckParams {
    /// The distance between stations, in metres.
    pub station_m: f64,
    /// The largest change of a bank's offset from one station to the next, in station steps.
    pub jump_factor: f64,
    /// A run of jumped stations longer than this, or one that runs to the end of the banks, is a real
    /// change of the river and is kept.
    pub max_run_m: f64,
    /// The half-length of the running median of the width, in metres.
    pub width_window_m: f64,
    pub width_low: f64,
    pub width_high: f64,
    /// An excursion stands outward of the running median by more than this share of the median width,
    /// and at least `excursion_min_m`.
    pub excursion_share: f64,
    pub excursion_min_m: f64,
    /// The half-length of the running median an excursion stands out of, in metres.
    pub excursion_window_m: f64,
    /// The most a repaired bank's offset may change from one station to the next, in metres (the edge
    /// fit's step limit); a repair too short to keep to it is widened until it does. `None`: no limit.
    pub max_step_m: Option<f64>,
    /// The half-length of the moving average that makes the reference line, in median river widths
    /// (at least two stations, at most 2 km).
    pub reference_widths: f64,
    /// The half-length used when no width can be measured, in metres.
    pub reference_half_m: f64,
    /// Where the rebuilt banks cross, the stations around the crossing narrower than this are a
    /// pinch: both banks are repaired across it in the next round (issue #410).
    pub pinch_width_m: f64,
}

impl CheckParams {
    /// The defaults at a station step.
    pub fn at_step(station_m: f64) -> CheckParams {
        CheckParams {
            station_m,
            jump_factor: 2.0,
            max_run_m: 1_000.0,
            width_window_m: 500.0,
            width_low: 0.5,
            width_high: 2.0,
            excursion_share: 0.25,
            excursion_min_m: 30.0,
            excursion_window_m: 250.0,
            max_step_m: None,
            reference_widths: 1.0,
            reference_half_m: 250.0,
            pinch_width_m: 10.0,
        }
    }

    /// The largest change of offset between stations, in metres.
    pub fn max_jump_m(&self) -> f64 {
        self.jump_factor * self.station_m
    }
}

/// Which checks a station failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Checks {
    /// A bank on the wrong side of the river.
    pub side: bool,
    /// The normal met no bank.
    pub missing: bool,
    /// A bank jumped outward past the limit (a spike, an inlet, a tributary mouth): repaired.
    pub jump: bool,
    /// Wider than its neighbours allow: repaired.
    pub width_wide: bool,
    /// Narrower than its neighbours: flagged, the banks kept.
    pub width_narrow: bool,
    /// A bank standing outward of its neighbours over a short run (a bay, a tributary mouth, a cut
    /// across an island into another channel), rising too gradually to be a jump: repaired.
    pub excursion: bool,
    /// The banks still cross here after every round of repairs: not repaired, and said loudly.
    pub crossing: bool,
}

impl Checks {
    pub fn any(self) -> bool {
        self.side
            || self.missing
            || self.jump
            || self.width_wide
            || self.width_narrow
            || self.excursion
            || self.crossing
    }

    pub fn or(self, o: Checks) -> Checks {
        Checks {
            side: self.side || o.side,
            missing: self.missing || o.missing,
            jump: self.jump || o.jump,
            width_wide: self.width_wide || o.width_wide,
            width_narrow: self.width_narrow || o.width_narrow,
            excursion: self.excursion || o.excursion,
            crossing: self.crossing || o.crossing,
        }
    }

    /// The names, as the document writes them.
    pub fn names(self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (on, name) in [
            (self.side, "side"),
            (self.missing, "missing"),
            (self.jump, "jump"),
            (self.width_wide, "width-wide"),
            (self.width_narrow, "width-narrow"),
            (self.excursion, "excursion"),
            (self.crossing, "crossing"),
        ] {
            if on {
                v.push(name);
            }
        }
        v
    }
}

/// A moving average over `k` stations either side, the window shrunk evenly near the ends, so the end
/// points stay where they are and the line keeps its length.
pub fn moving_average(pts: &[P2], k: usize) -> Vec<P2> {
    let n = pts.len();
    let mut pre = vec![[0.0, 0.0]; n + 1];
    for i in 0..n {
        pre[i + 1] = [pre[i][0] + pts[i][0], pre[i][1] + pts[i][1]];
    }
    (0..n)
        .map(|i| {
            let h = k.min(i).min(n - 1 - i);
            let (a, b) = (i - h, i + h);
            let m = (b - a + 1) as f64;
            [
                (pre[b + 1][0] - pre[a][0]) / m,
                (pre[b + 1][1] - pre[a][1]) / m,
            ]
        })
        .collect()
}

pub fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    })
}

/// Stations of one bank that jumped outward, away from the river (`None` is already bad). `out` is
/// the sign of outward: +1 for the left bank, -1 for the right. A jump inward narrows the river: it
/// is never cut here (the width check flags it), so a real throat is never widened away.
pub fn jumps(off: &[Option<f64>], out: f64, p: &CheckParams) -> Vec<bool> {
    let lim = p.max_jump_m();
    let max_run = (p.max_run_m / p.station_m).ceil() as usize;
    let mut bad = vec![false; off.len()];
    // Start from the first station that agrees with the running median of its neighbours, so an
    // outlier at the start (a hooked bank end) does not make everything after it look like a jump.
    // The stations before it are not judged: like a run that reaches the end, there is nothing
    // beyond them to judge them against.
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let med = |i: usize| {
        let (a, b) = (i.saturating_sub(kw), (i + kw).min(off.len() - 1));
        let mut v: Vec<f64> = off[a..=b].iter().flatten().copied().collect();
        median(&mut v)
    };
    let Some(start) = (0..off.len())
        .find(|i| off[*i].is_some_and(|o| med(*i).is_some_and(|m| (o - m).abs() <= lim)))
    else {
        return bad;
    };
    let mut last_good: Option<(usize, f64)> = None;
    let mut i = start;
    while i < off.len() {
        let Some(o) = off[i] else {
            i += 1;
            continue;
        };
        match last_good {
            Some((a, g)) if out * (o - g) > lim * (i - a) as f64 => {
                // A run: until a station comes back within the limit of the offset before it.
                let mut j = i;
                while j < off.len() && off[j].is_none_or(|v| out * (v - g) > lim) {
                    j += 1;
                }
                if j - i > max_run || j == off.len() {
                    // A real change of the river: keep it, from its first station.
                    last_good = Some((i, o));
                    i += 1;
                    continue;
                }
                for b in bad.iter_mut().take(j).skip(i) {
                    *b = true;
                }
                i = j;
            }
            _ => {
                last_good = Some((i, o));
                i += 1;
            }
        }
    }
    bad
}

/// Whether two lines cross each other.
pub fn lines_cross(a: &[P2], b: &[P2]) -> Option<P2> {
    if a.len() < 2 || b.len() < 2 {
        return None;
    }
    let idx = SegIndex::new(b, 50.0);
    let mut buf = Vec::new();
    for w in a.windows(2) {
        let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let d = [(w[1][0] - w[0][0]) / len, (w[1][1] - w[0][1]) / len];
        // A crossing is a hit along the segment's own direction within its length.
        let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
        if let Some(t) = idx.nearest_hit(mid, d, len / 2.0, &mut buf) {
            return Some([mid[0] + d[0] * t, mid[1] + d[1] * t]);
        }
    }
    None
}

/// Every point where line `a` crosses line `b`, in the order of `a`'s segments.
pub fn crossing_points(a: &[P2], b: &[P2]) -> Vec<P2> {
    if a.len() < 2 || b.len() < 2 {
        return Vec::new();
    }
    let idx = SegIndex::new(b, 50.0);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    for w in a.windows(2) {
        let len = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let d = [(w[1][0] - w[0][0]) / len, (w[1][1] - w[0][1]) / len];
        let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
        if let Some(t) = idx.nearest_hit(mid, d, len / 2.0, &mut buf) {
            out.push([mid[0] + d[0] * t, mid[1] + d[1] * t]);
        }
    }
    out
}

pub type Assessed = (
    Vec<P2>,
    Vec<(Option<f64>, Option<f64>)>,
    Vec<Checks>,
    Vec<bool>,
    Vec<bool>,
);

/// Check the banks against the reference made from `seed` (any rough centreline): per station of the
/// reference, its chainage, the two offsets and the checks failed. Nothing is changed. `pinches` are
/// points where the banks cross (from the last round): the stations around each that are narrower
/// than `pinch_width_m` are repaired on both banks, flagged `side` (issue #410).
pub fn assess(left: &[P2], right: &[P2], seed: &[P2], p: &CheckParams, pinches: &[P2]) -> Assessed {
    // The reference comes from the seed, not from the mid-line of the banks under test: banks that
    // cross back and forth would give a mid-line that does too.
    let m0 = resample(seed, p.station_m);
    // Smoothed over about one river width: enough that one inlet cannot drag it and that it does not
    // curl in a wide river, little enough that it stays on the water of a narrow, winding one.
    let half = median_width(&m0, left, right).map_or(p.reference_half_m, |w| {
        (w * p.reference_widths).clamp(2.0 * p.station_m, 2_000.0)
    });
    let k = (half / p.station_m).round() as usize;
    let reference = resample(&moving_average(&m0, k), p.station_m);
    let off = bank_offsets(&reference, left, right);
    let n = reference.len();
    let mut checks = vec![Checks::default(); n];
    let (mut bad_l, mut bad_r) = (vec![false; n], vec![false; n]);
    // 1. Own side, and a normal that meets no bank.
    let mut lo: Vec<Option<f64>> = Vec::with_capacity(n);
    let mut ro: Vec<Option<f64>> = Vec::with_capacity(n);
    // A normal past the end of a bank meets nothing: that is where the bank ends, not a fault. Only
    // stations between the first and the last that meet both banks are checked.
    let both = |i: usize| off[i].0.is_some() && off[i].1.is_some();
    let first = (0..n).find(|i| both(*i)).unwrap_or(n);
    let last = (0..n).rev().find(|i| both(*i)).unwrap_or(0);
    // The banks are out of order where the left one lies at or right of the right one along the
    // normal: one has crossed to the other shore. A reference that strays off a narrow river puts
    // both banks on one side of it, in order: that is not a fault, and repairing it would widen the
    // river.
    for (i, (l, r)) in off.iter().enumerate() {
        if i < first || i > last {
            lo.push(*l);
            ro.push(*r);
            continue;
        }
        if l.is_none() || r.is_none() {
            checks[i].missing = true;
        }
        let swapped = matches!((l, r), (Some(a), Some(b)) if a <= b);
        if swapped {
            checks[i].side = true;
        }
        bad_l[i] = l.is_none() || swapped;
        bad_r[i] = r.is_none() || swapped;
        lo.push(l.filter(|_| !swapped));
        ro.push(r.filter(|_| !swapped));
    }
    // 1b. Pinches: where the banks crossed, both banks lie together, a few metres apart or swapped
    // between the stations. That is a wrong bank, not a throat: from the station nearest the crossing,
    // both banks are repaired out to the first stations either side that are wider than
    // `pinch_width_m`, so the repair spans the pinch from real width on both sides. Only a pinch is
    // repaired: a crossing whose nearest station is not narrow, a run that meets a station with no
    // bank, or one longer than `max_run_m`, is left to the `crossing` flag.
    let narrow = |i: usize| matches!(off[i], (Some(l), Some(r)) if l - r < p.pinch_width_m);
    for x in pinches {
        let Some(i) = (first..=last.max(first).min(n.saturating_sub(1))).min_by(|a, b| {
            let d = |k: usize| (reference[k][0] - x[0]).powi(2) + (reference[k][1] - x[1]).powi(2);
            d(*a).total_cmp(&d(*b))
        }) else {
            continue;
        };
        if !narrow(i) {
            continue;
        }
        let (mut a, mut b) = (i, i);
        while a > first && narrow(a - 1) {
            a -= 1;
        }
        while b < last && narrow(b + 1) {
            b += 1;
        }
        let ends_have_banks = |k: usize| off[k].0.is_some() && off[k].1.is_some();
        if (b - a) as f64 * p.station_m > p.max_run_m
            || a == first
            || b == last
            || !ends_have_banks(a - 1)
            || !ends_have_banks(b + 1)
        {
            continue;
        }
        for j in a..=b {
            // A bank on the wrong side between the stations: `side`. The check `crossing` stays for a
            // crossing left after the last round.
            checks[j].side = true;
            bad_l[j] = true;
            bad_r[j] = true;
            lo[j] = None;
            ro[j] = None;
        }
    }
    // 2. Lateral jumps, per bank.
    for (o, bad, out) in [(&mut lo, &mut bad_l, 1.0), (&mut ro, &mut bad_r, -1.0)] {
        let j = jumps(o, out, p);
        for i in 0..n {
            if j[i] {
                checks[i].jump = true;
                bad[i] = true;
                o[i] = None;
            }
        }
    }
    // 2b. Excursions: a bank outward of the running median of its own offsets (over
    // `excursion_window_m` either side) by more than a share of the local median width.
    let ww: Vec<Option<f64>> = (0..n).map(|i| Some(lo[i]? - ro[i]?)).collect();
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let local_width: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(kw), (i + kw).min(n - 1));
            let mut v: Vec<f64> = ww[a..=b].iter().flatten().copied().collect();
            median(&mut v)
        })
        .collect();
    let ke = (p.excursion_window_m / p.station_m).round() as usize;
    for (o, bad, out) in [(&mut lo, &mut bad_l, 1.0), (&mut ro, &mut bad_r, -1.0)] {
        let med: Vec<Option<f64>> = (0..n)
            .map(|i| {
                let (a, b) = (i.saturating_sub(ke), (i + ke).min(n - 1));
                let mut v: Vec<f64> = o[a..=b].iter().flatten().copied().collect();
                median(&mut v)
            })
            .collect();
        for i in 0..n {
            if let (Some(v), Some(m), Some(w)) = (o[i], med[i], local_width[i])
                && out * (v - m) > (p.excursion_share * w).max(p.excursion_min_m)
            {
                checks[i].excursion = true;
                bad[i] = true;
                o[i] = None;
            }
        }
    }
    // 3. Width, against the running median of the good widths.
    let w: Vec<Option<f64>> = (0..n).map(|i| Some(lo[i]? - ro[i]?)).collect();
    let kw = (p.width_window_m / p.station_m).round() as usize;
    let running = |v: &[Option<f64>], i: usize| {
        let (a, b) = (i.saturating_sub(kw), (i + kw).min(n - 1));
        let mut s: Vec<f64> = v[a..=b].iter().flatten().copied().collect();
        median(&mut s)
    };
    for i in 0..n {
        let (Some(wi), Some(med)) = (w[i], running(&w, i)) else {
            continue;
        };
        if wi > p.width_high * med {
            checks[i].width_wide = true;
            let dl = running(&lo, i).map_or(0.0, |m| (lo[i].unwrap_or(m) - m).abs());
            let dr = running(&ro, i).map_or(0.0, |m| (ro[i].unwrap_or(m) - m).abs());
            if dl >= dr {
                bad_l[i] = true;
            } else {
                bad_r[i] = true;
            }
        } else if wi < p.width_low * med {
            checks[i].width_narrow = true;
        }
    }
    (reference, off, checks, bad_l, bad_r)
}

/// The failures of a pair of banks, without repairing them: chainage along the reference and checks.
pub fn check_banks(left: &[P2], right: &[P2], seed: &[P2], p: &CheckParams) -> Vec<(f64, Checks)> {
    let (reference, _, checks, _, _) = assess(left, right, seed, p, &[]);
    let s = arc(&reference);
    checks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.any())
        .map(|(i, c)| (s[i], *c))
        .collect()
}
