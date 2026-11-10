//! The optimise settings are one typed value, with its defaults in one place.
//!
//! What the stage writes into the document when nobody sets anything equals the defaults of
//! `OptimiseSettings`.
//!
//! The stage takes minutes in a debug build, so this test does not run it. It reads the settings
//! the stage wrote into a committed example, where the user set the candidate sites and nothing
//! else.
//!
//! Label: property

use owd_format::design::{OptimiseSettings, Session};
use serde_json::{Value, json};
use std::path::PathBuf;

#[test]
fn the_settings_the_stage_wrote_into_the_example_are_the_typed_defaults() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web/river-rolling-circle.owd");
    let s = Session::open(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut wrote: Value = s.document()["stages"]["optimise"]["settings"].clone();
    assert_eq!(
        wrote["candidate_sites_m"],
        json!([1200.0]),
        "the example sets one candidate site"
    );
    // Put back the one setting the user gave.
    wrote["candidate_sites_m"] = Value::Null;
    assert_eq!(wrote, OptimiseSettings::default().to_value());
}
