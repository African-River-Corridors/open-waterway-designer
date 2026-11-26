//! Synthetic rivers for the river-following line's tests (spec OPT-4, section 10). Built in code:
//! no real river.

use crate::geom::{Polygon, Pt};
use core::f64::consts::PI;

/// A straight river `length` metres long along +x.
pub fn straight(length: f64) -> Vec<Pt> {
    let n = (length / 10.0).ceil() as usize;
    (0..=n)
        .map(|i| [length * i as f64 / n as f64, 0.0])
        .collect()
}

/// A sine river `y = a sin(2π x / wavelength)`, `length` metres along x, sampled every metre.
pub fn sine(a: f64, wavelength: f64, length: f64) -> Vec<Pt> {
    let n = length.ceil() as usize;
    (0..=n)
        .map(|i| {
            let x = length * i as f64 / n as f64;
            [x, a * libm::sin(2.0 * PI * x / wavelength)]
        })
        .collect()
}

/// The amplitude of a sine of `wavelength` whose bends have radius `r` at their apex.
pub fn sine_amplitude_for_radius(r: f64, wavelength: f64) -> f64 {
    let w = 2.0 * PI / wavelength;
    1.0 / (r * w * w)
}

/// A river of straights and circular bends: from the origin heading +x, each `(turn_rad, radius)`
/// a bend (+ left) after a straight of `straight` metres; a last straight. Sampled every metre.
pub fn bends(straight_m: f64, turns: &[(f64, f64)]) -> Vec<Pt> {
    let mut out = vec![[0.0, 0.0]];
    let (mut x, mut y, mut th) = (0.0, 0.0, 0.0);
    let mut go = |k: f64, len: f64, out: &mut Vec<Pt>| {
        let n = len.ceil().max(1.0) as usize;
        let step = len / n as f64;
        for _ in 0..n {
            let p = super::elements::advance((x, y, th), k, step);
            (x, y, th) = p;
            out.push([x, y]);
        }
    };
    for &(turn, r) in turns {
        go(0.0, straight_m, &mut out);
        go(turn.signum() / r, turn.abs() * r, &mut out);
    }
    go(0.0, straight_m, &mut out);
    out
}

/// The river polygon `width` metres wide about `centre`: its two banks offset along the normal.
pub fn polygon(centre: &[Pt], width: f64) -> Polygon {
    let n = centre.len();
    let h = width / 2.0;
    let normal = |i: usize| {
        let (a, b) = (i.saturating_sub(1), (i + 1).min(n - 1));
        let (dx, dy) = (centre[b][0] - centre[a][0], centre[b][1] - centre[a][1]);
        let d = libm::hypot(dx, dy).max(1e-12);
        [-dy / d, dx / d]
    };
    let mut ring: Vec<Pt> = (0..n)
        .map(|i| {
            let m = normal(i);
            [centre[i][0] + h * m[0], centre[i][1] + h * m[1]]
        })
        .collect();
    ring.extend((0..n).rev().map(|i| {
        let m = normal(i);
        [centre[i][0] - h * m[0], centre[i][1] - h * m[1]]
    }));
    ring.push(ring[0]);
    Polygon {
        exterior: ring,
        holes: vec![],
    }
}

/// A long meandering river: `n` bends of alternating hand (now and then two of one hand), radii
/// from `r_lo` to `r_hi`, turns from 20° to 160°, straights from 0 to `straight_hi` metres, from a
/// fixed seed. Sampled every metre.
pub fn meanders(n: usize, r_lo: f64, r_hi: f64, straight_hi: f64, seed: u64) -> Vec<Pt> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 11) as f64) / ((1u64 << 53) as f64)
    };
    let mut out = vec![[0.0, 0.0]];
    let (mut x, mut y, mut th) = (0.0, 0.0, 0.0);
    let mut go = |k: f64, len: f64, out: &mut Vec<Pt>| {
        let m = len.ceil().max(1.0) as usize;
        let step = len / m as f64;
        for _ in 0..m {
            (x, y, th) = super::elements::advance((x, y, th), k, step);
            out.push([x, y]);
        }
    };
    let mut hand = 1.0;
    let mut heading = 0.0f64;
    for _ in 0..n {
        let r = r_lo + (r_hi - r_lo) * next();
        let mut turn = (20.0 + 140.0 * next()).to_radians();
        // Keep the river's general direction near +x.
        if (heading + hand * turn).abs() > 100f64.to_radians() {
            hand = -heading.signum();
            turn = turn.min(heading.abs() + 60f64.to_radians());
        }
        go(0.0, straight_hi * next(), &mut out);
        go(hand / r, turn * r, &mut out);
        heading += hand * turn;
        if next() < 0.85 {
            hand = -hand;
        }
    }
    go(0.0, straight_hi, &mut out);
    out
}

/// An omega meander: a straight approach heading +x, a right bend of `side_turn` at radius `r`, a
/// left loop of twice that, a right bend of `side_turn`, a straight away. Sampled every metre.
/// At `r` 320 m and a side turn of 115° the neck is about 520 m wide and the river round the loop
/// about 1,570 m long (three times the neck).
pub fn omega(r: f64, side_turn: f64, approach_m: f64) -> Vec<Pt> {
    let mut out = vec![[0.0, 0.0]];
    let (mut x, mut y, mut th) = (0.0, 0.0, 0.0);
    let mut go = |k: f64, len: f64, out: &mut Vec<Pt>| {
        let m = len.ceil().max(1.0) as usize;
        let step = len / m as f64;
        for _ in 0..m {
            (x, y, th) = super::elements::advance((x, y, th), k, step);
            out.push([x, y]);
        }
    };
    go(0.0, approach_m, &mut out);
    go(-1.0 / r, side_turn * r, &mut out);
    go(1.0 / r, 2.0 * side_turn * r, &mut out);
    go(-1.0 / r, side_turn * r, &mut out);
    go(0.0, approach_m, &mut out);
    out
}
