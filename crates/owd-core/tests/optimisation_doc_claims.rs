//! Statements of the optimisation document (`docs/optimisation/README.md`, sections 6 and 7) that
//! no other test pinned. The document names each of these tests in a hidden anchor, and
//! `owd-cli/tests/optimisation_doc.rs` fails when a named test is gone.
//!
//! Label: golden (closed forms by hand) and property.

use owd_core::cut_model::{CUT_MODEL, section_area};
use owd_core::optimise::lattice::{
    FieldScorer, HalfWidthSpec, LatticeConfig, Prim, SectionSpec, TerrainGrid, lattice_primitives,
};

const CELL_M: f64 = 2.0;
const SIDE: usize = 351; // 700 m square: the rake reaches 250 m either side of the line
// The rake's samples are 4 m apart and stand at -250, -246, ... so the ones nearest the line are at
// -2 m and 2 m: none is on the line. With a half-width of 16 m the eight bottom samples, at 2, 6, 10
// and 14 m either side, stand for exactly the 32 m of bottom.
const HALF_WIDTH_M: f64 = 16.0;
const DEPTH_M: f64 = 3.25;

/// A square grid with its middle at (0, 0); `ground(x, y)` gives the level and `wet` the river.
fn grid(ground: &dyn Fn(f64, f64) -> f64, wet: bool) -> TerrainGrid {
    let half = (SIDE as f64 - 1.0) / 2.0 * CELL_M;
    let rows = |f: &dyn Fn(f64, f64) -> f64| -> Vec<Vec<f64>> {
        (0..SIDE)
            .map(|r| {
                (0..SIDE)
                    .map(|c| f(-half + c as f64 * CELL_M, half - r as f64 * CELL_M))
                    .collect()
            })
            .collect()
    };
    TerrainGrid {
        ground: rows(ground),
        wet: vec![vec![wet; SIDE]; SIDE],
        valid: vec![vec![true; SIDE]; SIDE],
        corridor: vec![vec![true; SIDE]; SIDE],
        x0: -half,
        y0: half,
        cell_m: CELL_M,
    }
}

fn straight() -> Prim {
    let cfg = LatticeConfig::new(255.0).validated().unwrap();
    lattice_primitives(&cfg)
        .pop()
        .expect("the last move is the straight")
}

fn spec() -> SectionSpec {
    let mut s = SectionSpec::new(HalfWidthSpec::Fixed(HALF_WIDTH_M));
    s.side_slope_h_per_v = 3.0;
    s
}

/// The search cost of one metre of straight along x through (0, 0), with the bed at `-DEPTH_M`.
fn search_cost_per_m(g: &TerrainGrid, spec: &SectionSpec) -> (f64, bool) {
    let p = straight();
    let (c, ok) = FieldScorer::new(g, -DEPTH_M, HALF_WIDTH_M, spec).score(&p, (-12.0, 0.0, 0.0));
    (c / p.arc_len, ok)
}

/// Section 7, item 2: ground the search cannot see costs nothing, and a move is refused only when
/// under six tenths of its bottom samples are on known ground.
#[test]
fn unknown_ground_costs_no_cut_and_a_move_needs_six_tenths_of_its_bottom_known() {
    let plain = |_: f64, _: f64| 1.0; // 4.25 m above the bed, everywhere
    let (all, ok) = search_cost_per_m(&grid(&plain, false), &spec());
    assert!(ok && all > 0.0);

    // Unknown beyond the bottom (the side slopes, both sides): the move stands, and costs the
    // bottom only: 8 samples x 4 m x 4.25 m.
    let no_slopes = |_: f64, y: f64| {
        if y.abs() > HALF_WIDTH_M {
            f64::NAN
        } else {
            1.0
        }
    };
    let (bottom, ok) = search_cost_per_m(&grid(&no_slopes, false), &spec());
    assert!(ok);
    assert!((bottom - 8.0 * 4.0 * 4.25).abs() < 1e-6, "{bottom}");
    assert!(bottom < all);

    // The bottom samples are at 2, 6, 10 and 14 m either side. Unknown from 3 m on one side: 6, 10
    // and 14 are lost, 5 of 8 are known (0.625): the move stands and the lost samples cost nothing.
    let two_lost = |_: f64, y: f64| if y > 3.0 { f64::NAN } else { 1.0 };
    let (c, ok) = search_cost_per_m(&grid(&two_lost, false), &spec());
    assert!(ok);
    assert!(
        c < all,
        "unknown ground must not be charged: {c} against {all}"
    );

    // Unknown from -1 m: one whole side is lost, 4 of 8 are known (0.5 < 0.6): refused.
    let three_lost = |_: f64, y: f64| if y > -1.0 { f64::NAN } else { 1.0 };
    let (_, ok) = search_cost_per_m(&grid(&three_lost, false), &spec());
    assert!(
        !ok,
        "a move with under six tenths of its bottom known is refused"
    );

    // With the land cost on, unknown ground outside the river is land like any other.
    let mut land = spec();
    land.land_weight = 2.0;
    land.land_depth_m = DEPTH_M;
    let p = straight();
    let area = |g: &TerrainGrid| {
        FieldScorer::new(g, -DEPTH_M, HALF_WIDTH_M, &land).land_area(&p, (-12.0, 0.0, 0.0))
    };
    assert_eq!(area(&grid(&plain, false)), area(&grid(&two_lost, false)));
    assert!(area(&grid(&plain, false)) > 0.0);
}

