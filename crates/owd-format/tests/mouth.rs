//! The river mouth (spec 0017, ticket 4, its document part carried into ticket 6): a design states
//! its mouth, the river inputs are stored running from it, and every river chainage counts from it.
//!
//! The river is the synthetic river in UTM 30N, shifted into the zone. Its centreline is given in its
//! own direction (from the mouth) or the other way round (from upstream).
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_format::design::crs::Crs;
use owd_format::design::{AlignMethod, MouthAt, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// The synthetic river's centreline, from its first point (the mouth in these tests).
fn centre() -> Vec<Pt> {
    synthetic_river(RiverParams::default())
        .unwrap()
        .centreline
        .iter()
        .map(|p| [p[0] + SHIFT[0], p[1] + SHIFT[1]])
        .collect()
}

fn polygon_text() -> String {
    let ring: Vec<Pt> = synthetic_river(RiverParams::default())
        .unwrap()
        .polygon
        .exterior
        .iter()
        .map(|p| [p[0] + SHIFT[0], p[1] + SHIFT[1]])
        .collect();
    format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
        json!([ring])
    )
}

/// A sized design with `line` as its centreline.
fn design(line: &[Pt]) -> Session {
    let mut s = Session::new("Mouth", "2026-11-20");
    s.import(&[
        (Role::Centreline, &line_text(line)),
        (Role::RiverPolygon, &polygon_text()),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

/// A point in the project CRS as longitude and latitude.
fn lon_lat(p: Pt) -> [f64; 2] {
    match Crs::from_epsg(32630).unwrap() {
        Crs::Utm(u) => u.inverse(p[0], p[1]).unwrap(),
        Crs::Wgs84 => unreachable!(),
    }
}

fn stored_centre(s: &Session) -> Vec<Pt> {
    s.document()["inputs"]["centreline"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn close(a: Pt, b: Pt) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

#[test]
fn a_centreline_given_from_upstream_is_stored_from_the_mouth_and_gives_the_same_design() {
    let c = centre();
    let mouth = lon_lat(c[0]);
    // Given from the mouth, and given the other way round: the same mouth.
    let mut forward = design(&c);
    let mut reversed: Vec<Pt> = c.clone();
    reversed.reverse();
    let mut backward = design(&reversed);
    let r = forward.set_mouth(MouthAt::LonLat(mouth), None).unwrap();
    assert!(!r.turned, "already from the mouth");
    let r = backward.set_mouth(MouthAt::LonLat(mouth), None).unwrap();
    assert!(r.turned, "turned to run from the mouth");
    assert!(close(stored_centre(&backward)[0], c[0]));
    assert!(close(r.point, c[0]));
    for s in [&mut forward, &mut backward] {
        s.align(AlignMethod::RollingCircle, &Value::Null, None)
            .unwrap();
    }
    // The same design: the same line, from the mouth, with the same bends at the same chainages.
    let (a, b) = (
        &forward.document()["stages"]["alignment"]["results"],
        &backward.document()["stages"]["alignment"]["results"],
    );
    assert_eq!(a["line"], b["line"]);
    assert_eq!(a["bends"], b["bends"]);
    assert_eq!(a["cut_offs"], b["cut_offs"]);
    let first = &a["line"]["coordinates"][0];
    assert!(close(
        [first[0].as_f64().unwrap(), first[1].as_f64().unwrap()],
        c[0]
    ));
    assert_eq!(
        forward.document()["inputs"]["mouth"],
        json!({"point": c[0], "stated_as": "point"})
    );
}

#[test]
fn downstream_with_no_water_level_and_no_terrain_is_refused_with_river_mouth_unknown() {
    let mut s = design(&centre());
    let before = s.to_text();
    let e = s
        .set_mouth(MouthAt::Downstream, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("RIVER_MOUTH_UNKNOWN"), "{e}");
    assert!(e.contains("give the mouth as a point"), "{e}");
    assert_eq!(s.to_text(), before, "a refusal changes nothing");
}

#[test]
fn downstream_takes_the_lower_end_and_turns_every_river_chainage_with_the_line() {
    let c = centre();
    let len = owd_core::geom::linear::length(&c);
    // Given from upstream: the water falls along the line as given, so its end is the mouth.
    let mut reversed = c.clone();
    reversed.reverse();
    let mut s = design(&reversed);
    s.set_water_profile(&[[0.0, 12.0], [len, 10.0]]).unwrap();
    s.add_waterway_reach(0.0, Some(1000.0), "canal", false)
        .unwrap();
    let r = s.set_mouth(MouthAt::Downstream, None).unwrap();
    assert!(r.turned);
    assert!(close(stored_centre(&s)[0], c[0]));
    let d = s.document();
    assert_eq!(d["inputs"]["mouth"]["stated_as"], "downstream");
    // The level at the mouth is still 10 m, at chainage 0 now.
    let pts = d["inputs"]["water_level"]["points"].as_array().unwrap();
    assert_eq!(pts[0], json!([0.0, 10.0]));
    assert!((pts[1][0].as_f64().unwrap() - len).abs() < 1e-6);
    assert_eq!(pts[1][1], 12.0);
    // The canal that ran 0 to 1000 m from the old start is the last 1000 m from the mouth.
    let reach = &d["inputs"]["waterway_reaches"][0];
    assert!(
        (reach["start_m"].as_f64().unwrap() - (len - 1000.0)).abs() < 1e-6,
        "{reach}"
    );
    assert!(reach["end_m"].is_null(), "to the end of the river: {reach}");
    // Stating the same mouth again turns nothing.
    let again = s.set_mouth(MouthAt::Downstream, None).unwrap();
    assert!(!again.turned);
}

#[test]
fn a_new_mouth_makes_the_stages_that_read_the_river_stale() {
    let c = centre();
    let mut reversed = c.clone();
    reversed.reverse();
    let mut s = design(&reversed);
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    s.set_mouth(MouthAt::LonLat(lon_lat(c[0])), None).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
}

#[test]
fn banks_imported_after_the_mouth_run_from_it_and_keep_their_names() {
    // Two banks either side of the river, given running away from the mouth's end... and back.
    let c = centre();
    let side = |d: f64| -> Vec<Pt> { c.iter().map(|p| [p[0], p[1] + d]).collect() };
    // The river runs from its last point to its first (the first is the mouth): looking downstream
    // (towards decreasing x), the left bank is on the south.
    let (left, right) = (side(-60.0), side(60.0));
    let mut a = design(&c);
    a.set_mouth(MouthAt::LonLat(lon_lat(c[0])), None).unwrap();
    let report = a
        .import(&[
            (Role::LeftBank, &line_text(&left)),
            (Role::RightBank, &line_text(&right)),
        ])
        .unwrap();
    let d = a.document();
    let first = |k: &str| {
        let p = &d["inputs"][k]["coordinates"][0];
        [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]
    };
    // Every line runs from the mouth.
    assert!(close(first("centreline"), c[0]));
    assert!((first("left_bank")[0] - c[0][0]).abs() < 1e-6);
    assert!((first("right_bank")[0] - c[0][0]).abs() < 1e-6);
    // Named looking downstream, the banks keep their names, and the side check passes.
    assert!(first("left_bank")[1] < first("right_bank")[1]);
    assert!(
        report.warnings.iter().all(|w| !w.contains("side")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn a_mouth_text_is_read_one_way_for_every_surface() {
    use owd_format::design::parse_mouth;
    assert_eq!(parse_mouth(" Downstream ").unwrap(), MouthAt::Downstream);
    assert_eq!(
        parse_mouth("-1.5, 5.25").unwrap(),
        MouthAt::LonLat([-1.5, 5.25])
    );
    for bad in ["", "upstream", "1,2,3", "200,5"] {
        assert!(parse_mouth(bad).is_err(), "{bad}");
    }
}
