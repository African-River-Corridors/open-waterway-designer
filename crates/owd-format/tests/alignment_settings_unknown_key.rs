//! The alignment stage refuses a setting it does not know.
//!
//! A misspelt setting must not pass in silence, with the default used in its place. The stage
//! refuses it and names it, whether the caller gives it or the document holds it. It checks the
//! settings first, before it looks for terrain or does any work.
//!
//! Label: property

use owd_format::design::{AlignMethod, Session, Stage};
use serde_json::{Value, json};
use std::path::PathBuf;

/// `step_m`, misspelt: a rolling-circle setting.
const WRONG_STEP: &str = "stepp_m";
/// `corridor_sweep_m`, misspelt: a re-route setting.
const WRONG_SWEEP: &str = "corridor_sweep";

/// The example design: sizing and the rolling circle fresh.
fn example() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_rolling_circle_setting_the_stage_does_not_know_is_refused_by_name() {
    let mut s = Session::open(&example()).unwrap();
    let before = s.to_text();
    let run = s.align(
        AlignMethod::RollingCircle,
        &json!({ WRONG_STEP: 20.0 }),
        None,
    );
    match run {
        Ok(()) => panic!(
            "the rolling circle ran with the unknown setting `{WRONG_STEP}`; its settings are now {}",
            s.document()["stages"]["alignment"]["settings"]["rolling_circle"]
        ),
        Err(e) => assert!(
            e.to_string().contains(WRONG_STEP),
            "the error does not name `{WRONG_STEP}`: {e}"
        ),
    }
    assert_eq!(s.to_text(), before, "a refused run changed the document");
}

#[test]
fn a_re_route_setting_the_stage_does_not_know_is_refused_by_name() {
    let mut s = Session::open(&example()).unwrap();
    let before = s.to_text();
    // No terrain: a stage that checked its settings first never asks for it.
    let e = s
        .align(AlignMethod::Reroute, &json!({ WRONG_SWEEP: [40.0] }), None)
        .expect_err("the re-route cannot run")
        .to_string();
    assert!(
        e.contains(WRONG_SWEEP),
        "the error does not name `{WRONG_SWEEP}`: {e}"
    );
    assert_eq!(s.to_text(), before, "a refused run changed the document");
}

#[test]
fn a_setting_of_one_method_is_not_a_setting_of_the_other() {
    // `fairness_weight` belongs to the re-route. The rolling circle does not know it.
    let mut s = Session::open(&example()).unwrap();
    let run = s.align(
        AlignMethod::RollingCircle,
        &json!({"fairness_weight": 1.0}),
        None,
    );
    match run {
        Ok(()) => panic!("the rolling circle ran with the re-route setting `fairness_weight`"),
        Err(e) => assert!(
            e.to_string().contains("fairness_weight"),
            "the error does not name `fairness_weight`: {e}"
        ),
    }
}

#[test]
fn a_setting_the_document_holds_and_the_stage_does_not_know_is_refused_by_name() {
    let mut doc: Value = serde_json::from_str(&example()).unwrap();
    doc["stages"]["alignment"]["settings"]["rolling_circle"][WRONG_STEP] = json!(20.0);
    // The door may refuse the document, or the stage may refuse to run. Either names the setting.
    let said = match Session::open(&doc.to_string()) {
        Err(e) => e.to_string(),
        Ok(mut s) => match s.run_stage(Stage::Alignment) {
            Err(e) => e.to_string(),
            Ok(()) => panic!(
                "the document opened and the alignment stage ran with the unknown setting `{WRONG_STEP}`"
            ),
        },
    };
    assert!(
        said.contains(WRONG_STEP),
        "the error does not name `{WRONG_STEP}`: {said}"
    );
}
