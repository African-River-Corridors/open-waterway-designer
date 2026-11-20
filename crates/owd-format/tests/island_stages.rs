//! An island in the river polygon is dry land to the re-route and to the optimise stage, as it is to
//! the volumes (spec 0017, ticket 3: one parse of the polygon, holes kept). The river is a straight
//! synthetic channel, 400 m wide and 2 km long, with a flat terrain above the water; an island sits
//! across its middle line. Each stage is run with and without the island and must read it.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#"{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![
        [X0 + x0, Y0 + y0],
        [X0 + x1, Y0 + y0],
        [X0 + x1, Y0 + y1],
        [X0 + x0, Y0 + y1],
        [X0 + x0, Y0 + y0],
    ]
}

fn island() -> Vec<Pt> {
    rect(700.0, 1300.0, -60.0, 60.0)
}

/// Ground at 9 m, above the 8 m water and the design bed: the same everywhere, so only the polygon
/// says where the island is. Cut is dry cost on the island and cheap wet cost in the water.
fn ground(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![9.0; xs.len()]
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![10.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    }
}

/// A sized, aligned (rolling circle) design with the water at 8 m and the given islands.
fn design(islands: &[Vec<Pt>]) -> Session {
    let centre: Vec<Pt> = (0..=40).map(|i| [X0 + 50.0 * f64::from(i), Y0]).collect();
    let mut s = Session::new("Island", "2026-11-20");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString","crs":{UTM30},"coordinates":{}}}"#,
            json!(centre)
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let mut rings = vec![rect(-100.0, 2100.0, -200.0, 200.0)];
    rings.extend(islands.iter().cloned());
    s.import(&[(
        Role::RiverPolygon,
        &format!(
            r#"{{"type":"Polygon","crs":{UTM30},"coordinates":{}}}"#,
            json!(rings)
        ),
    )])
    .unwrap();
    s.set_water_profile(&[[0.0, 8.0]]).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    s.set_dem("dem.tif", b"flat", &dem());
    s
}

fn line_of(o: &Value) -> Vec<Pt> {
    o["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn option(s: &Session, method: &str) -> Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap()
        .clone()
}

/// How many points of the line stand on the island rectangle.
fn on_island(line: &[Pt]) -> usize {
    line.iter()
        .filter(|p| (700.0..=1300.0).contains(&(p[0] - X0)) && (p[1] - Y0).abs() <= 60.0)
        .count()
}

#[test]
fn the_reroute_keeps_to_the_water_round_an_island() {
    let settings = json!({
        "corridor_sweep_m": [60.0, 120.0],
        "wet_weight": 0.01,
        "dry_weight": 5.0,
    });
    let mut open = design(&[]);
    open.align(AlignMethod::Reroute, &settings, Some(&ground))
        .unwrap();
    let mut isl = design(&[island()]);
    isl.align(AlignMethod::Reroute, &settings, Some(&ground))
        .unwrap();
    let (a, b) = (
        on_island(&line_of(&option(&open, "reroute"))),
        on_island(&line_of(&option(&isl, "reroute"))),
    );
    assert!(
        a > 0,
        "with no island in the polygon the line crosses that ground: {a}"
    );
    assert_eq!(
        b, 0,
        "with the island in the polygon the line stays off it: {b} points on it"
    );
}

#[test]
fn the_optimise_stage_prices_an_island_as_dry_land() {
    let settings = json!({
        "candidate_sites_m": [500.0, 1000.0, 1500.0],
        "lifts_m": [0.5, 1.0],
        "terminal_min_level_m": 8.5,
        "min_spacing_m": 1000.0,
        "dry_weight": 1.5,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "cut_model": "b",
        "maxw_m": 120.0,
        "rake_half_m": 80.0,
        "rake_step_m": 5.0,
        "ds_step_m": 10.0,
        "pitch_m": 20.0,
        "n_heading": 24,
        "radius_multiples": [1.0, 2.0, 4.0],
        "corridor_half_m": 180.0,
        "cell_m": 5.0,
    });
    let cost = |islands: &[Vec<Pt>]| -> f64 {
        let mut s = design(islands);
        s.run_optimise(&settings, Some(&ground)).unwrap();
        s.document()["stages"]["optimise"]["results"]["cascade"]["reach_cost"]
            .as_f64()
            .unwrap()
    };
    let (open, isl) = (cost(&[]), cost(&[island()]));
    assert!(
        isl > open,
        "the island is dry cut, which costs more than dredge: {isl} against {open}"
    );
}
