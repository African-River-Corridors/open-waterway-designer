//! The corridor as a real limit: `LatticeConfig::corridor_limit`.
//!
//! By default the corridor (`corridor_half_m` round the baseline) only says where the heuristic
//! samples; the search may go anywhere on the grid, which is the corridor's bounding rectangle. So
//! on open ground a line can be found hundreds of metres outside the corridor. With the limit on, no
//! state, no point of a primitive and no point of a closing path may lie outside the corridor, and
//! a pool that has no line inside it keeps its baseline and says why.
//!
//! The cases are synthetic: a flat plain at -10 m under a pool at 0 m with a 3.5 m channel, so no
//! ground anywhere needs cutting, and a cost of 1 per metre of line, so the cheapest line is the
//! shortest. The baseline is a sine meander; the river polygon is 120 m wide round it and plays no
//! part (no land cost). R_min is 340 m; the grid cell is 10 m, and the corridor mask is read at the
//! nearest cell, so "inside" is to within one cell.
//!
//! Label: property

use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, NO_LINE_IN_CORRIDOR_REASON, PoolOptions, PoolsResult,
    SectionSpec, UNCLOSED_IN_CORRIDOR_REASON, optimise_pools,
};

const R_MIN: f64 = 340.0;
const DEPTH: f64 = 3.5;
const CELL: f64 = 10.0;

/// A sine meander of `th0` degrees, in 5 m steps.
fn centreline(th0_deg: f64, lam: f64, len: f64) -> Vec<Pt> {
    let th0 = th0_deg.to_radians();
    let ds = 5.0;
    let n = (len / ds) as usize;
    let mut out = vec![[0.0, 0.0]];
    for i in 0..n {
        let th = th0 * libm::sin(2.0 * core::f64::consts::PI * (i as f64 * ds) / lam);
        let [x, y] = out[out.len() - 1];
        out.push([x + ds * libm::cos(th), y + ds * libm::sin(th)]);
    }
    out
}

fn river(cl: &[Pt]) -> Vec<Polygon> {
    match buffer_line(cl, 60.0, 16, Cap::Round).unwrap() {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(p) => p,
        other => panic!("the river came out as {other:?}"),
    }
}

fn plain(xs: &[f64], _: &[f64]) -> Vec<f64> {
    vec![-10.0; xs.len()]
}

fn spec() -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 80.0;
    spec.rake_step_m = 4.0;
    spec.ds_step_m = 8.0;
    spec.length_weight = 1.0;
    spec
}

fn cfg(limit: bool) -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.pitch_m = 20.0;
    c.corridor_limit = limit;
    c.validated().unwrap()
}

fn run(cl: &[Pt], pools: &[(f64, f64, f64)], half: f64, limit: bool) -> PoolsResult {
    optimise_pools(
        cl,
        &plain,
        &river(cl),
        pools,
        DEPTH,
        &spec(),
        &cfg(limit),
        None,
        PoolOptions {
            corridor_half_m: half,
            cell_m: CELL,
        },
        true,
    )
    .unwrap()
}

/// The furthest `line` goes from `cl` (m), read every 2 m.
fn furthest_from(line: &[Pt], cl: &[Pt]) -> f64 {
    let len = linear::length(line);
    (0..=(len / 2.0).floor() as usize)
        .map(|i| {
            let p = linear::interpolate(line, i as f64 * 2.0).unwrap();
            let q = linear::interpolate(cl, linear::project(cl, p)).unwrap();
            libm::hypot(p[0] - q[0], p[1] - q[1])
        })
        .fold(0.0, f64::max)
}

/// The limit off is the default, and the search then leaves the corridor on an open plain: the
/// shortest line runs up to about 300 m from the baseline of a 120 m corridor. With the limit on,
/// the same two pools give a line that never goes further than the corridor's half-width (to
/// within one grid cell); each pool's own line is inside too; every pool closes on its end pose,
/// R_min holds, every join passes and the line does not cross itself.
#[test]
fn with_the_limit_no_part_of_the_line_leaves_the_corridor_on_an_open_plain() {
    let cl = centreline(50.0, 1600.0, 4800.0);
    let total = linear::length(&cl);
    let pools = [(0.0, 2400.0, 0.0), (2400.0, total, 0.0)];
    let half = 120.0;
    assert!(!LatticeConfig::new(R_MIN).corridor_limit);

    let free = run(&cl, &pools, half, false);
    let far_free = furthest_from(&free.line, &cl);
    println!("limit off: up to {far_free:.0} m from the baseline");
    assert!(far_free > 2.0 * half, "{far_free}");

    let held = run(&cl, &pools, half, true);
    let far = furthest_from(&held.line, &cl);
    println!(
        "limit on: up to {far:.0} m from the baseline, {} m long against {:.0} m free",
        linear::length(&held.line).round(),
        linear::length(&free.line)
    );
    assert!(far <= half + CELL, "{far}");
    for p in &held.pools {
        assert!(p.used, "{:?}", p.kept_baseline_because);
        assert!(p.result.goal_closed);
        let own = furthest_from(p.result.line.as_ref().unwrap(), &cl);
        assert!(own <= half + CELL, "{own}");
    }
    assert!(held.radius_ok, "{}", held.measured_min_radius_m);
    assert_eq!(held.failed_joins().count(), 0, "{:?}", held.joins);
    assert_eq!(held.self_crossing_m, None);
    // The limit costs length: the held line is longer than the free one.
    assert!(linear::length(&held.line) > linear::length(&free.line));
}

/// A pool that cannot be searched inside the corridor says so and keeps its baseline: the line
/// never leaves the corridor silently. The baseline is a meander of 110 degrees (bends of 166 m,
/// far tighter than R_min 340 m) and the corridor is 100 m either side, so no line that holds R_min
/// fits inside it. With the limit off the same pool gets a line, far outside the corridor. With it
/// on, the pool is not used, the reason is recorded in plain words, the result lists it, and the
/// joined line is the baseline itself there.
#[test]
fn a_pool_with_no_line_inside_the_corridor_keeps_its_baseline_and_says_so() {
    let cl = centreline(110.0, 2000.0, 4000.0);
    let pools = [(500.0, 3000.0, 0.0)];
    let half = 100.0;

    let free = run(&cl, &pools, half, false);
    assert!(free.pools[0].result.converged);
    let far_free = furthest_from(free.pools[0].result.line.as_ref().unwrap(), &cl);
    println!("limit off: the pool's line goes up to {far_free:.0} m from the baseline");
    assert!(far_free > 1.5 * half, "{far_free}");

    let held = run(&cl, &pools, half, true);
    let p = &held.pools[0];
    println!(
        "limit on: converged {}, closed {}, used {}, {:?}, {} expansions",
        p.result.converged,
        p.result.goal_closed,
        p.used,
        p.kept_baseline_because,
        p.result.expansions
    );
    assert!(!p.used);
    assert!(!p.result.hit_cap);
    let why = p.kept_baseline_because.unwrap();
    assert!(
        why == NO_LINE_IN_CORRIDOR_REASON || why == UNCLOSED_IN_CORRIDOR_REASON,
        "{why}"
    );
    assert!(why.contains("inside the corridor"), "{why}");
    assert_eq!(
        held.no_line_in_corridor_pools().count() + held.unclosed_pools().count(),
        1
    );
    // The joined line is the baseline: nothing left the corridor.
    assert!(furthest_from(&held.line, &cl) < 1e-6);
    assert!((linear::length(&held.line) - linear::length(&cl)).abs() < 1e-6);
}
