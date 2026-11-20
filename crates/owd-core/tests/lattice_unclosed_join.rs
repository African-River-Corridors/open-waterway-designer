//! A pool whose search cannot close on its goal pose (issue #565).
//!
//! The search ends on the goal pose exactly through a closing path of R_min. When it finds none it
//! falls back to a state within `goal_pos_tol_m` and one heading bin, and that line used to be spliced
//! into the joined line as it was: the join then steps sideways and reads far under R_min (or, with a
//! larger gap and a heading miss, the joined line can cross itself).
//!
//! The case is synthetic: a straight leg east, a bend of radius 260 m turning 90 degrees left, a
//! straight leg north, against R_min 340 m. The valley is narrow: known ground reaches 100 m on the
//! inside of the bend, 120 m on the outside downstream of mid-bend, and only 10 m on the outside
//! upstream of it; beyond that the ground is unknown. The pool boundary is at mid-bend. To end on the
//! boundary's pose (the bend's tangent there) the first pool would have to swing about 23 m out to the
//! outside upstream, where there is no ground, so it cannot close; it stops 16 m off the pose instead.
//! Moved downstream onto the straight leg, its end pose can be reached by cutting the corner.
//!
//! Label: property

use owd_core::alignment::loops::is_self_intersecting;
use owd_core::alignment::radius_check::radius_ok;
use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::{Geometry, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, SectionSpec, heading_at, optimise_pool,
    optimise_pools,
};

const R_MIN: f64 = 340.0;
const RB: f64 = 260.0;
const LEG: f64 = 1500.0;
const RIVER_HALF: f64 = 20.0;
const W_IN: f64 = 100.0;
const W_OUT_UP: f64 = 10.0;
const W_OUT_DN: f64 = 120.0;
const OPTS: PoolOptions = PoolOptions {
    corridor_half_m: 300.0,
    cell_m: 5.0,
};

/// A straight leg east, a bend of radius `RB` turning 90 degrees left, a straight leg north.
fn baseline() -> Vec<Pt> {
    let mut out: Vec<Pt> = (0..=(LEG / 5.0) as usize)
        .map(|i| [-LEG + i as f64 * 5.0, 0.0])
        .collect();
    let n = 80;
    for i in 1..=n {
        let t = core::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
        out.push([RB * libm::sin(t), RB * (1.0 - libm::cos(t))]);
    }
    for i in 1..=(LEG / 5.0) as usize {
        out.push([RB, RB + i as f64 * 5.0]);
    }
    out
}

/// The chainage of mid-bend.
fn mid_bend_m() -> f64 {
    LEG + RB * core::f64::consts::FRAC_PI_4
}

/// The valley: river (-12 m) within 20 m of the baseline, dry ground (+3 m) out to the valley's
/// edge, unknown (NaN) beyond.
fn terrain(base: Vec<Pt>) -> impl Fn(&[f64], &[f64]) -> Vec<f64> {
    let edge_at_m = mid_bend_m();
    move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let s = linear::project(&base, [x, y]);
                let q = linear::interpolate(&base, s).unwrap();
                let h = heading_at(&base, s, 5.0);
                let (dx, dy) = (x - q[0], y - q[1]);
                let d = libm::hypot(dx, dy);
                let left = libm::cos(h) * dy - libm::sin(h) * dx >= 0.0;
                let w = if left {
                    W_IN
                } else if s < edge_at_m {
                    W_OUT_UP
                } else {
                    W_OUT_DN
                };
                if d > w {
                    f64::NAN
                } else if d <= RIVER_HALF {
                    -12.0
                } else {
                    3.0
                }
            })
            .collect()
    }
}

fn spec() -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(10.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 60.0;
    spec
}

fn river(base: &[Pt]) -> Vec<owd_core::geom::Polygon> {
    match buffer_line(base, RIVER_HALF, 16, Cap::Flat).unwrap() {
        Geometry::Polygon(p) => vec![p],
        other => panic!("the river came out as {other:?}"),
    }
}

/// A lattice with a wide fallback tolerance (20 m), as a coarse search might be set.
fn cfg(close_reach_m: Option<f64>) -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.goal_pos_tol_m = Some(20.0);
    c.close_reach_m = close_reach_m;
    c.validated().unwrap()
}

