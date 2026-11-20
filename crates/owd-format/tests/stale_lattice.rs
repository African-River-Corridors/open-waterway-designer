//! QA on PR 558, item 2: a pool-lattice line from a stale optimise stage is never the design line the
//! later stages read. After a new alignment run, the map, the exports, the drawings and the volumes
//! read the alignment's fresh line, and the session says why.
//!
//! The lattice's line is put into the document by hand (a straight copy of the alignment's line,
//! moved 5 m), with the optimise stage's hash made fresh, so no terrain or pool search is needed.
//!
//! Label: property

use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn design() -> Session {
    let centre: Vec<[f64; 2]> = (0..=30)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Stale lattice", "2026-11-20");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(centre)
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::Centreline, &Value::Null, None)
        .unwrap();
    s
}

/// The design with a fresh optimise stage whose refined line is the alignment's, moved 5 m north.
fn with_fresh_lattice() -> Session {
    let s = design();
    let mut doc = s.document().clone();
    let mut lattice = doc["stages"]["alignment"]["results"].clone();
    lattice["id"] = json!("lattice");
    lattice["name"] = json!("Pool lattice");
    lattice["method"] = json!("lattice");
    for p in lattice["line"]["coordinates"].as_array_mut().unwrap() {
        p[1] = json!(p[1].as_f64().unwrap() + 5.0);
    }
    let engine = doc["engine_version"].clone();
    let op = &mut doc["stages"]["optimise"];
    op["settings"] = json!({});
    op["results"] = json!({"design_line": lattice});
    op["engine_version"] = engine;
    op["depends_on"] = json!("0".repeat(64));
    let s = Session::open(&doc.to_string()).unwrap();
    doc["stages"]["optimise"]["depends_on"] = json!(s.stage_hash(Stage::Optimise));
    let s = Session::open(&doc.to_string()).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    s
}

#[test]
fn a_fresh_lattice_line_is_the_design_line_and_a_stale_one_is_not() {
    let mut s = with_fresh_lattice();
    assert_eq!(s.design_lines()[0]["id"], "lattice");
    assert!(s.stale_refined_line_note().is_none());
    // A new alignment run: the optimise stage goes stale, and its line is no longer read.
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Stale);
    let lines = s.design_lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["id"], "rolling-circle");
    // The map and the exports read the same line.
    let map = s.map_geometry().unwrap();
    assert_eq!(map["options"][0]["id"], "rolling-circle");
    let gj: Value = serde_json::from_str(&s.export_geojson(false, None).unwrap()).unwrap();
    assert_eq!(gj["features"][0]["properties"]["id"], "rolling-circle");
    // The session says so, in words, for `owd status`.
    let note = s.stale_refined_line_note().expect("a note");
    assert!(
        note.contains("pool lattice") && note.contains("owd optimise"),
        "{note}"
    );
}
