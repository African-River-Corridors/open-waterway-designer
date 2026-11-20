//! The cut-rate multiplier and no-go areas in the core (spec 0017, ticket 13).
//!
//! The section walk weighs each sample's cut by the multiplier at the sample; the plain areas do
//! not move. The re-route's cost grid weighs each transect point's cut the same way, and marks a
//! cell impassable when the channel footprint at that offset reaches a no-go area or a multiplier
//! at or above the hard-exclusion value.
//!
//! Every case is closed form, as in `volume_by_kind.rs`: flat terrain at 10 m, the water at 10 m,
//! depth 2 m, bottom half-width 20 m, wet slope 3, samples 1 m apart: 92 m² a section, all dry cut
//! (no river). The line is 1000 m long with a station every 25 m: 41 sections.
//!
//! Label: golden (the hand-worked volumes), property (the rest)

use owd_core::alignment::route::{
    GridConstraints, GridParams, HARD_EXCLUSION, build_cost_grid, build_cost_grid_constrained,
};
use owd_core::geom::{Polygon, Pt};
use owd_core::types::TerrainSampler;
use owd_core::volumes::sections::{
    DEFAULT_CHUNK, HalfWidth, Level, SectionVolumes, section_volumes, section_volumes_weighted,
};

const SECTION_M2: f64 = 92.0;
const STATION_M: f64 = 25.0;

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![10.0; xs.len()]
}

fn walk(weight: Option<TerrainSampler>) -> SectionVolumes {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    section_volumes_weighted(
        &line,
        &flat,
        &[],
        &Level::Fixed(10.0),
        2.0,
        &HalfWidth::Fixed(20.0),
        "a",
        3.0,
        STATION_M,
        100.0,
        1.0,
        None,
        None,
        DEFAULT_CHUNK,
        weight,
    )
    .unwrap()
}

fn plain() -> SectionVolumes {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    section_volumes(
        &line,
        &flat,
        &[],
        &Level::Fixed(10.0),
        2.0,
        &HalfWidth::Fixed(20.0),
        "a",
        3.0,
        STATION_M,
        100.0,
        1.0,
        None,
        None,
        DEFAULT_CHUNK,
    )
    .unwrap()
}

fn close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-9 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

#[test]
fn with_no_multiplier_the_walk_is_the_plain_walk_and_has_no_weighted_cut() {
    let (a, b) = (walk(None), plain());
    assert_eq!(a.stations.len(), b.stations.len());
    for (x, y) in a.stations.iter().zip(&b.stations) {
        assert_eq!(x.dredge_area_m2, y.dredge_area_m2);
        assert_eq!(x.excav_area_m2, y.excav_area_m2);
        assert_eq!(x.weighted_dredge_area_m2, None);
        assert_eq!(x.weighted_excav_area_m2, None);
        assert_eq!(y.weighted_dredge_area_m2, None);
    }
}

#[test]
fn a_multiplier_of_one_weighs_exactly_the_plain_cut_and_moves_no_plain_number() {
    let one = |xs: &[f64], _: &[f64]| vec![1.0; xs.len()];
    let (a, b) = (walk(Some(&one)), plain());
    for (x, y) in a.stations.iter().zip(&b.stations) {
        assert_eq!(
            x.dredge_area_m2, y.dredge_area_m2,
            "plain numbers do not move"
        );
        assert_eq!(
            x.excav_area_m2, y.excav_area_m2,
            "plain numbers do not move"
        );
        assert_eq!(x.weighted_dredge_area_m2, Some(x.dredge_area_m2));
        assert_eq!(x.weighted_excav_area_m2, Some(x.excav_area_m2));
    }
}

#[test]
fn a_patch_of_two_over_the_far_half_doubles_the_cut_there_by_hand() {
    // ×2 where x >= 500 m: the stations at 500, 525, ..., 1000 m (21 of 41). No data (NaN) is 1.
    let patch = |xs: &[f64], _: &[f64]| -> Vec<f64> {
        xs.iter()
            .map(|&x| if x >= 500.0 { 2.0 } else { f64::NAN })
            .collect()
    };
    let v = walk(Some(&patch));
    assert_eq!(v.stations.len(), 41);
    for s in &v.stations {
        close(s.excav_area_m2, SECTION_M2, "plain dry cut");
        let want = if s.chainage_m >= 500.0 { 2.0 } else { 1.0 } * SECTION_M2;
        close(s.weighted_excav_area_m2.unwrap(), want, "weighted dry cut");
        assert_eq!(s.weighted_dredge_area_m2, Some(0.0), "no river, no dredge");
    }
    let weighted: f64 = v
        .stations
        .iter()
        .map(|s| s.weighted_excav_area_m2.unwrap())
        .sum::<f64>()
        * STATION_M;
    // (20 stations at 1, 21 at 2) × 92 m² × 25 m.
    close(
        weighted,
        (20.0 + 2.0 * 21.0) * SECTION_M2 * STATION_M,
        "weighted m³",
    );
    close(
        v.excav_m3(),
        41.0 * SECTION_M2 * STATION_M,
        "plain m³ unchanged",
    );
}

