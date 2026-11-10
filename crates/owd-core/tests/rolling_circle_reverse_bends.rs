//! The rolling circle at reverse bends (issue #498): where an arc turning one way meets an arc
//! turning the other way, the rolled line has no corner, and it holds R_min on the default chord.
//!
//! The rivers are synthetic, built in the test: a chain of circular arcs that turn left and right
//! in turn (an S, then an S again), and sine centrelines, with a vertex every 10 m and a straight
//! at each end. Every bend is
//! tighter than R_min = 240 m, so the roll must straighten each one. The roll runs as the command
//! line runs it: step 10 m, two passes, both sides, the default gap and check chord.
//!
//! Before the fix, every one of these rivers kinked. On the chain of arcs of 120 m and 120 degrees
//! all 6 circles fell back to the crossing exit, the largest turn on 8 m stations was 17.4 degrees
//! (a 240 m circle turns 1.9 degrees there), and the radius read 91.0 m on the 80 m default chord.
//! On the sine of wavelength 600 m and amplitude 150 m, the largest turn at a change of direction
//! was 19.7 degrees and the radius read 75.2 m.
//!
//! Label: property (no corner where the turn changes direction; R_min held on the default chord).

use owd_core::alignment::limits::resample_line;
use owd_core::alignment::radius_check::{RADIUS_TOL, min_radius_on_chord};
use owd_core::alignment::rolling_circle::{RollingCircleResult, rolling_circle};
use owd_core::geom::{Geometry, Pt};
use owd_core::synthetic::sine_generated_centreline;

const R_MIN: f64 = 240.0;

/// The station spacing (m) the turns are read on.
const STATION_M: f64 = 8.0;

/// A turn on one station larger than this (degrees) is a kink. A circle of R_min turns
/// 2 asin(8 / 480) = 1.91 degrees on an 8 m station.
const KINK_DEG: f64 = 4.0;

fn heading(a: Pt, b: Pt) -> f64 {
    libm::atan2(b[1] - a[1], b[0] - a[0])
}

/// `a` wrapped to [-pi, pi).
fn wrap(a: f64) -> f64 {
    let two_pi = 2.0 * core::f64::consts::PI;
    let w = (a + core::f64::consts::PI).rem_euclid(two_pi);
    w - core::f64::consts::PI
}

/// A 300 m straight, then `n` circular arcs of radius `r` that turn left and right in turn, each
/// sweeping `sweep_deg` (half that at the first and the last), then a 300 m straight. A vertex
/// every `ds` metres along the arcs.
fn arc_chain(r: f64, sweep_deg: f64, n: usize, ds: f64) -> Vec<Pt> {
    let mut p: Pt = [0.0, 0.0];
    let mut h: f64 = 0.0;
    let mut pts = vec![p];
    let walk = |pts: &mut Vec<Pt>, p: &mut Pt, h: f64| {
        *p = [p[0] + ds * libm::cos(h), p[1] + ds * libm::sin(h)];
        pts.push(*p);
    };
    for _ in 0..30 {
        walk(&mut pts, &mut p, h);
    }
    for k in 0..n {
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        let part = if k == 0 || k == n - 1 { 0.5 } else { 1.0 };
        let total = sweep_deg.to_radians() * part;
        let steps = libm::round(total * r / ds) as usize;
        let dh = sign * total / steps as f64;
        for _ in 0..steps {
            h += dh;
            walk(&mut pts, &mut p, h);
        }
    }
    for _ in 0..30 {
        walk(&mut pts, &mut p, h);
    }
    pts
}

fn roll(line: Vec<Pt>) -> RollingCircleResult {
    let sides = vec!["left".to_string(), "right".to_string()];
    rolling_circle(
        &Geometry::LineString(line),
        R_MIN,
        10.0,
        2,
        &sides,
        None,
        None,
    )
    .unwrap()
}

/// The turn (degrees, signed, left positive) at each interior station of `line` on `STATION_M`
/// stations, with the station.
fn station_turns(line: &[Pt]) -> Vec<(f64, Pt)> {
    let st = resample_line(line, STATION_M).unwrap();
    st.windows(3)
        .map(|w| {
            (
                wrap(heading(w[1], w[2]) - heading(w[0], w[1])).to_degrees(),
                w[1],
            )
        })
        .collect()
}

