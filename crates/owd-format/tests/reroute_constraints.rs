//! The re-route steered by constraints (spec 0017, ticket 13), through the pure alignment stage.
//!
//! A straight synthetic river, 3 km long and 400 m wide, on flat ground at 10 m with the water at
//! 9 m. A no-go area is impassable: the re-route goes round one it would otherwise cross. A
//! multiplier patch makes cutting dearer: the re-route moves off it when that is cheaper. A cell at
//! or above the hard-exclusion multiplier is a no-go too. The methods that do not search (the rolling
//! circle, the centreline, a given line) do not steer: their line reports what it touches.
//!
//! Label: property

use owd_core::rules::engine;
use owd_format::design::constraints::{
    Area, Constraints, HARD_EXCLUSION, HARD_EXCLUSION_KIND, Multiplier, MultiplierRaster, NoGoArea,
    constraints_touched,
};
use owd_format::design::design_line::{
    AlignmentIn, DesignLine, alignment, alignment_with, check_constraints,
};
use owd_format::design::export::failed_checks;
use owd_format::design::river::{RiverIn, river};
use owd_format::design::sizing::{SizingIn, size};
use owd_format::design::{AlignMethod, AlignmentSettings};
use owd_format::types::{Polyline, StageError, TerrainSampler};
use serde_json::json;

fn flat(xs: &[f64], _: &[f64]) -> Vec<f64> {
    vec![10.0; xs.len()]
}

fn input(method: &str, constraints: Option<Constraints>) -> AlignmentIn {
    let centre: Vec<[f64; 2]> = (0..=60).map(|i| [50.0 * i as f64, 0.0]).collect();
    let river_in: RiverIn = serde_json::from_value(json!({
        "centreline": centre,
        "river_polygon": {"shell": [[-50.0, -200.0], [3050.0, -200.0], [3050.0, 200.0],
                                    [-50.0, 200.0], [-50.0, -200.0]]},
        "water_profile": [[0.0, 9.0]]
    }))
    .unwrap();
    let mut sizing: SizingIn = serde_json::from_value(json!({
        "rule_sets": [{"id": "pianc-wg141", "version": "0.3.0"}, {"id": "owd", "version": "0.3.0"}],
        "set_inputs": [{"name": "se_quality", "value": "C"}],
        "vessel_standard_id": "CEMT_IV"
    }))
    .unwrap();
    for set in &mut sizing.rule_sets {
        set.hash = engine::built_in()
            .find(&set.id, &set.version)
            .map(|s| s.hash.clone());
    }
    let settings: AlignmentSettings = serde_json::from_value(json!({
        "method": method,
        "reroute": {"corridor_sweep_m": [40.0, 80.0, 120.0, 160.0],
                    "station_step_m": 25.0, "offset_step_m": 5.0}
    }))
    .unwrap();
    AlignmentIn {
        river: river(&river_in).unwrap(),
        size: size(&sizing, engine::built_in()).unwrap(),
        settings,
        seed: None,
        constraints,
        volumes: None,
    }
}

fn box_area(name: &str, x0: f64, x1: f64, y0: f64, y1: f64) -> NoGoArea {
    NoGoArea {
        name: name.into(),
        kind: "settlement".into(),
        polygon: Area {
            shell: Polyline(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]),
            holes: vec![],
        },
    }
}

fn no_go(areas: Vec<NoGoArea>) -> Constraints {
    Constraints {
        no_go: areas,
        cut_rate_multiplier: None,
    }
}

fn multiplier() -> Constraints {
    Constraints {
        no_go: vec![],
        cut_rate_multiplier: Some(MultiplierRaster {
            path: "weights.tif".into(),
            sha256: "0".repeat(64),
            crs: "EPSG:32630".into(),
            nodata: None,
        }),
    }
}

/// A multiplier given as a function of the point, read on cells `cell_m` across.
fn per_cell(at: TerrainSampler, cell_m: f64) -> Multiplier {
    Multiplier { at, cell_m }
}

fn reroute(
    c: Option<Constraints>,
    weight: Option<TerrainSampler>,
) -> Result<DesignLine, StageError> {
    alignment_with(
        &input("reroute", c),
        Some(&flat),
        weight.map(|w| per_cell(w, 5.0)),
    )
}

/// The line's offset from the river centreline (y) where it crosses `x`.
fn y_at(line: &DesignLine, x: f64) -> f64 {
    let p = &line.line.0;
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a[0] - x) * (b[0] - x) <= 0.0 && a[0] != b[0] {
            return a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]);
        }
    }
    panic!("the line does not cross x = {x}");
}

fn half_width(i: &AlignmentIn) -> f64 {
    i.size.bottom_width_straight_m / 2.0
}

#[test]
fn with_no_constraints_the_stage_gives_the_line_it_gave_before() {
    let a = alignment(&input("reroute", None), Some(&flat)).unwrap();
    let b = reroute(None, None).unwrap();
    assert_eq!(a, b);
    assert!(a.constraints_touched.is_empty());
}

