//! A side check that fails at most stations means one fact is wrong (the direction, or the names),
//! not hundreds (ticket B2b). Over a stated share of the checked stations, the import says so in
//! one plain message; under it, the per-check report stays.
//!
//! Label: property

use owd_format::design::session::{SIDE_WHOLESALE_SHARE, side_wholesale_message};
use owd_format::design::{Role, Session};
use serde_json::json;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn line(pts: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn bank(y: f64) -> Vec<[f64; 2]> {
    (0..=32)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0 + y])
        .collect()
}

#[test]
fn the_share_is_one_named_constant_with_a_test_on_each_side() {
    const { assert!(SIDE_WHOLESALE_SHARE > 0.0 && SIDE_WHOLESALE_SHARE < 1.0) };
    let checked = 1000;
    let at = (SIDE_WHOLESALE_SHARE * checked as f64).round() as usize;
    assert!(
        side_wholesale_message(at + 1, checked).is_some(),
        "just over the share"
    );
    assert!(
        side_wholesale_message(at, checked).is_none(),
        "at the share"
    );
    assert!(
        side_wholesale_message(at - 1, checked).is_none(),
        "just under the share"
    );
    let m = side_wholesale_message(checked, checked).unwrap();
    assert!(
        m.contains("wrong way round") && m.contains("--downstream"),
        "{m}"
    );
}

#[test]
fn a_local_side_fault_keeps_the_per_check_report() {
    // The right bank dives north past the left bank for a short stretch: it is on the wrong side
    // there, and only there. (A centreline that strays off the river is no side fault: the checks
    // allow it.) Banks that cross come in only through the unchecked import, which runs the same
    // bank checks.
    let right: Vec<[f64; 2]> = bank(-100.0)
        .into_iter()
        .map(|[x, y]| {
            let along = x - 500_000.0;
            [
                x,
                if (1500.0..=1600.0).contains(&along) {
                    600_150.0
                } else {
                    y
                },
            ]
        })
        .collect();
    let mut s = Session::new("Local fault", "2026-01-02");
    let report = s
        .import_unchecked(&[
            (Role::LeftBank, &line(&bank(100.0))),
            (Role::RightBank, &line(&right)),
            (Role::Centreline, &line(&bank(0.0))),
        ])
        .unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("the banks fail the `side` check at")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .all(|w| !w.contains("wrong way round")),
        "{:?}",
        report.warnings
    );
}