/// Section 7, item 3: the search's section is not Cut Model A. On a dry plain 10 m above the pool
/// the search cuts one slope of 1 : 3 up to the ground; Cut Model A cuts 1 : 3 to the pool level,
/// a 2 m berm and 1 : 1.5 above. In the river the search cuts no side slope at all.
#[test]
fn the_search_section_is_not_cut_model_a() {
    let (wl, grade) = (0.0, -DEPTH_M);
    let offsets: Vec<f64> = (-200..=200).map(f64::from).collect();
    let sides: Vec<i8> = offsets
        .iter()
        .map(|&o| if o < 0.0 { -1 } else { 1 })
        .collect();
    let model_a = |z: f64, wet: bool| -> (f64, f64) {
        let (d, e, _) = section_area(
            &vec![z; offsets.len()],
            &offsets,
            &sides,
            HALF_WIDTH_M,
            grade,
            wl,
            DEPTH_M,
            CUT_MODEL,
            &vec![wet; offsets.len()],
            3.0,
            1.0,
            None,
        )
        .unwrap();
        (d, e)
    };
    let within = |got: f64, want: f64, share: f64| (got - want).abs() <= share * want;

    // A dry plain 10 m above the pool: the bed is h = 13.25 m below it.
    let h = 10.0 + DEPTH_M;
    let (search, ok) = search_cost_per_m(&grid(&|_, _| 10.0, false), &spec());
    assert!(ok);
    // By hand: bottom 32 h, and two slopes of 1 : 3 up to the ground, 3 h^2 together.
    let search_by_hand = 32.0 * h + 3.0 * h * h;
    assert!(
        within(search, search_by_hand, 0.03),
        "{search} against {search_by_hand}"
    );
    let (_, dry) = model_a(10.0, false);
    // By hand: bottom 32 h; under-water slopes 2 (9.75 h - 9.75^2 / 6); berms 2 x 2 x 10; upper
    // slopes 2 x (10 x 15 / 2).
    let a_by_hand = 32.0 * h + 2.0 * (9.75 * h - 9.75 * 9.75 / 6.0) + 40.0 + 150.0;
    assert!(within(dry, a_by_hand, 0.03), "{dry} against {a_by_hand}");
    assert!(
        search > 1.08 * dry,
        "the two sections differ: {search} against {dry}"
    );

    // In the river, 1 m below the pool level (2.25 m above the bed): the search cuts the bottom
    // only; Cut Model A cuts the two under-water slopes too.
    let (search, ok) = search_cost_per_m(&grid(&|_, _| -1.0, true), &spec());
    assert!(ok);
    assert!((search - 32.0 * 2.25).abs() < 1e-6, "{search}");
    let (dredge, _) = model_a(-1.0, true);
    let a_by_hand = 32.0 * 2.25 + 2.0 * (2.25 * 6.75 / 2.0);
    assert!(
        within(dredge, a_by_hand, 0.05),
        "{dredge} against {a_by_hand}"
    );
    assert!(dredge > 1.15 * search);
}

/// Section 7, item 9: the grain of the default lattice. Sixteen headings, so each arc turns
/// 22.5 degrees; three radii, none under R_min; one straight of twice the pitch.
#[test]
fn the_default_lattice_has_sixteen_headings_three_radii_and_a_straight_of_twice_the_pitch() {
    let r_min = 255.0;
    let cfg = LatticeConfig::new(r_min).validated().unwrap();
    assert_eq!(cfg.n_heading, 16);
    assert_eq!(cfg.radius_multiples, vec![1.0, 1.5, 3.0]);
    let prims = lattice_primitives(&cfg);
    assert_eq!(prims.len(), 7);
    let step = 2.0 * core::f64::consts::PI / 16.0;
    for (i, m) in [1.0, 1.5, 3.0].into_iter().enumerate() {
        for (p, sign) in [(&prims[2 * i], 1.0), (&prims[2 * i + 1], -1.0)] {
            assert_eq!(p.r, r_min * m);
            assert!(p.r >= r_min);
            assert!(
                (p.end.2 - sign * step).abs() < 1e-12,
                "an arc turns one heading step"
            );
            assert!((p.arc_len - p.r * step).abs() < 1e-9);
        }
    }
    let straight = &prims[6];
    assert_eq!(straight.sign, 0);
    assert_eq!(straight.arc_len, 2.0 * cfg.pitch_m);
    assert_eq!(straight.arc_len, 24.0);

    let mut tight = LatticeConfig::new(r_min);
    tight.radius_multiples = vec![0.9, 1.0];
    assert!(
        tight.validated().is_err(),
        "a radius under R_min is refused"
    );
}
