//! Banks made from imagery come in through `Session::import_unchecked`, which keeps two banks
//! that cross. They get the same bank checks as any import, and the crossing is reported, not
//! refused. The same banks through `Session::import` are refused.
//!
//! Label: property

use owd_format::design::{Role, Session};
use serde_json::json;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A bank 2 km long, `off` metres north of y = 600 000, that dives `dive` metres south between
/// 900 m and 1100 m.
fn bank(off: f64, dive: f64) -> String {
    let pts: Vec<[f64; 2]> = (0..=40)
        .map(|i| {
            let x = 50.0 * f64::from(i);
            let y = if (900.0..=1100.0).contains(&x) {
                off - dive
            } else {
                off
            };
            [500_000.0 + x, 600_000.0 + y]
        })
        .collect();
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

#[test]
fn crossing_banks_through_the_unchecked_import_are_reported_not_refused() {
    // The left bank dives 300 m south: it crosses the right bank, 200 m south of it.
    let (left, right) = (bank(100.0, 300.0), bank(-100.0, 0.0));
    let items = [
        (Role::LeftBank, left.as_str()),
        (Role::RightBank, right.as_str()),
    ];

    let mut s = Session::new("Unchecked", "2026-11-10");
    let report = s
        .import_unchecked(&items)
        .expect("the unchecked import keeps banks that cross");
    assert!(
        report.warnings.iter().any(|w| w.contains("cross")),
        "no warning says the banks cross: {:?}",
        report.warnings
    );
    assert!(s.document()["inputs"]["left_bank"].is_object());

    let mut strict = Session::new("Checked", "2026-11-10");
    let refused = strict
        .import(&items)
        .expect_err("the import refuses banks that cross");
    assert!(refused.to_string().contains("cross"), "{refused}");
}

#[test]
fn a_sound_pair_through_the_unchecked_import_has_no_warning() {
    let (left, right) = (bank(100.0, 0.0), bank(-100.0, 0.0));
    let mut s = Session::new("Unchecked", "2026-11-10");
    let report = s
        .import_unchecked(&[
            (Role::LeftBank, left.as_str()),
            (Role::RightBank, right.as_str()),
        ])
        .unwrap();
    assert_eq!(report.warnings, Vec::<String>::new());
}
