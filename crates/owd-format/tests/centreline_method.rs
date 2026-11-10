//! Ticket C29: the alignment method `centreline`. The design line is the river centreline, as
//! given or as the stage builds it from the two banks. Nothing repairs its radius, and its option
//! says so: `radius_ok` is false where the centreline is tighter than the minimum radius.
//!
//! Label: property

use owd_core::alignment::centreline::build_centreline;
use owd_core::geom::Pt;
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};
use std::f64::consts::FRAC_PI_2;
use std::path::Path;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn method() -> AlignMethod {
    AlignMethod::from_name("centreline").expect("`centreline` is not an alignment method")
}

/// A sized design (PIANC WG 141 quality B, CEMT IV) with `line` as its centreline.
fn design(line: &[Pt]) -> Session {
    let mut s = Session::new("Centreline method", "2026-11-10");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(line)
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

fn r_min(s: &Session) -> f64 {
    s.document()["stages"]["sizing"]["results"]["min_bend_radius_m"]
        .as_f64()
        .unwrap()
}

fn points(line: &Value) -> Vec<Pt> {
    line["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn option<'a>(s: &'a Session, method: &str) -> &'a Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap_or_else(|| panic!("no option of the method `{method}`"))
}

/// 600 m straight, a quarter circle of radius `r` to the left, 600 m straight; a point about
/// every 5 m.
fn line_with_bend(r: f64) -> Vec<Pt> {
    let (x0, y0) = (500_000.0, 600_000.0);
    let mut pts: Vec<Pt> = (0..120).map(|i| [x0 + 5.0 * f64::from(i), y0]).collect();
    let n = (r * FRAC_PI_2 / 5.0).ceil();
    for i in 0..=(n as u32) {
        let t = FRAC_PI_2 * f64::from(i) / n;
        pts.push([x0 + 600.0 + r * libm::sin(t), y0 + r - r * libm::cos(t)]);
    }
    for i in 1..=120 {
        pts.push([x0 + 600.0 + r, y0 + r + 5.0 * f64::from(i)]);
    }
    pts
}

#[test]
fn the_line_is_the_centreline_and_a_tight_bend_is_reported_not_repaired() {
    let straight: Vec<Pt> = vec![[500_000.0, 600_000.0], [501_500.0, 600_000.0]];
    let r_min = r_min(&design(&straight));
    let bend_r = r_min / 2.0;
    let mut s = design(&line_with_bend(bend_r));
    s.align(method(), &Value::Null, None).unwrap();

    let o = option(&s, "centreline");
    assert_eq!(o["id"], "centreline");
    assert_eq!(
        points(&o["line"]),
        points(&s.document()["inputs"]["centreline"]),
        "the design line is not the centreline, point for point"
    );
    let checks = &o["checks"];
    assert_eq!(checks["r_min_m"], json!(r_min));
    assert_eq!(checks["radius_ok"], false, "{checks}");
    let measured = checks["measured_min_radius_m"].as_f64().unwrap();
    assert!(
        (measured - bend_r).abs() < 0.05 * bend_r,
        "the measured minimum radius is {measured}; the bend has {bend_r}"
    );
    assert!(checks["radius_check_chord_m"].as_f64().unwrap() > 0.0);
    // The line does not hold the minimum radius: its bends widen by their own radius.
    assert_eq!(o["widening_floor"], "none", "{o}");

    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert_eq!(
        s.document()["stages"]["alignment"]["results"]["methods"],
        json!(["centreline"])
    );
}

#[test]
fn a_straight_centreline_passes_the_radius_check() {
    let straight: Vec<Pt> = (0..=30)
        .map(|i| [500_000.0 + 50.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = design(&straight);
    s.align(method(), &Value::Null, None).unwrap();
    let o = option(&s, "centreline");
    assert_eq!(points(&o["line"]), straight);
    assert_eq!(o["checks"]["radius_ok"], true, "{}", o["checks"]);
}

#[test]
fn with_two_banks_and_no_centreline_the_line_is_the_one_the_stage_builds() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd");
    let mut s = Session::open(&std::fs::read_to_string(path).unwrap()).unwrap();
    let inputs = s.document()["inputs"].clone();
    assert!(inputs["centreline"].is_null(), "the example has banks only");
    // As `Session::base_line` builds it.
    let built = build_centreline(
        &points(&inputs["left_bank"]),
        &points(&inputs["right_bank"]),
        20.0,
        5,
        2,
    )
    .unwrap();
    s.align(method(), &Value::Null, None).unwrap();
    assert_eq!(points(&option(&s, "centreline")["line"]), built);
}

#[test]
fn the_centreline_option_sits_beside_the_rolling_circle_option() {
    let r_min = r_min(&design(&[[500_000.0, 600_000.0], [501_500.0, 600_000.0]]));
    let mut s = design(&line_with_bend(r_min / 2.0));
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let rolled = option(&s, "rolling_circle").clone();
    s.align(method(), &Value::Null, None).unwrap();
    s.align(method(), &Value::Null, None).unwrap();
    assert_eq!(s.document()["options"].as_array().unwrap().len(), 2);
    assert_eq!(option(&s, "rolling_circle"), &rolled);
    assert_eq!(
        s.document()["stages"]["alignment"]["results"]["methods"],
        json!(["rolling_circle", "centreline"])
    );
    // A rerun of the stage runs both again and keeps both.
    s.run_stage(Stage::Alignment).unwrap();
    assert_eq!(s.document()["options"].as_array().unwrap().len(), 2);
    assert_eq!(method().name(), "centreline");
}
