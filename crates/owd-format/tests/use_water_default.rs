//! The water-aware default of the rolling circle (ticket #488). With `use_water` unset, the line
//! keeps to the water whenever the document has a river polygon: one the user gave, or one the
//! import built from the two banks. A centreline alone has no polygon, so the default is off. The
//! option records the value used, also when it came from the default.
//!
//! The river is closed-form: two sine banks 150 m apart, 6 km long, 600 m amplitude and 1.2 km
//! wavelength. Its bends (radius about 60 m) are far tighter than the 255 m the vessel needs, so a
//! line held in the water differs from one that is not.
//!
//! Label: property

use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A sine line `off` metres north of the middle of the river.
fn sine(off: f64) -> Vec<Pt> {
    (0..=600)
        .map(|i| {
            let x = 10.0 * f64::from(i);
            let y = 600.0 * libm::sin(2.0 * std::f64::consts::PI * x / 1200.0) + off;
            [500_000.0 + x, 600_000.0 + y]
        })
        .collect()
}

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn polygon_text(ring: &[Pt]) -> String {
    format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    )
}

/// The ring of the two banks with one more vertex on its closing side: a polygon the user gave,
/// not the one the import builds.
fn given_ring() -> Vec<Pt> {
    let (l, r) = (sine(75.0), sine(-75.0));
    let mut ring = l.clone();
    ring.extend(r.iter().rev());
    let (a, b) = (ring[ring.len() - 1], ring[0]);
    ring.push([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    ring.push(ring[0]);
    ring
}

/// The inputs of each case.
#[derive(Clone, Copy)]
enum Case {
    /// Two banks, no polygon given: the import builds one.
    TwoBanks,
    /// Two banks and a polygon the user gave.
    GivenPolygon,
    /// A centreline alone: no polygon, no banks.
    CentrelineAlone,
}

fn sized(case: Case) -> Session {
    let (left, right) = (line_text(&sine(75.0)), line_text(&sine(-75.0)));
    let (centre, given) = (line_text(&sine(0.0)), polygon_text(&given_ring()));
    let items: Vec<(Role, &str)> = match case {
        Case::TwoBanks => vec![(Role::LeftBank, &left), (Role::RightBank, &right)],
        Case::GivenPolygon => vec![
            (Role::LeftBank, &left),
            (Role::RightBank, &right),
            (Role::RiverPolygon, &given),
        ],
        Case::CentrelineAlone => vec![(Role::Centreline, &centre)],
    };
    let mut s = Session::new("Use water", "2026-11-10");
    s.import(&items).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

/// The rolling-circle option after a run with `overrides`, and the session.
fn rolled(case: Case, overrides: Value) -> (Value, Session) {
    let mut s = sized(case);
    s.align(AlignMethod::RollingCircle, &overrides, None)
        .unwrap();
    let option = s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "rolling_circle")
        .expect("a rolling-circle option")
        .clone();
    (option, s)
}

#[test]
fn two_banks_and_no_polygon_given_are_water_aware_by_default() {
    let (default, s) = rolled(Case::TwoBanks, json!({}));
    assert!(
        s.document()["inputs"]["river_polygon"].is_object(),
        "the import builds the polygon from the two banks"
    );
    let (on, _) = rolled(Case::TwoBanks, json!({"use_water": true}));
    let (off, _) = rolled(Case::TwoBanks, json!({"use_water": false}));
    assert_eq!(
        default["line"], on["line"],
        "the default line is not the water-aware line"
    );
    assert_ne!(
        default["line"], off["line"],
        "the water-aware and the plain line are the same: the case cannot tell them apart"
    );
    assert_eq!(default["settings"]["use_water"], json!(true));
    assert_eq!(off["settings"]["use_water"], json!(false));
}

#[test]
fn a_given_polygon_stays_water_aware_by_default() {
    let (default, _) = rolled(Case::GivenPolygon, json!({}));
    let (on, _) = rolled(Case::GivenPolygon, json!({"use_water": true}));
    let (off, _) = rolled(Case::GivenPolygon, json!({"use_water": false}));
    assert_eq!(default["line"], on["line"]);
    assert_ne!(default["line"], off["line"]);
    assert_eq!(default["settings"]["use_water"], json!(true));
}

#[test]
fn a_centreline_alone_is_not_water_aware_by_default() {
    let (default, s) = rolled(Case::CentrelineAlone, json!({}));
    assert!(s.document()["inputs"]["river_polygon"].is_null());
    let (off, _) = rolled(Case::CentrelineAlone, json!({"use_water": false}));
    assert_eq!(default["line"], off["line"]);
    assert_eq!(default["settings"]["use_water"], json!(false));
}

#[test]
fn the_stage_keeps_an_unset_setting_unset() {
    // The option records the value used; the stage's settings keep what the user set, so a later
    // change of the inputs still takes the default.
    let (option, s) = rolled(Case::TwoBanks, json!({}));
    assert_eq!(option["settings"]["use_water"], json!(true));
    assert_eq!(
        s.document()["stages"]["alignment"]["settings"]["rolling_circle"]["use_water"],
        Value::Null
    );
}

#[test]
fn a_stored_setting_is_kept_and_honoured_by_a_rerun() {
    let mut s = sized(Case::TwoBanks);
    s.set_alignment_settings(AlignMethod::RollingCircle, &json!({"use_water": false}))
        .unwrap();
    assert_eq!(
        s.document()["stages"]["alignment"]["settings"]["rolling_circle"]["use_water"],
        json!(false)
    );
    s.run_stale_with(None).unwrap();
    let (off, _) = rolled(Case::TwoBanks, json!({"use_water": false}));
    let option = &s.document()["options"][0];
    assert_eq!(option["line"], off["line"]);
    assert_eq!(option["settings"]["use_water"], json!(false));
}

#[test]
fn a_bad_setting_is_refused_by_name_and_changes_nothing() {
    let mut s = sized(Case::TwoBanks);
    let before = s.to_text();
    let e = s
        .set_alignment_settings(AlignMethod::RollingCircle, &json!({"use_water": "yes"}))
        .unwrap_err();
    assert!(e.to_string().contains("use_water"), "{e}");
    assert_eq!(s.to_text(), before);
}
