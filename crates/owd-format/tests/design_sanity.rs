//! The design sanity check in the document: every option carries `sanity` (`checked`, `warnings`).
//! A line with a bend under R_min says so, with its stretch and radius; a repaired line says
//! nothing; a warning never marks the option as failed.
//!
//! The river is a closed-form sine centreline alone: 600 m amplitude, 1.2 km wavelength (bends of
//! about 60 m radius against R_min 255 m).
//!
//! Label: property

use owd_format::design::export::failed_checks;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

fn sine_text() -> String {
    let pts: Vec<[f64; 2]> = (0..=300)
        .map(|i| {
            let x = 10.0 * f64::from(i);
            [
                500_000.0 + x,
                600_000.0 + 600.0 * libm::sin(2.0 * std::f64::consts::PI * x / 1200.0),
            ]
        })
        .collect();
    format!(
        r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn session() -> Session {
    let mut s = Session::new("Sanity", "2026-11-19");
    s.import(&[(Role::Centreline, &sine_text())]).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

fn option(s: &Session, method: AlignMethod) -> Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method.name())
        .unwrap()
        .clone()
}

#[test]
fn the_unrepaired_line_warns_of_its_bends_and_is_not_marked_failed_by_the_warning() {
    let mut s = session();
    s.align(AlignMethod::Centreline, &Value::Null, None)
        .unwrap();
    let o = option(&s, AlignMethod::Centreline);
    assert_eq!(o["sanity"]["checked"], json!(true));
    let w = o["sanity"]["warnings"].as_array().unwrap();
    assert!(!w.is_empty(), "no warning on a line of 60 m bends");
    assert!(
        w.iter()
            .all(|m| m.as_str().unwrap().contains("tighter than R_min")),
        "{w:?}"
    );
    // The radius check fails here by itself; the warning adds no failed check of its own.
    assert_eq!(failed_checks(&o), vec!["R_min"]);
}

#[test]
fn the_repaired_line_has_no_warning() {
    let mut s = session();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    let o = option(&s, AlignMethod::RollingCircle);
    assert_eq!(o["checks"]["radius_ok"], json!(true));
    assert_eq!(o["sanity"]["checked"], json!(true));
    assert_eq!(o["sanity"]["warnings"], json!([]));
}
