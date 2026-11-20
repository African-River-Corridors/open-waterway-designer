//! A pool (or piece) whose result says its goal is closed must end on that goal, so every join of
//! the joined line has no gap. A length weight showed the fault; the weight is not its cause.
//!
//! The fault: a search state is a snapped position and heading. When a state is reached again more
//! cheaply, its pose is replaced, and states already reached from its old pose keep theirs. The line
//! is rebuilt by running the primitives forward from the start, so it can end some metres from the
//! pose the closing path was computed from. The closing path was appended as it was: the line ended
//! off the goal, in line with it (no heading gap), while the result said the goal was closed, and
//! the next piece then started tens of metres away, with a bend far tighter than R_min at the join.
//! At weight 0 on wide water every edge in the water costs nothing, so no state is reached again
//! more cheaply and the fault hides; with a cost per metre it shows. Now the closing path is
//! computed again from where the line really ends (or the goal is reported not closed).
//!
//! The case is synthetic: a closed-form sine river (amplitude 250 m, wavelength 1,600 m, 16 km of
//! x), water 240 m wide at -12 m in ground at +3 m; a channel half-width of 25 m; R_min 360 m,
//! pitch 36 m, cell 6 m, corridor 350 m, radius multiples 1, 1.5 and 3, 16 headings, dry weight 3.
//! Two pools, each cut into pieces (`max_pool_piece_m` 2,500 m), the baseline between them. Before
//! the fix, at weight 5 the second piece of the last pool said closed and ended 20.8 m off its goal.
//!
//! Label: property

use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, SectionSpec, optimise_pools,
};

const R_MIN: f64 = 360.0;
const AMP: f64 = 250.0;
const WAVE: f64 = 1600.0;
const LEN: f64 = 16_000.0;
const WATER_HALF: f64 = 120.0;

fn sine(off: f64) -> Vec<Pt> {
    let k = 2.0 * core::f64::consts::PI / WAVE;
    (0..=(LEN / 10.0) as usize)
        .map(|i| {
            let x = i as f64 * 10.0;
            let y = AMP * libm::sin(k * x);
            let d = [1.0, AMP * k * libm::cos(k * x)];
            let m = libm::hypot(d[0], d[1]);
            [x - d[1] / m * off, y + d[0] / m * off]
        })
        .collect()
}

fn dist_to_centre(p: Pt, centre: &[Pt]) -> f64 {
    let n = centre.len();
    let i = (p[0] / 10.0).round().clamp(0.0, (n - 1) as f64) as usize;
    let (a, b) = (i.saturating_sub(40), (i + 40).min(n - 1));
    linear::point_line_distance(p, &centre[a..=b])
}

fn run(w: f64) -> PoolsResult {
    let mut ring = sine(WATER_HALF);
    let mut right = sine(-WATER_HALF);
    right.reverse();
    ring.extend(right);
    ring.push(ring[0]);
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let centre = sine(0.0);
    let total = linear::length(&centre);
    let c2 = centre.clone();
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if dist_to_centre([x, y], &c2) <= WATER_HALF + 5.0 {
                    -12.0
                } else {
                    3.0
                }
            })
            .collect()
    };
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(25.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 100.0;
    spec.dry_weight = 3.0;
    spec.length_weight = w;
    let mut lat = LatticeConfig::new(R_MIN);
    lat.pitch_m = 36.0;
    lat.n_heading = 16;
    lat.radius_multiples = vec![1.0, 1.5, 3.0];
    lat.max_pool_piece_m = 2500.0;
    let lat = lat.validated().unwrap();
    optimise_pools(
        &centre,
        &terrain,
        core::slice::from_ref(&poly),
        &[(0.0, 0.2 * total, 0.0), (0.75 * total, total, 1.0)],
        3.25,
        &spec,
        &lat,
        None,
        PoolOptions {
            corridor_half_m: 350.0,
            cell_m: 6.0,
        },
        true,
    )
    .unwrap()
}

fn check(w: f64) {
    let r = run(w);
    for p in &r.pools {
        println!(
            "weight {w}: {:.0}..{:.0} used {} closed {} end {:.3} m off, cost {:.0}",
            p.start_m, p.end_m, p.used, p.result.goal_closed, p.result.end_err_m, p.result.cost
        );
    }
    assert_eq!(r.pools.len(), 4, "each pool is cut in two");
    for p in &r.pools {
        assert!(
            p.used,
            "the piece {:.0}..{:.0} is not used",
            p.start_m, p.end_m
        );
        assert!(
            p.result.goal_closed
                && p.result.end_err_m < 1e-6
                && p.result.end_heading_err_rad < 1e-9,
            "weight {w}: the piece {:.0}..{:.0} says closed {} and ends {} m off",
            p.start_m,
            p.end_m,
            p.result.goal_closed,
            p.result.end_err_m
        );
        assert!(p.result.radius_ok);
    }
    // The joins between two pieces of a pool hold R_min; every join has no gap.
    for j in &r.joins {
        println!(
            "weight {w}: join at {:.0}: gap {:.3} m, radius {:.0} m",
            j.at_m, j.position_gap_m, j.measured_min_radius_m
        );
        assert!(j.position_gap_m < 1e-6, "weight {w}: {j:?}");
        let inner = r
            .pools
            .iter()
            .any(|p| p.piece.is_some_and(|c| c.index > 0) && (p.start_m - j.at_m).abs() < 1e-9);
        if inner {
            assert!(j.radius_ok, "weight {w}: {j:?}");
        }
    }
    assert_eq!(r.self_crossing_m, None);
}

#[test]
fn every_closed_piece_ends_on_its_goal_at_weight_zero() {
    check(0.0);
}

#[test]
fn every_closed_piece_ends_on_its_goal_with_a_length_weight() {
    check(5.0);
}