#[test]
fn the_re_route_goes_round_a_no_go_area_it_would_otherwise_cross() {
    let pond = box_area("pond", 1300.0, 1700.0, -60.0, 60.0);
    let free = reroute(None, None).unwrap();
    let i = input("reroute", None);
    let crossed = constraints_touched(&free.line.0, half_width(&i), std::slice::from_ref(&pond));
    assert!(!crossed.is_empty(), "the free line crosses the area");

    let line = reroute(Some(no_go(vec![pond])), None).unwrap();
    assert!(
        line.constraints_touched.is_empty(),
        "{:?}",
        line.constraints_touched
    );
    let y = y_at(&line, 1500.0);
    assert!(
        y.abs() >= 60.0 + half_width(&i),
        "round, not through: y = {y}"
    );
    assert_eq!(line.checks["radius_ok"], true, "and it still meets R_min");
}

#[test]
fn an_area_no_line_can_go_round_is_refused_by_name() {
    // From bank to bank: no offset clears it.
    let wall = box_area("wall", 1400.0, 1600.0, -250.0, 250.0);
    let e = reroute(Some(no_go(vec![wall])), None).unwrap_err();
    // The grid has no passable cell at the wall's stations, so the search finds no route.
    assert_eq!(e.code, "ALIGN_NO_ROUTE", "{e}");
}

/// A line straight down the middle of the river, given by the user.
fn given_input(constraints: Option<Constraints>) -> AlignmentIn {
    let mut i = input("given", constraints);
    i.settings.given = Some(
        serde_json::from_value(json!({
            "name": "straight",
            "line": {"type": "LineString", "coordinates": [[0.0, 0.0], [3000.0, 0.0]]}
        }))
        .unwrap(),
    );
    i
}

#[test]
fn a_re_route_whose_final_line_still_enters_a_no_go_area_is_refused_with_its_own_code() {
    // The grid keeps a margin round a no-go area, so the search itself does not end in one: the
    // stage's last step is what refuses such a line. It is tested on a line through the area.
    let pond = box_area("pond", 1300.0, 1700.0, -60.0, 60.0);
    let i = given_input(Some(no_go(vec![pond])));
    let mut line = alignment_with(&i, None, None).unwrap();
    let mut as_reroute = i.clone();
    as_reroute.settings.method = AlignMethod::Reroute;
    let e = check_constraints(&mut line, &as_reroute, None).unwrap_err();
    assert_eq!(e.code, "ALIGN_NO_GO_CROSSED", "{e}");
    assert!(e.message.contains("pond"), "{e}");
    let at = e.at.expect("the refusal has a chainage").0;
    assert!((at - 1300.0).abs() <= 5.0, "{at}");
}

/// ×1e6 (a hard exclusion) on a strip 4 m wide across the whole river, x 1310..1314: narrower than
/// the 25 m station step, between two stations.
fn narrow_strip(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .map(|&x| {
            if (1310.0..1314.0).contains(&x) {
                HARD_EXCLUSION
            } else {
                f64::NAN
            }
        })
        .collect()
}

#[test]
fn a_hard_exclusion_strip_narrower_than_the_station_step_is_not_crossed_in_silence() {
    // Every line from one end of the river to the other crosses the strip, and the grid's stations
    // (25 m apart, at 1300 m and 1325 m) do not see it. The final line is walked over the raster
    // at its cell size, so the re-route refuses by name.
    let e = alignment_with(
        &input("reroute", Some(multiplier())),
        Some(&flat),
        Some(per_cell(&narrow_strip, 1.0)),
    )
    .unwrap_err();
    assert_eq!(e.code, "ALIGN_HARD_EXCLUSION_CROSSED", "{e}");
    let at = e.at.expect("the refusal has a chainage").0;
    assert!((1300.0..1325.0).contains(&at), "{at}");
}

#[test]
fn every_method_lists_the_hard_exclusion_ground_its_footprint_enters() {
    for method in ["centreline", "given", "rolling_circle"] {
        let mut i = given_input(Some(multiplier()));
        i.settings.method = serde_json::from_value(json!(method)).unwrap();
        let line = alignment_with(&i, None, Some(per_cell(&narrow_strip, 1.0))).unwrap();
        let hard: Vec<_> = line
            .constraints_touched
            .iter()
            .filter(|t| t.kind == HARD_EXCLUSION_KIND)
            .collect();
        assert_eq!(hard.len(), 1, "{method}: {:?}", line.constraints_touched);
        let t = hard[0];
        assert_eq!(t.name, "weights.tif", "named by the raster");
        assert!(
            t.from.0 >= 1305.0 && t.to.0 <= 1320.0 && t.from.0 < t.to.0,
            "{t:?}"
        );
        // The strip is 4 m wide across the whole footprint: 4 m × the bottom width, by hand.
        let want = 4.0 * 2.0 * half_width(&i);
        assert!(
            (t.area_m2 - want).abs() <= 0.25 * want,
            "{method}: {} vs {want}",
            t.area_m2
        );
        assert_eq!(t.footprint_width_m, 2.0 * half_width(&i));
        assert_eq!(line.checks["no_go_clear"]["ok"], false, "{method}");
        assert!(failed_checks(&line.to_view()).contains(&"no_go_clear"));
    }
}