/// Two pools at one level, split at `boundary_m`.
fn run(cfg: &LatticeConfig, boundary_m: f64) -> (Vec<Pt>, PoolsResult) {
    let base = baseline();
    let total = linear::length(&base);
    let t = terrain(base.clone());
    let pools = [(0.0, boundary_m, 0.0), (boundary_m, total, 0.0)];
    let r = optimise_pools(
        &base,
        &t,
        &river(&base),
        &pools,
        3.25,
        &spec(),
        cfg,
        None,
        OPTS,
        true,
    )
    .unwrap();
    (base, r)
}

fn describe(r: &PoolsResult) {
    for p in &r.pools {
        println!(
            "pool {:.0}..{:.0}: converged {}, closed {}, end {:.1} m / {:.1} deg off, used {}, {:?}",
            p.start_m,
            p.end_m,
            p.result.converged,
            p.result.goal_closed,
            p.result.end_err_m,
            p.result.end_heading_err_rad.to_degrees(),
            p.used,
            p.kept_baseline_because,
        );
    }
    println!("joins {:?}", r.joins);
}

fn assert_every_join_closed(r: &PoolsResult) {
    for j in &r.joins {
        assert!(
            j.position_gap_m < 1e-6 && j.heading_gap_rad < 1e-6,
            "the join at {:.0} m steps {:.1} m and {:.1} deg",
            j.at_m,
            j.position_gap_m,
            j.heading_gap_rad.to_degrees()
        );
    }
}

/// The fault: the first pool cannot close on the mid-bend pose. Its line must still not leave a
/// join far under R_min, a gap, or a loop in the joined line.
#[test]
fn a_pool_that_cannot_close_does_not_leave_a_broken_join() {
    let (_, r) = run(&cfg(None), mid_bend_m());
    describe(&r);
    assert_every_join_closed(&r);
    for j in &r.joins {
        assert!(
            j.radius_ok,
            "the join at {:.0} m reads {:.0} m against R_min {R_MIN} m",
            j.at_m, j.measured_min_radius_m
        );
    }
    assert!(
        !is_self_intersecting(&r.line),
        "the joined line crosses itself"
    );
    assert!(
        radius_ok(r.measured_min_radius_m, R_MIN),
        "the joined line reads {:.0} m",
        r.measured_min_radius_m
    );
    assert_eq!(r.n_replaced(), 2, "both pools still use their lattice line");
}

/// When no boundary closes (closing paths are only tried from 1 m away, so the search never closes),
/// the pool's line is not spliced in with a stepped join: every join left in the line is closed.
#[test]
fn a_pool_that_closes_nowhere_does_not_splice_a_stepped_join() {
    let (_, r) = run(&cfg(Some(1.0)), mid_bend_m());
    describe(&r);
    assert_every_join_closed(&r);
    assert!(
        !is_self_intersecting(&r.line),
        "the joined line crosses itself"
    );
}

/// Pools that close give the line they gave before: each pool's own line, spliced end to end.
/// The boundary sits 60 m upstream of mid-bend, where the first pool closes.
#[test]
fn pools_that_close_give_the_same_line() {
    let c = cfg(None);
    let at = mid_bend_m() - 60.0;
    let (base, r) = run(&c, at);
    describe(&r);
    assert!(r.pools.iter().all(|p| p.used && p.result.goal_closed));
    let total = linear::length(&base);
    let t = terrain(base.clone());
    let mut want: Vec<Pt> = Vec::new();
    for (a, b) in [(0.0, at), (at, total)] {
        let p = optimise_pool(
            &base,
            &t,
            &river(&base),
            a,
            b,
            0.0,
            3.25,
            &spec(),
            &c,
            None,
            OPTS,
        )
        .unwrap();
        for xy in p.result.line.unwrap() {
            if want
                .last()
                .is_none_or(|l| libm::hypot(xy[0] - l[0], xy[1] - l[1]) > 1e-9)
            {
                want.push(xy);
            }
        }
    }
    assert_eq!(r.line, want);
}
