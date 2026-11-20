//! The reference line as a typed stage (spec 0017, ticket 7): a pure function from a
//! `ReferenceLineIn` to a `ReferenceLine`, tested alone; stored as a stage record under
//! `stages.reference_line`, its `depends_on` the hash of the input it read (no hash of its own); a
//! line stored the old way (`inputs.reference_line`) moves there when the file opens.
//!
//! Label: property

use owd_format::design::reference_line::{ReferenceLineIn, reference_line_stage};
use owd_format::design::{Role, Session, StageStatus};
use owd_format::types::{Polyline, RiverPolygon};
use serde_json::json;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn at(x: f64, y: f64) -> [f64; 2] {
    [x + 500_000.0, y + 600_000.0]
}

type Line = Vec<[f64; 2]>;

/// A straight river 100 m wide and 2 km long: the left bank, the right bank and the polygon's ring.
fn banks() -> (Line, Line, Line) {
    let left: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, 50.0)).collect();
    let right: Vec<[f64; 2]> = (0..=100).map(|i| at(20.0 * i as f64, -50.0)).collect();
    let mut ring = right.clone();
    ring.extend(left.iter().rev());
    ring.push(ring[0]);
    (left, right, ring)
}

fn input() -> ReferenceLineIn {
    let (left, right, ring) = banks();
    ReferenceLineIn {
        left_bank: Polyline(left),
        right_bank: Polyline(right),
        river_polygon: RiverPolygon {
            shell: Polyline(ring),
            holes: vec![],
        },
        ..ReferenceLineIn::default()
    }
}

#[test]
fn the_stage_alone_lays_the_line_midway_in_the_water() {
    let out = reference_line_stage(&input()).unwrap();
    assert!(out.line.len() > 10);
    assert!(out.line.0.iter().all(|p| (p[1] - 600_000.0).abs() < 0.5));
    assert!(out.outside_water.is_empty());
    // The stored form reads back as the same output.
    let v = serde_json::to_value(&out).unwrap();
    assert_eq!(v["line"]["type"], "LineString");
    assert_eq!(
        serde_json::from_value::<owd_format::design::reference_line::ReferenceLine>(v).unwrap(),
        out
    );
}

#[test]
fn the_stage_refuses_a_river_with_no_banks_by_code() {
    let e = reference_line_stage(&ReferenceLineIn::default()).unwrap_err();
    assert_eq!(
        (e.stage.as_str(), e.code.as_str()),
        ("reference_line", "REFERENCE_LINE_NO_BANKS")
    );
}

fn study() -> Session {
    let (left, right, ring) = banks();
    let line = |p: &[[f64; 2]]| {
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(p)
        )
    };
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    );
    let mut s = Session::new("t", "2026-11-20");
    s.import(&[
        (Role::RiverPolygon, polygon.as_str()),
        (Role::LeftBank, line(&left).as_str()),
        (Role::RightBank, line(&right).as_str()),
    ])
    .unwrap();
    s
}

#[test]
fn the_session_stores_a_stage_record_with_a_hash() {
    let mut s = study();
    s.run_reference_line().unwrap();
    let d = s.document();
    assert!(
        d["inputs"].get("reference_line").is_none(),
        "no longer an input"
    );
    let rec = &d["stages"]["reference_line"];
    assert_eq!(rec["results"]["line"]["type"], "LineString");
    assert!(
        rec["settings"]["seed_pull_weight"].is_number(),
        "{}",
        rec["settings"]
    );
    assert!(rec["depends_on"].is_string());
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.reference_line_status(), StageStatus::Fresh);
}

#[test]
fn a_line_stored_the_old_way_moves_to_the_stage_and_is_stale() {
    let mut s = study();
    s.run_reference_line().unwrap();
    let mut doc = s.document().clone();
    let rec = doc["stages"]["reference_line"].take();
    doc["stages"]
        .as_object_mut()
        .unwrap()
        .remove("reference_line");
    doc["inputs"]["reference_line"] = json!({
        "line": rec["results"]["line"],
        "parameters": rec["settings"],
        "depends_on": "an old hash",
        "low_confidence": [],
        "outside_water": [],
        "notes": [],
    });
    let back = Session::open(&doc.to_string()).unwrap();
    assert!(back.document()["inputs"].get("reference_line").is_none());
    assert_eq!(
        back.document()["stages"]["reference_line"]["results"]["line"],
        rec["results"]["line"]
    );
    assert_eq!(back.reference_line_status(), StageStatus::Stale);
}