#[test]
fn a_named_multiplier_with_no_raster_to_read_is_refused_by_every_method() {
    for method in ["centreline", "given", "rolling_circle"] {
        let mut i = given_input(Some(multiplier()));
        i.settings.method = serde_json::from_value(json!(method)).unwrap();
        let e = alignment_with(&i, None, None).unwrap_err();
        assert_eq!(e.code, "ALIGN_NO_MULTIPLIER", "{method}: {e}");
    }
}

#[test]
fn a_given_line_through_a_no_go_area_is_saved_and_fails_no_go_clear() {
    let pond = box_area("pond", 1300.0, 1700.0, -60.0, 60.0);
    let i = given_input(Some(no_go(vec![pond])));
    let line = alignment_with(&i, None, None).unwrap();
    assert_eq!(line.constraints_touched.len(), 1);
    let c = &line.checks["no_go_clear"];
    assert_eq!(c["ok"], false);
    assert_eq!(c["footprint_width_m"], json!(2.0 * half_width(&i)));
    assert!(c["note"].as_str().unwrap().contains("bottom width"));
    assert_eq!(failed_checks(&line.to_view()), vec!["no_go_clear"]);
}

#[test]
fn a_clean_given_line_passes_no_go_clear() {
    let far = box_area("far", 1300.0, 1700.0, 120.0, 180.0);
    let i = given_input(Some(no_go(vec![far])));
    let line = alignment_with(&i, None, None).unwrap();
    assert!(line.constraints_touched.is_empty());
    assert_eq!(line.checks["no_go_clear"]["ok"], true);
    assert!(failed_checks(&line.to_view()).is_empty());
}

#[test]
fn with_no_constraints_a_line_has_no_no_go_clear_check() {
    // The check is not written at all without constraints, so a stored line does not change.
    for method in ["centreline", "given", "rolling_circle"] {
        let mut i = given_input(None);
        i.settings.method = serde_json::from_value(json!(method)).unwrap();
        let line = alignment_with(&i, None, None).unwrap();
        assert!(line.checks.get("no_go_clear").is_none(), "{method}");
    }
    let line = reroute(None, None).unwrap();
    assert!(line.checks.get("no_go_clear").is_none());
}

#[test]
fn a_multiplier_patch_moves_the_line_off_dear_ground() {
    // ×5 on a band 80 m wide along the middle of the river, from 1000 m to 2000 m.
    let band = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if (1000.0..=2000.0).contains(&x) && y.abs() <= 40.0 {
                    5.0
                } else {
                    f64::NAN
                }
            })
            .collect()
    };
    let free = reroute(None, None).unwrap();
    let moved = reroute(Some(multiplier()), Some(&band)).unwrap();
    let on_band = |l: &DesignLine| {
        (1200..=1800)
            .step_by(50)
            .filter(|&x| y_at(l, x as f64).abs() <= 40.0)
            .count()
    };
    assert!(on_band(&free) > 0, "the free line runs on the band");
    assert_eq!(on_band(&moved), 0, "the weighted line runs off it");
    assert_eq!(moved.checks["radius_ok"], true);
}

#[test]
fn a_hard_exclusion_in_the_multiplier_is_impassable() {
    let wall = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if (1300.0..=1700.0).contains(&x) && y.abs() <= 60.0 {
                    HARD_EXCLUSION
                } else {
                    1.0
                }
            })
            .collect()
    };
    let i = input("reroute", None);
    let line = reroute(Some(multiplier()), Some(&wall)).unwrap();
    assert!(
        line.constraints_touched.is_empty(),
        "{:?}",
        line.constraints_touched
    );
    let y = y_at(&line, 1500.0);
    assert!(
        y.abs() >= 60.0 + half_width(&i),
        "round, not through: y = {y}"
    );
}

#[test]
fn a_named_multiplier_with_no_raster_to_read_is_refused() {
    let e = reroute(Some(multiplier()), None).unwrap_err();
    assert_eq!(e.code, "ALIGN_NO_MULTIPLIER");
}

#[test]
fn the_centreline_does_not_steer_and_reports_the_area_it_enters() {
    let pond = box_area("pond", 1300.0, 1700.0, -60.0, 60.0);
    let i = input("centreline", Some(no_go(vec![pond])));
    let line = alignment_with(&i, None, None).unwrap();
    assert_eq!(line.method, AlignMethod::Centreline.name());
    assert_eq!(line.constraints_touched.len(), 1);
    let t = &line.constraints_touched[0];
    assert_eq!((t.name.as_str(), t.kind.as_str()), ("pond", "settlement"));
    assert!((t.from.0 - 1300.0).abs() <= 5.0 && (t.to.0 - 1700.0).abs() <= 5.0);
    let want = 400.0 * 2.0 * half_width(&i);
    assert!(
        (t.area_m2 - want).abs() <= 0.02 * want,
        "{} vs {want}",
        t.area_m2
    );
}
