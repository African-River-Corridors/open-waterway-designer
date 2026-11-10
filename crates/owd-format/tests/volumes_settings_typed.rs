//! The volumes settings are one typed value, with its defaults in one place.
//!
//! What the stage writes into the document when nobody sets anything equals the defaults of
//! `VolumesSettings`. A setting the caller gives changes that one setting and no other.
//!
//! Label: property

use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Terrain, VolumesSettings};
use owd_format::geotiff;
use serde_json::{Value, json};
use std::path::PathBuf;

fn web(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// The example design with its volumes stage emptied, and its terrain.
fn example_before_volumes() -> (Session, geotiff::Dem) {
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["stages"]["volumes"] = json!({
        "settings": null, "results": null, "depends_on": null, "engine_version": null,
    });
    let dem = geotiff::read_bytes(&std::fs::read(web("river-dem.tif")).unwrap()).unwrap();
    (Session::open(&doc.to_string()).unwrap(), dem)
}

#[test]
fn a_run_with_no_settings_writes_the_typed_defaults() {
    let (mut s, dem) = example_before_volumes();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    s.run_volumes(&Value::Null, Some(&terrain as Terrain))
        .unwrap();
    assert_eq!(
        s.document()["stages"]["volumes"]["settings"],
        VolumesSettings::default().to_value()
    );
}

#[test]
fn a_setting_the_caller_gives_changes_that_setting_only() {
    let (mut s, dem) = example_before_volumes();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    s.run_volumes(&json!({"station_m": 50.0}), Some(&terrain as Terrain))
        .unwrap();
    let mut want = VolumesSettings::default().to_value();
    assert_ne!(want["station_m"], json!(50.0));
    want["station_m"] = json!(50.0);
    assert_eq!(s.document()["stages"]["volumes"]["settings"], want);
}
