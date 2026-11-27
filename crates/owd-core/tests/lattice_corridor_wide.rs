//! A corridor wider than the baseline's tightest bend. The corridor of a pool is every point
//! within `corridor_half_m` of its baseline. It was taken as the buffer of the baseline, and that
//! buffer can come out EMPTY when the corridor is wider than a bend of the line (300 m round a bend
//! of radius 180 m): the raw offset curve then crosses itself, and for some coordinates resolving
//! it gives nothing (the same shape 70 m to the south buffers well). The search grid then covered
//! nothing, the search stopped after one expansion,
//! and the pool kept its baseline as "the search did not converge". `line_corridor` now builds the
//! corridor again as the union of the buffers of short stretches of the line when the buffer does
//! not hold the line, and a pool whose corridor still cannot be built says so by name.
//!
//! The baseline: 2.1 km cut from a line of 500 m straights between bends of 50 degrees: a right
//! bend at a radius of 320 m, then two left bends at 180 m. Its plain buffer at 300 m is empty.
//!
//! Label: property

use owd_core::geom::overlay::contains_xy;
use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, line_corridor, optimise_pools,
};

/// Straights and arcs in 5 m steps: `(length, radius, sign)`, radius 0 for a straight.
fn path(parts: &[(f64, f64, f64)]) -> Vec<Pt> {
    let (mut x, mut y, mut th) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut out = vec![[x, y]];
    for &(len, r, sgn) in parts {
        let n = (len / 5.0).ceil() as usize;
        let ds = len / n as f64;
        for _ in 0..n {
            let dth = if r > 0.0 { sgn * ds / r } else { 0.0 };
            let chord = if r > 0.0 {
                2.0 * r * libm::sin(0.5 * ds / r)
            } else {
                ds
            };
            x += chord * libm::cos(th + 0.5 * dth);
            y += chord * libm::sin(th + 0.5 * dth);
            th += dth;
            out.push([x, y]);
        }
    }
    out
}

fn baseline() -> Vec<Pt> {
    let turn = 50f64.to_radians();
    let bend = |r: f64, sgn: f64| (r * turn, r, sgn);
    let s = (500.0, 0.0, 0.0);
    let whole = path(&[
        s,
        bend(180.0, 1.0),
        s,
        bend(320.0, -1.0),
        s,
        bend(320.0, -1.0),
        s,
        bend(180.0, 1.0),
        s,
        bend(180.0, 1.0),
        s,
    ]);
    // 70 m north, as the case that first showed the fault: whether the buffer fails depends on
    // the coordinates as well as the shape.
    let whole: Vec<Pt> = whole.into_iter().map(|p| [p[0], p[1] + 70.0]).collect();
    match linear::substring(&whole, 1730.0, 3821.0) {
        owd_core::geom::Geometry::LineString(l) => l,
        other => panic!("a substring came out as {other:?}"),
    }
}

/// The corridor holds the line, reaches 0.95 of its half-width to both sides all along it, and
/// stops within 1.02 of it: at 300 m (wider than the 180 m bend) as at 100 m (narrower).
#[test]
fn the_corridor_of_a_line_with_a_tight_bend_is_never_empty() {
    let line = baseline();
    let len = linear::length(&line);
    for half in [100.0, 300.0] {
        let (corridor, ok) = line_corridor(&line, half).unwrap();
        assert!(ok && !corridor.is_empty(), "half {half}: no corridor");
        let mut s = 0.0;
        while s <= len {
            let p = linear::interpolate(&line, s).unwrap();
            let q = linear::interpolate(&line, (s + 1.0).min(len)).unwrap();
            let h = libm::atan2(q[1] - p[1], q[0] - p[0]);
            assert!(
                contains_xy(&corridor, p),
                "half {half}: the line at {s:.0} m"
            );
            for side in [-1.0, 1.0] {
                let at = |d: f64| {
                    [
                        p[0] - side * d * libm::sin(h),
                        p[1] + side * d * libm::cos(h),
                    ]
                };
                // A point 0.95 of the half-width to the side is within the half-width of the line.
                assert!(
                    contains_xy(&corridor, at(0.95 * half)),
                    "half {half}: {:?} beside {s:.0} m is outside",
                    at(0.95 * half)
                );
                // A point further than 1.02 of the half-width from the whole line is outside.
                let far = at(1.02 * half);
                if linear::point_line_distance(far, &line) > 1.02 * half - 1e-6 {
                    assert!(
                        !contains_xy(&corridor, far),
                        "half {half}: {far:?} is inside"
                    );
                }
            }
            s += 50.0;
        }
    }
}

/// The pool search on that baseline with a 300 m corridor searches: it does not stop after one
/// expansion, and it does not keep its baseline for want of a corridor.
#[test]
fn a_pool_with_a_corridor_wider_than_its_bend_is_searched() {
    let line = baseline();
    let len = linear::length(&line);
    let far = Polygon {
        exterior: vec![
            [1e6, 1e6],
            [1e6 + 1.0, 1e6],
            [1e6 + 1.0, 1e6 + 1.0],
            [1e6, 1e6],
        ],
        holes: vec![],
    };
    let flat = |xs: &[f64], _: &[f64]| vec![3.0; xs.len()];
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    spec.rake_half_m = 80.0;
    spec.rake_step_m = 8.0;
    spec.ds_step_m = 16.0;
    let mut cfg = LatticeConfig::new(340.0);
    cfg.pitch_m = 24.0;
    let r = optimise_pools(
        &line,
        &flat,
        core::slice::from_ref(&far),
        &[(0.0, len, 0.0)],
        3.25,
        &spec,
        &cfg.validated().unwrap(),
        None,
        PoolOptions {
            corridor_half_m: 300.0,
            cell_m: 8.0,
        },
        true,
    )
    .unwrap();
    let p = &r.pools[0];
    println!(
        "converged {}, {} expansions, used {}, {:?}",
        p.result.converged, p.result.expansions, p.used, p.kept_baseline_because
    );
    assert!(p.corridor_ok);
    assert!(p.result.converged && p.result.expansions > 10);
    assert!(p.used, "{:?}", p.kept_baseline_because);
}
