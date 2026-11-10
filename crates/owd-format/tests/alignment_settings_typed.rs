//! The alignment settings are one typed value, with its defaults in one place.
//!
//! `AlignmentSettings` holds the settings of each method under the method's name, as the document
//! does. What a method writes into the document when nobody sets anything equals its defaults in
//! `AlignmentSettings`. A setting the caller gives changes that one setting and no other.
//!
//! Label: property

use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, AlignmentSettings, Session, Terrain};
use owd_format::geotiff;
use serde_json::{Value, json};
use std::path::PathBuf;

fn web(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// The example design (sizing fresh, a water level, a river polygon) with no alignment settings.
fn example_with_no_alignment_settings() -> Session {
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["stages"]["alignment"]["settings"] = json!({});
    Session::open(&doc.to_string()).unwrap()
}

fn written(s: &Session, method: &str) -> Value {
    s.document()["stages"]["alignment"]["settings"][method].clone()
}

#[test]
fn the_rolling_circle_with_no_settings_writes_the_typed_defaults() {
    let mut s = example_with_no_alignment_settings();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let defaults = AlignmentSettings::default().to_value();
    assert!(defaults["rolling_circle"].is_object());
    assert_eq!(written(&s, "rolling_circle"), defaults["rolling_circle"]);
}

#[test]
fn the_re_route_with_no_settings_writes_the_typed_defaults() {
    let mut s = example_with_no_alignment_settings();
    let dem = geotiff::read_bytes(&std::fs::read(web("river-dem.tif")).unwrap()).unwrap();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    s.align(
        AlignMethod::Reroute,
        &Value::Null,
        Some(&terrain as Terrain),
    )
    .unwrap();
    let defaults = AlignmentSettings::default().to_value();
    assert!(defaults["reroute"].is_object());
    assert_eq!(written(&s, "reroute"), defaults["reroute"]);
}

#[test]
fn a_setting_the_caller_gives_changes_that_setting_only() {
    let mut s = example_with_no_alignment_settings();
    s.align(AlignMethod::RollingCircle, &json!({"step_m": 20.0}), None)
        .unwrap();
    let mut want = AlignmentSettings::default().to_value()["rolling_circle"].clone();
    assert_ne!(want["step_m"], json!(20.0));
    want["step_m"] = json!(20.0);
    assert_eq!(written(&s, "rolling_circle"), want);
}
