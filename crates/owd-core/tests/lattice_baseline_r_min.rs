//! The pool lattice and a baseline that breaks R_min (issue #547). The lattice keeps a pool's
//! baseline segment when its own line costs more. On a meandering river the baseline is tighter
//! than R_min and follows the water, so it is cheap; keeping it breaks R_min though the lattice
//! line holds it. The rule: a baseline segment that breaks R_min is never kept in place of a
//! converged lattice line that holds it; `n_replaced` counts the pools whose lattice line is in
//! the joined line.
//!
//! The river is a closed-form sine: amplitude 250 m, wavelength 1,600 m (centreline radius about
//! 260 m) against R_min 340 m; a river 80 m wide at -12 m in ground at +3 m; a channel half-width of 25 m.
//!
//! Label: property

use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};

const R_MIN: f64 = 340.0;

fn sine(len: f64, off: f64) -> Vec<Pt> {
    sine_of(250.0, 1600.0, len, off)
}

fn sine_of(amp: f64, wl: f64, len: f64, off: f64) -> Vec<Pt> {
    (0..=(len / 10.0) as usize)
        .map(|i| {
            let x = i as f64 * 10.0;
            let k = 2.0 * core::f64::consts::PI / wl;
            let y = amp * libm::sin(k * x);
            let d = [1.0, amp * k * libm::cos(k * x)];
            let m = libm::hypot(d[0], d[1]);
            [x - d[1] / m * off, y + d[0] / m * off]
        })
        .collect()
}

fn run(len: f64, amp: f64, wl: f64, baseline: &[Pt]) -> owd_core::optimise::lattice::PoolsResult {
    let mut ring = sine_of(amp, wl, len, 40.0);
    let mut right = sine_of(amp, wl, len, -40.0);
    right.reverse();
    ring.extend(right);
    ring.push(ring[0]);
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let centre = sine_of(amp, wl, len, 0.0);
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if linear::point_line_distance([x, y], &centre) <= 45.0 {
                    -12.0
                } else {
                    3.0
                }
            })
            .collect()
    };
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(25.0));
    spec.side_slope_h_per_v = 3.0;
    let lat = LatticeConfig::new(R_MIN).validated().unwrap();
    optimise_pools(
        baseline,
        &terrain,
        core::slice::from_ref(&poly),
        &[(0.0, linear::length(baseline), 0.0)],
        3.25,
        &spec,
        &lat,
        None,
        PoolOptions {
            corridor_half_m: 300.0,
            cell_m: 5.0,
        },
        true,
    )
    .unwrap()
}

#[test]
fn a_baseline_under_r_min_is_not_kept_in_place_of_a_lattice_line_that_holds_it() {
    let baseline = sine(3200.0, 0.0);
    let chord = owd_core::alignment::radius_check::default_check_chord(R_MIN);
    let base_r = min_radius_on_chord(&baseline, chord).unwrap();
    assert!(
        !radius_ok(base_r, R_MIN),
        "the baseline must break R_min for this case ({base_r})"
    );
    let r = run(3200.0, 250.0, 1600.0, &baseline);
    let pool = &r.pools[0];
    assert!(
        pool.result.converged && pool.result.radius_ok,
        "the lattice line holds R_min"
    );
    assert!(
        r.radius_ok,
        "the joined line breaks R_min: {} m against {R_MIN} m",
        r.measured_min_radius_m
    );
    assert!(pool.used, "the pool's lattice line is in the option");
    assert_eq!(r.n_replaced(), 1);
}

#[test]
fn a_cheaper_baseline_that_holds_r_min_is_kept_and_says_why() {
    // A gentle river: the baseline holds R_min.
    let baseline = sine_of(60.0, 3000.0, 3000.0, 0.0);
    let chord = owd_core::alignment::radius_check::default_check_chord(R_MIN);
    assert!(radius_ok(
        min_radius_on_chord(&baseline, chord).unwrap(),
        R_MIN
    ));
    let r = run(3000.0, 60.0, 3000.0, &baseline);
    let pool = &r.pools[0];
    assert!(r.radius_ok);
    assert!(
        pool.result.cost > pool.baseline_cost,
        "the case needs a dearer lattice line"
    );
    assert!(!pool.used && pool.kept_baseline_because.is_some());
    assert_eq!(r.n_replaced(), 0);
}
