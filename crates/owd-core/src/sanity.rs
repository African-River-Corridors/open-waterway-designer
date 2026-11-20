//! The design sanity check: three plain faults a design line or its cut can have, found after the
//! alignment and after the volumes, and reported as warnings (never as a refusal, and never as a
//! failed check that changes an exit code).
//!
//! 1. The line **crosses itself**.
//! 2. A **bend is under R_min** on the check chord.
//! 3. A **station's cut is far above its neighbours'**.
//!
//! Pure: no I/O. Quantities in metres and square metres.

use crate::alignment::navigable::RadiusOn;
use crate::alignment::radius_check::radius_ok;
use crate::geom::{Pt, linear};

/// Station spacing of the radius read along the line (m).
const RADIUS_STEP_M: f64 = 10.0;

/// A station's cut is far above its neighbours' when it is more than this many times their median.
pub const SPIKE_RATIO: f64 = 3.0;
/// ... and above it by more than this (m2), so that small cuts beside a near-zero median are not
/// flagged.
pub const SPIKE_MIN_EXCESS_M2: f64 = 100.0;
/// The neighbours read each side of a station.
pub const SPIKE_NEIGHBOURS: usize = 5;

/// What was found.
#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// The line crosses itself first at this point, `chainage_m` along it.
    SelfCrossing { at: Pt, chainage_m: f64 },
    /// A stretch where the radius on the check chord is under R_min.
    BendUnderRMin {
        from_m: f64,
        to_m: f64,
        min_radius_m: f64,
        r_min_m: f64,
    },
    /// A stretch of stations whose cut is far above their neighbours'.
    CutSpike {
        from_m: f64,
        to_m: f64,
        max_cut_m2: f64,
        neighbours_median_m2: f64,
    },
}

impl Finding {
    /// One plain sentence.
    pub fn message(&self) -> String {
        match self {
            Finding::SelfCrossing { at, chainage_m } => format!(
                "the design line crosses itself near {chainage_m:.0} m along the line, at \
                 ({:.1}, {:.1}): a loop is not a design",
                at[0], at[1]
            ),
            Finding::BendUnderRMin {
                from_m,
                to_m,
                min_radius_m,
                r_min_m,
            } => format!(
                "a bend between {from_m:.0} m and {to_m:.0} m is tighter than R_min: radius \
                 {min_radius_m:.0} m against {r_min_m:.0} m"
            ),
            Finding::CutSpike {
                from_m,
                to_m,
                max_cut_m2,
                neighbours_median_m2,
            } => format!(
                "the cut between {from_m:.0} m and {to_m:.0} m reaches {max_cut_m2:.0} m2 a \
                 station, far above its neighbours' {neighbours_median_m2:.0} m2: check the line \
                 and the terrain there"
            ),
        }
    }
}

/// The first point at which two segments of `line` that are not neighbours cross, and the chainage
/// of the earlier one. `None` when the line is simple.
fn first_crossing(line: &[Pt]) -> Option<(Pt, f64)> {
    let n = line.len();
    let mut cum = Vec::with_capacity(n);
    let mut s = 0.0;
    cum.push(0.0);
    for w in line.windows(2) {
        s += crate::geom::dist(w[0], w[1]);
        cum.push(s);
    }
    let cross = |a: Pt, b: Pt, c: Pt, d: Pt| -> Option<(Pt, f64)> {
        let (r, q) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
        let den = r[0] * q[1] - r[1] * q[0];
        if den == 0.0 {
            return None;
        }
        let t = ((c[0] - a[0]) * q[1] - (c[1] - a[1]) * q[0]) / den;
        let u = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
        ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
            .then(|| ([a[0] + t * r[0], a[1] + t * r[1]], t))
    };
    for i in 0..n.saturating_sub(1) {
        for j in i + 2..n.saturating_sub(1) {
            if let Some((p, t)) = cross(line[i], line[i + 1], line[j], line[j + 1]) {
                let seg = cum[i + 1] - cum[i];
                return Some((p, cum[i] + t * seg));
            }
        }
    }
    None
}

/// The findings on a design line: a self-crossing, and each stretch where the radius read on
/// `chord_m` is under `r_min_m` (within the usual reading tolerance).
pub fn line_findings(line: &[Pt], chord_m: f64, r_min_m: f64) -> Vec<Finding> {
    let mut out = Vec::new();
    if line.len() < 3 {
        return out;
    }
    if crate::alignment::loops::is_self_intersecting(line)
        && let Some((at, chainage_m)) = first_crossing(line)
    {
        out.push(Finding::SelfCrossing { at, chainage_m });
    }
    if !(r_min_m.is_finite() && r_min_m > 0.0 && chord_m.is_finite() && chord_m > 0.0) {
        return out;
    }
    let len = linear::length(line);
    let radius = RadiusOn::new(line);
    let mut stretch: Option<(f64, f64, f64)> = None;
    let mut s: f64 = 0.0;
    loop {
        let at = s.min(len);
        let r = radius.at(at, chord_m);
        if r.is_finite() && !radius_ok(r, r_min_m) {
            stretch = Some(match stretch {
                Some((a, _, m)) => (a, at, m.min(r)),
                None => (at, at, r),
            });
        } else if let Some((a, b, m)) = stretch.take() {
            out.push(Finding::BendUnderRMin {
                from_m: a,
                to_m: b,
                min_radius_m: m,
                r_min_m,
            });
        }
        if s >= len {
            break;
        }
        s += RADIUS_STEP_M;
    }
    if let Some((a, b, m)) = stretch {
        out.push(Finding::BendUnderRMin {
            from_m: a,
            to_m: b,
            min_radius_m: m,
            r_min_m,
        });
    }
    out
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// The stretches of stations whose cut is far above their neighbours': more than
/// [`SPIKE_RATIO`] times the median of the [`SPIKE_NEIGHBOURS`] stations each side, and above it
/// by more than [`SPIKE_MIN_EXCESS_M2`]. `cuts` is `(chainage_m, cut_m2)` per station, in order.
/// A cut that is high all along a stretch is not a spike: its neighbours are high too.
pub fn cut_spikes(cuts: &[(f64, f64)]) -> Vec<Finding> {
    let n = cuts.len();
    let mut flagged: Vec<(usize, f64)> = Vec::new();
    for i in 0..n {
        if !cuts[i].1.is_finite() {
            continue;
        }
        let lo = i.saturating_sub(SPIKE_NEIGHBOURS);
        let hi = (i + SPIKE_NEIGHBOURS + 1).min(n);
        let mut near: Vec<f64> = (lo..hi)
            .filter(|&j| j != i && cuts[j].1.is_finite())
            .map(|j| cuts[j].1)
            .collect();
        if near.len() < SPIKE_NEIGHBOURS {
            continue;
        }
        let m = median(&mut near);
        if cuts[i].1 > SPIKE_RATIO * m && cuts[i].1 - m > SPIKE_MIN_EXCESS_M2 {
            flagged.push((i, m));
        }
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < flagged.len() {
        let mut e = k;
        while e + 1 < flagged.len() && flagged[e + 1].0 == flagged[e].0 + 1 {
            e += 1;
        }
        let run = &flagged[k..=e];
        let max_cut = run.iter().map(|&(i, _)| cuts[i].1).fold(0.0, f64::max);
        let med = run.iter().map(|&(_, m)| m).sum::<f64>() / run.len() as f64;
        out.push(Finding::CutSpike {
            from_m: cuts[run[0].0].0,
            to_m: cuts[run[run.len() - 1].0].0,
            max_cut_m2: max_cut,
            neighbours_median_m2: med,
        });
        k = e + 1;
    }
    out
}
