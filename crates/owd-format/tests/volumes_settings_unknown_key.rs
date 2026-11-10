//! The volumes stage refuses a setting it does not know.
//!
//! A misspelt setting must not pass in silence, with the default used in its place. The stage
//! refuses it and names it, whether the caller gives it or the document holds it. Nothing is
//! written to the document.
//!
//! Label: property

use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Terrain};
use owd_format::geotiff;
use serde_json::{Value, json};
use std::path::PathBuf;

/// `station_m`, misspelt.
const WRONG: &str = "statoin_m";

fn web(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// The example design: sizing, the rolling circle and volumes, all fresh, with its terrain.
fn example() -> (String, geotiff::Dem) {
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let dem = geotiff::read_bytes(&std::fs::read(web("river-dem.tif")).unwrap()).unwrap();
    (text, dem)
}

#[test]
fn a_setting_the_caller_gives_and_the_stage_does_not_know_is_refused_by_name() {
    let (text, dem) = example();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    let mut s = Session::open(&text).unwrap();
    let before = s.to_text();
    let run = s.run_volumes(&json!({ WRONG: 50.0 }), Some(&terrain as Terrain));
    match run {
        Ok(()) => panic!(
            "the volumes stage ran with the unknown setting `{WRONG}`; its settings are now {}",
            s.document()["stages"]["volumes"]["settings"]
        ),
        Err(e) => assert!(
            e.to_string().contains(WRONG),
            "the error does not name `{WRONG}`: {e}"
        ),
    }
    assert_eq!(s.to_text(), before, "a refused run changed the document");
}

#[test]
fn a_setting_the_document_holds_and_the_stage_does_not_know_is_refused_by_name() {
    let (text, dem) = example();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["stages"]["volumes"]["settings"][WRONG] = json!(50.0);
    // The door may refuse the document, or the stage may refuse to run. Either names the setting.
    let said = match Session::open(&doc.to_string()) {
        Err(e) => e.to_string(),
        Ok(mut s) => match s.run_volumes(&Value::Null, Some(&terrain as Terrain)) {
            Err(e) => e.to_string(),
            Ok(()) => panic!(
                "the document opened and the volumes stage ran with the unknown setting `{WRONG}`"
            ),
        },
    };
    assert!(
        said.contains(WRONG),
        "the error does not name `{WRONG}`: {said}"
    );
}
