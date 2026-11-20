//! The closing path of the pool lattice (issue #6, PR 556) holds R_min where it meets the lattice
//! line and along itself. The case is the Python golden test
//! `test_golden_search_goes_round_a_hill_and_holds_r_min`: a straight 600 m baseline over flat
//! ground at +1 m with a 12 m Gaussian hill (sigma 50 m) on it at x = 300 m; R_min 150 m.
//!
//! Label: property

use owd_core::alignment::limits::resample_line;
use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::alignment::route::min_radius_of_line;
use owd_core::geom::Polygon;
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pool,
};

fn hill(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| {
            1.0 + 12.0 * libm::exp(-((x - 300.0) * (x - 300.0) + y * y) / (2.0 * 50.0 * 50.0))
        })
        .collect()
}

/// No river anywhere.
fn far() -> Polygon {
    Polygon {
        exterior: vec![
            [1e6, 1e6],
            [1e6 + 1.0, 1e6],
            [1e6 + 1.0, 1e6 + 1.0],
            [1e6, 1e6],
        ],
        holes: vec![],
    }
}

/// The lattice line round the hill at `r_min` (the Python golden case's settings scaled by
/// `r_min / 150`): its radius on 25 m vertices and on the default check chord.
fn round_the_hill(r_min: f64) -> (f64, f64, bool) {
    let base = vec![[0.0, 0.0], [600.0, 0.0]];
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(8.0));
    spec.side_slope_h_per_v = 2.0;
    spec.rake_half_m = 60.0;
    spec.rake_step_m = 4.0;
    spec.ds_step_m = 8.0;
    let mut cfg = LatticeConfig::new(r_min);
    cfg.radius_multiples = vec![1.0, 2.0, 4.0];
    cfg.pitch_m = 10.0;
    cfg.n_heading = 24;
    let cfg = cfg.validated().unwrap();
    let pa = optimise_pool(
        &base,
        &hill,
        &[far()],
        0.0,
        600.0,
        3.0,
        3.0,
        &spec,
        &cfg,
        None,
        PoolOptions {
            corridor_half_m: 150.0,
            cell_m: 4.0,
        },
    )
    .unwrap();
    let r = &pa.result;
    assert!(r.converged, "R_min {r_min}: the search converges");
    let line = r.line.as_ref().unwrap();
    let on_vertices = min_radius_of_line(&resample_line(line, 25.0).unwrap()).0;
    let on_chord = min_radius_on_chord(line, default_check_chord(r_min)).unwrap();
    // The geometry itself, junctions included: every three consecutive vertices of the line lie
    // on a circle no tighter than R_min (vertices sit on the arcs, so this is exact).
    let (r3, at) = min_radius_of_line(line);
    assert!(
        r3 >= r_min * (1.0 - 1e-9),
        "R_min {r_min}: the vertices about {:?} turn on {r3} m",
        line[at]
    );
    (on_vertices, on_chord, r.goal_closed)
}

#[test]
fn the_golden_hill_case_holds_r_min_on_25_m_vertices() {
    let (v, c, closed) = round_the_hill(150.0);
    assert!(closed, "the goal is closed");
    assert!(
        v >= 0.99 * 150.0,
        "25 m vertices read {v:.3} m against R_min 150 m"
    );
    assert!(radius_ok(c, 150.0), "the check chord reads {c:.3} m");
}

/// The same case at other R_min: every one closes its goal and holds R_min on 25 m vertices and
/// on the check chord.
#[test]
fn the_hill_case_holds_r_min_at_every_r_min() {
    for r_min in [150.0, 255.0, 340.0, 360.0] {
        let (v, c, closed) = round_the_hill(r_min);
        println!("R_min {r_min}: 25 m vertices {v:.3} m, chord {c:.3} m, closed {closed}");
        assert!(closed, "R_min {r_min}: the goal is closed");
        assert!(
            v >= 0.99 * r_min,
            "R_min {r_min}: 25 m vertices read {v:.3} m"
        );
        assert!(
            radius_ok(c, r_min),
            "R_min {r_min}: the check chord reads {c:.3} m"
        );
    }
}