/// The kinks of `line` that sit at a change of turn direction: a station turn over `KINK_DEG`
/// whose sign differs from the nearest clear turn (over 0.05 degree) before it or after it.
fn kinks_at_reverse(line: &[Pt]) -> Vec<(f64, Pt)> {
    let turns = station_turns(line);
    let clear = |t: &&(f64, Pt)| t.0.abs() > 0.05;
    (0..turns.len())
        .filter(|&i| turns[i].0.abs() > KINK_DEG)
        .filter(|&i| {
            let s = turns[i].0.signum();
            let before = turns[..i].iter().rev().find(clear).map(|t| t.0.signum());
            let after = turns[i + 1..].iter().find(clear).map(|t| t.0.signum());
            before.is_some_and(|b| b != s) || after.is_some_and(|a| a != s)
        })
        .map(|i| turns[i])
        .collect()
}

/// `line` with a straight of `len` metres before its start and after its end, each on the heading
/// of the line's end segment, a vertex every 10 m. A roll cannot straighten a bend that runs into
/// the end of the line (no circle can leave it), so each river ends on a straight.
fn with_straights(line: &[Pt], len: f64) -> Vec<Pt> {
    let n = libm::round(len / 10.0) as usize;
    let unit = |a: Pt, b: Pt| {
        let d = libm::hypot(b[0] - a[0], b[1] - a[1]);
        [(b[0] - a[0]) / d, (b[1] - a[1]) / d]
    };
    let (s, e) = (line[0], line[line.len() - 1]);
    let us = unit(line[1], s);
    let ue = unit(line[line.len() - 2], e);
    let mut out: Vec<Pt> = (1..=n)
        .rev()
        .map(|k| {
            [
                s[0] + 10.0 * k as f64 * us[0],
                s[1] + 10.0 * k as f64 * us[1],
            ]
        })
        .collect();
    out.extend_from_slice(line);
    out.extend((1..=n).map(|k| {
        [
            e[0] + 10.0 * k as f64 * ue[0],
            e[1] + 10.0 * k as f64 * ue[1],
        ]
    }));
    out
}

/// The synthetic rivers: (name, centreline).
fn rivers() -> Vec<(String, Vec<Pt>)> {
    let mut out = Vec::new();
    for (r, sweep) in [
        (120.0, 120.0),
        (100.0, 150.0),
        (150.0, 100.0),
        (180.0, 90.0),
    ] {
        out.push((
            format!("arcs of {r} m, {sweep} deg"),
            arc_chain(r, sweep, 6, 10.0),
        ));
    }
    for (wl, a) in [(600.0, 150.0), (700.0, 120.0), (900.0, 200.0)] {
        out.push((
            format!("sine of wavelength {wl} m, amplitude {a} m"),
            with_straights(&sine_generated_centreline(3000.0, wl, a, 10.0), 400.0),
        ));
    }
    out
}

/// The rivers are fair to start with: bends, but no corner of their own (the tightest bend, on the
/// sine of wavelength 600 m, has a radius of 61 m and turns 9.1 degrees on an 8 m station), and
/// each is tighter than R_min.
#[test]
fn each_old_line_is_smooth_and_tighter_than_r_min() {
    for (name, line) in rivers() {
        let worst = station_turns(&line)
            .iter()
            .map(|t| t.0.abs())
            .fold(0.0, f64::max);
        assert!(worst < 10.0, "{name}: the old line turns {worst:.2} deg");
        assert!(
            min_radius_on_chord(&line, R_MIN / 3.0).unwrap() < R_MIN,
            "{name}: the old line already holds R_min"
        );
    }
}

#[test]
fn no_kink_where_one_bend_turns_into_the_next() {
    let mut bad = Vec::new();
    for (name, line) in rivers() {
        let res = roll(line);
        for (t, p) in kinks_at_reverse(&res.centreline) {
            bad.push(format!("{name}: {t:+.2} deg at ({:.0}, {:.0})", p[0], p[1]));
        }
    }
    assert!(bad.is_empty(), "kinks at reverse bends: {bad:#?}");
}

#[test]
fn the_rolled_line_holds_r_min_on_the_default_chord() {
    let mut bad = Vec::new();
    for (name, line) in rivers() {
        let res = roll(line);
        // The default chord, R_min / 3 = 80 m, and the 120 m chord of the report.
        for chord in [R_MIN / 3.0, 120.0] {
            let r = min_radius_on_chord(&res.centreline, chord).unwrap();
            if r < (1.0 - RADIUS_TOL) * R_MIN {
                bad.push(format!("{name}: {r:.1} m on a {chord} m chord"));
            }
        }
        if !res.radius_ok {
            bad.push(format!(
                "{name}: radius_ok false, {:.1} m",
                res.measured_min_radius_m
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
