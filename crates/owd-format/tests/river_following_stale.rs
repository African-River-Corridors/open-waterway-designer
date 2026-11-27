//! The river-following line in the session (OPT-4, part 5): a change of rule quality makes the
//! sizing and the alignment stale, and "run what is due" builds the line again from the river.
//! Other designs keep their settings and hashes.
//!
//! Label: property

use owd_core::alignment::river_following::fixtures;
use owd_core::geom::Pt;
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn geo(kind: &str, coords: Value) -> String {
    format!(r#"{{"type":"{kind}"{UTM30},"coordinates":{coords}}}"#)
}

fn design(quality: &str) -> Session {
    let wl = 2000.0;
    let centre: Vec<Pt> =
        fixtures::sine(fixtures::sine_amplitude_for_radius(250.0, wl), wl, 4.0 * wl)
            .iter()
            .map(|p| [p[0] + SHIFT[0], p[1] + SHIFT[1]])
            .collect();
    let poly = fixtures::polygon(&centre, 80.0);
    let mut s = Session::new("River", "2026-11-26");
    s.import(&[
        (Role::Centreline, &geo("LineString", json!(centre))),
        (Role::RiverPolygon, &geo("Polygon", json!([poly.exterior]))),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", quality)])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

#[test]
fn a_change_of_rule_quality_makes_the_line_stale_and_run_builds_it_again() {
    let mut s = design("A");
    s.align(AlignMethod::RiverFollowing, &Value::Null, None)
        .unwrap();
    let before = s.document()["stages"]["alignment"]["results"].clone();
    assert_eq!(before["method"], "river_following");
    let r_before = before["checks"]["r_min_m"].as_f64().unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    // The rule quality changes: R_min changes, the line is stale.
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "C")])
        .unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    let ran = s.run_stale_with(None).unwrap();
    assert!(ran.contains(&Stage::Alignment), "{ran:?}");
    let after = s.document()["stages"]["alignment"]["results"].clone();
    let r_after = after["checks"]["r_min_m"].as_f64().unwrap();
    eprintln!("R_min {r_before} -> {r_after}");
    assert_eq!(after["method"], "river_following");
    assert!(r_after < r_before);
    assert_ne!(before["line"], after["line"]);
    assert_eq!(after["checks"]["radius_ok"], true);
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    // A setting of the method makes it stale too, and is stored only when it is not its default.
    s.set_alignment_settings(
        AlignMethod::RiverFollowing,
        &json!({"seed_tolerance_m": 3.0}),
    )
    .unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    assert_eq!(
        s.document()["stages"]["alignment"]["settings"]["river_following"],
        json!({"seed_tolerance_m": 3.0})
    );
}

#[test]
fn a_design_that_never_used_it_keeps_its_settings() {
    let mut s = design("B");
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let st = &s.document()["stages"]["alignment"]["settings"];
    assert!(st.get("river_following").is_none(), "{st}");
}
