//! The optimise stage refuses a setting it does not know.
//!
//! A misspelt setting must not pass in silence, with the default used in its place. The stage
//! refuses it and names it, whether the caller gives it or the document holds it. It checks the
//! settings first, before it looks for terrain or does any work, so the refusal costs nothing.
//!
//! Label: property

use owd_format::design::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

/// `lifts_m`, misspelt.
const WRONG: &str = "lift_m";

/// The example design: sizing and the rolling circle fresh, the optimise stage not run, and a
/// terrain file named.
fn example() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_setting_the_caller_gives_and_the_stage_does_not_know_is_refused_by_name() {
    let mut s = Session::open(&example()).unwrap();
    let before = s.to_text();
    // No terrain: a stage that checked its settings first never asks for it.
    let e = s
        .run_optimise(&json!({ WRONG: [0.5] }), None)
        .expect_err("the stage cannot run")
        .to_string();
    assert!(e.contains(WRONG), "the error does not name `{WRONG}`: {e}");
    assert_eq!(s.to_text(), before, "a refused run changed the document");
}

#[test]
fn a_setting_the_document_holds_and_the_stage_does_not_know_is_refused_by_name() {
    let mut doc: Value = serde_json::from_str(&example()).unwrap();
    doc["stages"]["optimise"]["settings"] = json!({ WRONG: [0.5] });
    // The door may refuse the document, or the stage may refuse to run. Either names the setting.
    let said = match Session::open(&doc.to_string()) {
        Err(e) => e.to_string(),
        Ok(mut s) => s
            .run_optimise(&Value::Null, None)
            .expect_err("the stage cannot run")
            .to_string(),
    };
    assert!(
        said.contains(WRONG),
        "the error does not name `{WRONG}`: {said}"
    );
}
