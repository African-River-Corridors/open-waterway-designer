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

/// A straight river 3 km long, given from upstream (x = 503 000) to its mouth (x = 500 000), sized,
/// with the centreline as its design line.
fn straight_from_upstream() -> Session {
    let line: Vec<Pt> = (0..=30)
        .map(|i| [503_000.0 - 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Straight", "2026-11-20");
    s.import(&[(Role::Centreline, &line_text(&line))]).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::Centreline, &Value::Null, None)
        .unwrap();
    s
}

/// The mouth of [`straight_from_upstream`], as longitude and latitude.
fn straight_mouth() -> MouthAt {
    MouthAt::LonLat(lon_lat([500_000.0, 600_000.0]))
}

/// `doc` with `edit` made, opened again.
fn edited(s: &Session, edit: impl FnOnce(&mut Value)) -> Session {
    let mut doc = s.document().clone();
    edit(&mut doc);
    Session::open(&doc.to_string()).unwrap()
}

/// QA on PR 558, item 4: the optimise stage's candidate weir sites are metres along the design line:
/// the turn counts them from the other end too.
#[test]
fn the_candidate_weir_sites_turn_with_the_river() {
    let s = straight_from_upstream();
    let mut s = edited(&s, |d| {
        d["stages"]["optimise"]["settings"] = json!({"candidate_sites_m": [500.0, 1000.0]});
    });
    assert!(s.set_mouth(straight_mouth(), None).unwrap().turned);
    assert_eq!(
        s.document()["stages"]["optimise"]["settings"]["candidate_sites_m"],
        json!([2000.0, 2500.0])
    );
}

/// QA on PR 558, item 5: a bank dataset imported after the mouth is stated, with its banks drawn
/// from upstream, stores its low-confidence stretches counted from the mouth, as the banks are.
#[test]
fn a_bank_dataset_imported_after_the_mouth_counts_its_stretches_from_it() {
    let mut s = straight_from_upstream();
    assert!(s.set_mouth(straight_mouth(), None).unwrap().turned);
    // Banks 100 m either side, drawn from upstream like the centreline was; a stretch 200 to 500 m
    // from where they start (the upstream end).
    let bank = |dy: f64| -> Vec<Pt> {
        (0..=30)
            .map(|i| [503_000.0 - 100.0 * f64::from(i), 600_000.0 + dy])
            .collect()
    };
    let file = json!({
        "type": "FeatureCollection",
        "crs": {"type": "name", "properties": {"name": "urn:ogc:def:crs:EPSG::32630"}},
        "bank_dataset": {
            "version": 1,
            "source": {"kind": "file", "name": "a test file", "date": "2026-11-20", "licence": null},
            "accuracy_m": 2.0,
            "low_confidence": [{"from_m": 200.0, "to_m": 500.0, "reason": "trees"}]
        },
        "features": [
            {"type": "Feature", "properties": {"role": "left_bank"},
             "geometry": {"type": "LineString", "coordinates": bank(-100.0)}},
            {"type": "Feature", "properties": {"role": "right_bank"},
             "geometry": {"type": "LineString", "coordinates": bank(100.0)}}
        ]
    });
    s.import_bank_dataset(&file.to_string()).unwrap();
    let d = s.document();
    assert!(
        (d["inputs"]["left_bank"]["coordinates"][0][0]
            .as_f64()
            .unwrap()
            - 500_000.0)
            .abs()
            < 1e-6
    );
    let lc = &d["inputs"]["banks"]["low_confidence"][0];
    assert!(
        (lc["from_m"].as_f64().unwrap() - 2500.0).abs() < 1e-6,
        "{lc}"
    );
    assert!((lc["to_m"].as_f64().unwrap() - 2800.0).abs() < 1e-6, "{lc}");
}

/// QA on PR 558, item 8: every river chainage the inputs hold turns with the river, the poses and
/// legs of a joined line change ends, and a reference line made from the other end is removed.
#[test]
fn every_chainage_input_turns_and_the_reference_line_is_removed() {
    let s = straight_from_upstream();
    let line = |a: f64, b: f64| json!({"type": "LineString", "coordinates": [[a, 600_000.0], [b, 600_000.0]]});
    let mut s = edited(&s, |d| {
        let inp = &mut d["inputs"];
        inp["bed_model"] =
            json!({"points": [[0.0, 2.0], [1000.0, 3.0]], "shape": "parabola", "season": null});
        inp["banks"] =
            json!({"low_confidence": [{"from_m": 100.0, "to_m": 300.0, "reason": "trees"}]});
        inp["seed_legs"] = json!([line(503_000.0, 502_000.0), line(502_000.0, 500_000.0)]);
        inp["fixed"] = json!({
            "start_pose": {"point": [503_000.0, 600_000.0], "heading_deg": 180.0},
            "end_pose": {"point": [500_000.0, 600_000.0], "heading_deg": 180.0},
            "must_follow": [line(501_000.0, 500_500.0)]
        });
        inp["reference_line"] =
            json!({"line": line(503_000.0, 500_000.0), "parameters": {}, "depends_on": "x"});
        d["low_confidence_width"] = json!({"min_width_m": 30.0, "stretches": [{"from_m": 0.0, "to_m": 400.0, "kind": "narrow"}]});
    });
    let r = s.set_mouth(straight_mouth(), None).unwrap();
    assert!(r.turned);
    assert!(
        r.notes.iter().any(|n| n.contains("reference line")),
        "{:?}",
        r.notes
    );
    let d = s.document();
    let inp = &d["inputs"];
    assert_eq!(
        inp["bed_model"]["points"],
        json!([[2000.0, 3.0], [3000.0, 2.0]])
    );
    assert_eq!(
        inp["banks"]["low_confidence"],
        json!([{"from_m": 2700.0, "to_m": 2900.0, "reason": "trees"}])
    );
    assert_eq!(
        d["low_confidence_width"]["stretches"],
        json!([{"from_m": 2600.0, "to_m": 3000.0, "kind": "narrow"}])
    );
    assert_eq!(
        inp["seed_legs"],
        json!([line(500_000.0, 502_000.0), line(502_000.0, 503_000.0)])
    );
    assert_eq!(
        inp["fixed"]["start_pose"],
        json!({"point": [500_000.0, 600_000.0], "heading_deg": 0.0})
    );
    assert_eq!(
        inp["fixed"]["end_pose"],
        json!({"point": [503_000.0, 600_000.0], "heading_deg": 0.0})
    );
    assert_eq!(
        inp["fixed"]["must_follow"],
        json!([line(500_500.0, 501_000.0)])
    );
    assert!(inp.get("reference_line").is_none());
    assert!(d["stages"].get("reference_line").is_none());
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}

/// QA on PR 558, item 8: a second mouth, at the other end, turns the river back, water and all.
#[test]
fn a_second_mouth_at_the_other_end_turns_the_river_back() {
    let mut s = straight_from_upstream();
    s.set_water_profile(&[[0.0, 12.0], [3000.0, 9.0]]).unwrap();
    let before = s.document()["inputs"].clone();
    assert!(s.set_mouth(straight_mouth(), None).unwrap().turned);
    assert_eq!(
        s.document()["inputs"]["water_level"]["points"][0],
        json!([0.0, 9.0])
    );
    let other = MouthAt::LonLat(lon_lat([503_000.0, 600_000.0]));
    assert!(s.set_mouth(other, None).unwrap().turned);
    let after = &s.document()["inputs"];
    assert_eq!(after["centreline"], before["centreline"]);
    assert_eq!(after["water_level"], before["water_level"]);
    assert_eq!(after["mouth"]["point"], json!([503_000.0, 600_000.0]));
}
