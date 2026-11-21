//! Adopt only if better (OPT-3, part 4). A pool keeps its baseline when the baseline costs less than
//! the lattice's line (`PoolAlignment::baseline_cost` against `LatticeResult::cost`,
//! `kept_baseline_because`). The two must be measured the same way, with the length cost and the
//! land cost in both.
//!
//! The case leaves the search nothing to find: flat dry ground at +3 m, a straight baseline, no
//! river. The cheapest line is the baseline itself, so the two costs are the same channel measured
//! twice: once as the lattice's straights, once as the baseline's segment.
//!
//! The corridor is wide (700 m either side) on purpose. Ground off the search grid reads as unknown
//! and costs nothing to cut, so in a narrow corridor the search finds a line along the grid's edge
//! that is longer and still "cheaper" than the baseline (measured here with a 150 m corridor:
//! 1,504 m at 474,732 against the baseline's 1,440 m at 530,400). That is a fault of the search
//! cost at the edge of its grid, reported under OPT-3 part 4 and not changed here.
//!
//! Label: property

use owd_core::geom::{Polygon, Pt};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolAlignment, PoolOptions, SectionSpec, optimise_pool,
};

const LEN: f64 = 1440.0;

fn pool(length_weight: f64, land_weight: f64) -> PoolAlignment {
    let line: Vec<Pt> = vec![[0.0, 0.0], [LEN, 0.0]];
    // No river anywhere near: every cut is dry and every m2 of footprint is land.
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
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 80.0;
    spec.length_weight = length_weight;
    spec.land_weight = land_weight;
    spec.land_depth_m = 3.25;
    let mut cfg = LatticeConfig::new(340.0);
    cfg.pitch_m = 24.0;
    optimise_pool(
        &line,
        &flat,
        core::slice::from_ref(&far),
        0.0,
        LEN,
        0.0,
        3.25,
        &spec,
        &cfg.validated().unwrap(),
        None,
        PoolOptions {
            corridor_half_m: 700.0,
            cell_m: 8.0,
        },
    )
    .unwrap()
}

fn length_of(p: &PoolAlignment) -> f64 {
    p.result.prim_seq.iter().map(|q| q.2).sum()
}

/// The lattice's line and the baseline are the same straight, and cost the same to within 1 %:
/// with no weights, with the length cost, with the land cost, and with both. Each weight adds the
/// same amount to both sides: the weight times the same length, and the weight times the same area.
#[test]
fn the_line_and_the_baseline_are_costed_the_same_way_with_length_and_land_in_both() {
    let plain = pool(0.0, 0.0);
    assert!(plain.result.converged && plain.result.goal_closed);
    println!(
        "line length {}, cost {}, baseline {}",
        length_of(&plain),
        plain.result.cost,
        plain.baseline_cost
    );
    assert!(
        (length_of(&plain) - LEN).abs() < 1.0,
        "the line is the straight"
    );
    let close = |a: f64, b: f64| (a - b).abs() <= 0.01 * b.abs();
    println!(
        "no weights: line {:.0}, baseline {:.0}",
        plain.result.cost, plain.baseline_cost
    );
    assert!(close(plain.result.cost, plain.baseline_cost));

    for (lw, aw) in [(5.0, 0.0), (0.0, 2.0), (5.0, 2.0)] {
        let p = pool(lw, aw);
        assert!((length_of(&p) - LEN).abs() < 1.0);
        let (line_area, base_area) = (
            p.result.land_area_m2.unwrap_or(0.0),
            p.baseline_land_area_m2.unwrap_or(0.0),
        );
        println!(
            "length weight {lw}, land weight {aw}: line {:.0}, baseline {:.0}; land {line_area:.0} m2 against {base_area:.0} m2",
            p.result.cost, p.baseline_cost
        );
        assert!(close(p.result.cost, p.baseline_cost));
        // What the weights add, on each side.
        let added_line = p.result.cost - plain.result.cost;
        let added_base = p.baseline_cost - plain.baseline_cost;
        assert!(close(added_line, lw * LEN + aw * line_area), "{added_line}");
        assert!(close(added_base, lw * LEN + aw * base_area), "{added_base}");
        if aw > 0.0 {
            assert!(close(line_area, base_area));
            // The footprint: 40 m of bed and 1:3 sides through 3.25 m, read on a 4 m rake.
            assert!(close(line_area, LEN * 60.0), "{line_area}");
        }
    }
}