fn params() -> GridParams {
    GridParams {
        bottom_width: 40.0,
        depth: 2.0,
        side_slope: 3.0,
        corridor_half: 60.0,
        station_step: 50.0,
        offset_step: 10.0,
        transect_step: 2.0,
        daylight_m: None,
    }
}

fn box_(x0: f64, x1: f64, y0: f64, y1: f64) -> Polygon {
    Polygon {
        exterior: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]],
        holes: vec![],
    }
}

#[test]
fn the_cost_grid_weighs_each_cell_by_the_multiplier_and_is_plain_without_one() {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    let grade = |_: f64| 8.0;
    let base = build_cost_grid(&line, &flat, &[], &grade, &params()).unwrap();
    let none = build_cost_grid_constrained(
        &line,
        &flat,
        &[],
        &grade,
        &params(),
        &GridConstraints::default(),
    )
    .unwrap();
    assert_eq!(none, base, "no constraints: the grid of today, bit for bit");
    assert!(base.no_go.is_empty());
    let three = |xs: &[f64], _: &[f64]| vec![3.0; xs.len()];
    let g = build_cost_grid_constrained(
        &line,
        &flat,
        &[],
        &grade,
        &params(),
        &GridConstraints {
            weight: Some(&three),
            ..Default::default()
        },
    )
    .unwrap();
    for (rb, rg) in base.vol.iter().zip(&g.vol) {
        for (b, x) in rb.iter().zip(rg) {
            close(*x, 3.0 * b, "a ×3 multiplier triples each cell's cut");
        }
    }
}

#[test]
fn a_no_go_area_blocks_the_cells_whose_footprint_reaches_it() {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    let grade = |_: f64| 8.0;
    // The box: x 400 to 600 m, y 30 to 80 m. Footprint half-width 20 m.
    let no_go = [box_(400.0, 600.0, 30.0, 80.0)];
    let g = build_cost_grid_constrained(
        &line,
        &flat,
        &[],
        &grade,
        &params(),
        &GridConstraints {
            no_go: &no_go,
            footprint_half_m: 20.0,
            ..Default::default()
        },
    )
    .unwrap();
    let k = |o: f64| g.offsets.iter().position(|x| (x - o).abs() < 1e-9).unwrap();
    let i = |s: f64| {
        g.stations_m
            .iter()
            .position(|x| (x - s).abs() < 1e-9)
            .unwrap()
    };
    // At 500 m: offset 40 (footprint 20 to 60) is in the box; offset -40 is far from it.
    assert!(g.blocked(i(500.0), k(40.0)));
    assert!(!g.blocked(i(500.0), k(-40.0)));
    // Far along the line from the box nothing is blocked.
    assert!(!g.blocked(i(0.0), k(40.0)));
    assert!(!g.blocked(i(1000.0), k(40.0)));
}

#[test]
fn a_multiplier_at_the_hard_exclusion_value_is_impassable() {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    let grade = |_: f64| 8.0;
    let wall = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if (450.0..=550.0).contains(&x) && y < -30.0 {
                    HARD_EXCLUSION
                } else {
                    1.0
                }
            })
            .collect()
    };
    let g = build_cost_grid_constrained(
        &line,
        &flat,
        &[],
        &grade,
        &params(),
        &GridConstraints {
            weight: Some(&wall),
            footprint_half_m: 20.0,
            ..Default::default()
        },
    )
    .unwrap();
    let k = |o: f64| g.offsets.iter().position(|x| (x - o).abs() < 1e-9).unwrap();
    let i = |s: f64| {
        g.stations_m
            .iter()
            .position(|x| (x - s).abs() < 1e-9)
            .unwrap()
    };
    assert!(g.blocked(i(500.0), k(-40.0)));
    assert!(!g.blocked(i(500.0), k(40.0)));
}
