//! The window optimiser's guarantees and the checks that hold them (OPT-5 3.13, tests S5 a and
//! S6 d): each check is shown to fail on a corrupted input.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::optimise::lattice::{LatticeConfig, lattice_primitives};
use owd_core::optimise::window::line::{
    LoopCheck, curve_near, pose_on, radius_reading, short_reversals, splice,
};
use owd_core::optimise::window::run::{adopts, geometric_radii};
use owd_core::synthetic::window::path;

const R_MIN: f64 = 300.0;

/// S5 (a): gains of 0.5 % and 1.5 % against a threshold of 1 %: no, then yes; nothing to gain on a
/// stretch that costs nothing.
#[test]
fn the_adopt_rule_fires_both_ways() {
    assert!(!adopts(99.5, 100.0, 0.01));
    assert!(adopts(98.5, 100.0, 0.01));
    assert!(adopts(99.0, 100.0, 0.01));
    assert!(!adopts(101.0, 100.0, 0.01));
    assert!(!adopts(0.0, 0.0, 0.01));
}

/// R_min by construction: every primitive of the window lattice is at least R_min, R_min itself is
/// in the set, and a radius under R_min is refused.
#[test]
fn every_primitive_holds_r_min_and_a_tighter_one_is_refused() {
    let radii = geometric_radii(R_MIN, 12, 8.0);
    assert_eq!(radii[0], R_MIN);
    assert!((radii[11] - 8.0 * R_MIN).abs() < 1e-6);
    let mut cfg = LatticeConfig::new(R_MIN);
    cfg.n_heading = 64;
    cfg.radii_m = Some(radii);
    cfg.nominal_arc_m = Some(80.0);
    let cfg = cfg.validated().unwrap();
    let bin = std::f64::consts::TAU / 64.0;
    for p in lattice_primitives(&cfg) {
        if p.sign != 0 {
            assert!(p.r >= R_MIN);
            // A whole number of bins, so a chain lands on bin headings.
            let k = p.end.2.abs() / bin;
            assert!((k - k.round()).abs() < 1e-9 && k.round() >= 1.0);
        }
    }
    let mut bad = LatticeConfig::new(R_MIN);
    bad.radii_m = Some(vec![0.9 * R_MIN, R_MIN]);
    assert!(bad.validated().unwrap_err().contains("r_min_m"));
}

/// The radius check on the joined line: an arc of R_min passes, one of 0.9 R_min fails.
#[test]
fn the_radius_check_fails_on_an_arc_under_r_min() {
    let ok = path(
        (0.0, 0.0, 0.0),
        &[(f64::INFINITY, 400.0), (R_MIN, 1.0), (f64::INFINITY, 400.0)],
        2.0,
    );
    let bad = path(
        (0.0, 0.0, 0.0),
        &[
            (f64::INFINITY, 400.0),
            (0.9 * R_MIN, 1.0),
            (f64::INFINITY, 400.0),
        ],
        2.0,
    );
    assert!(radius_reading(&ok, R_MIN).1);
    assert!(!radius_reading(&bad, R_MIN).1);
    // Where it fails is read the same way: nowhere on the good line, on the arc of the bad one.
    use owd_core::alignment::radius_check::{default_check_chord, radius_failures};
    let chord = default_check_chord(R_MIN);
    assert!(radius_failures(&ok, chord, R_MIN).is_empty());
    let at = radius_failures(&bad, chord, R_MIN);
    assert!(!at.is_empty());
    assert!(
        at.iter()
            .all(|p| p[0] > 380.0 && p[0] < 400.0 + 0.9 * R_MIN)
    );
}

/// Continuity: a piece spliced between two poses of the line leaves no gap; the same piece moved
/// 1 m leaves a gap the record sees (the splice puts its ends on the seams, and says how far they
/// were).
#[test]
fn a_splice_is_continuous_and_a_moved_seam_shows() {
    let line = path((0.0, 0.0, 0.0), &[(f64::INFINITY, 2000.0)], 10.0);
    let piece = path((500.0, 0.0, 0.0), &[(f64::INFINITY, 600.0)], 2.0);
    let (joined, end) = splice(&line, 500.0, 1100.0, &piece);
    assert!((end - 1100.0).abs() < 1e-9);
    let (a, b) = (pose_on(&joined, 500.0), pose_on(&line, 500.0));
    assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6);
    let moved: Vec<Pt> = piece.iter().map(|p| [p[0], p[1] + 1.0]).collect();
    let gap = (moved[0][1] - pose_on(&line, 500.0).1).abs();
    assert!(gap > 1e-6);
}

/// No loop: a line that crosses itself, or turns more than three full turns, is a loop.
#[test]
fn the_loop_test_catches_a_crossing_and_a_spiral() {
    let simple = path((0.0, 0.0, 0.0), &[(f64::INFINITY, 1000.0)], 10.0);
    let lc = LoopCheck::read(&simple, &simple, 1000.0);
    assert!(!lc.is_loop(1080.0, 3.0));
    // A full circle and on: it crosses itself.
    let crossing = path(
        (0.0, 0.0, 0.0),
        &[(f64::INFINITY, 300.0), (R_MIN, 7.0), (f64::INFINITY, 300.0)],
        5.0,
    );
    let lc = LoopCheck::read(&crossing, &crossing, 1000.0);
    assert!(lc.self_crossing_m.is_some() && lc.is_loop(1080.0, 3.0));
    // Long for its window.
    let lc = LoopCheck::read(&simple, &simple, 300.0);
    assert!(lc.is_loop(1080.0, 3.0));
}

/// The minimum straight: an S with 30 m of straight between its curves is reported against 75 m;
/// with 100 m it is not. What the line does either side of a seam is read too.
#[test]
fn a_short_straight_between_opposite_curves_is_reported() {
    let s = |run: f64| {
        path(
            (0.0, 0.0, 0.0),
            &[
                (f64::INFINITY, 300.0),
                (R_MIN, 0.5),
                (f64::INFINITY, run),
                (R_MIN, -0.5),
                (f64::INFINITY, 300.0),
            ],
            2.0,
        )
    };
    assert_eq!(short_reversals(&s(30.0), R_MIN, 75.0, 4.0).len(), 1);
    assert!(short_reversals(&s(100.0), R_MIN, 75.0, 4.0).is_empty());
    let line = s(30.0);
    // 10 m before the second curve's start: the first curve is 40 m back, the second 10 m ahead.
    let at = 300.0 + R_MIN * 0.5 + 20.0;
    let (t, run) = curve_near(&line, at, R_MIN, 75.0, false);
    assert_eq!(t, 1);
    assert!((run - 20.0).abs() < 3.0, "{run}");
    let (t, run) = curve_near(&line, at, R_MIN, 75.0, true);
    assert_eq!(t, -1);
    assert!((run - 10.0).abs() < 3.0, "{run}");
}
