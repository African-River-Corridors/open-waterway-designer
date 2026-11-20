//! Stage storage and schema generation (spec 0017, ticket 2): the record every stage is stored in,
//! and the check that the schemas in `schema/stages/` are the ones the Rust types generate.
//!
//! Label: property
//!
//! To regenerate the checked-in files after a change to a stage type (and a bump of its version),
//! run `OWD_WRITE_STAGE_SCHEMAS=1 cargo test -p owd-format --test stage_schemas`.

use owd_format::design::{Session, Stage};
use owd_format::stage::{
    STAGE_CONTRACT_VERSION, StageContract, StageRecord, check_dir, contracts, file_name, write_dir,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn repo_schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/stages")
}

/// An empty scratch folder, named for the test and the process.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owd-stage-schemas-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct OutV1 {
    width_m: f64,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct OutV1Plus {
    width_m: f64,
    depth_m: f64,
}

fn contract_of<T: JsonSchema>(stage: &'static str, version: u32) -> StageContract {
    StageContract::of::<StageRecord<Value, T>>(stage, version)
}

#[test]
fn the_checked_in_schemas_are_the_ones_the_types_generate() {
    let dir = repo_schemas();
    if std::env::var_os("OWD_WRITE_STAGE_SCHEMAS").is_some() {
        write_dir(&dir, &contracts()).unwrap();
    }
    let drift = check_dir(&dir, &contracts());
    assert!(
        drift.is_empty(),
        "schema/stages is out of step with the stage types:\n{}\nregenerate with \
         `OWD_WRITE_STAGE_SCHEMAS=1 cargo test -p owd-format --test stage_schemas` and raise the \
         stage's version if what its output means changed",
        drift.join("\n")
    );
}

#[test]
fn a_written_folder_checks_clean() {
    let dir = scratch("clean");
    let cs = [contract_of::<OutV1>("probe", 1)];
    write_dir(&dir, &cs).unwrap();
    assert!(dir.join("probe.1.schema.json").is_file());
    assert_eq!(check_dir(&dir, &cs), Vec::<String>::new());
}

#[test]
fn a_changed_type_fails_the_check() {
    let dir = scratch("changed-type");
    write_dir(&dir, &[contract_of::<OutV1>("probe", 1)]).unwrap();
    // The type gains a field and keeps its version: the file on disk is now wrong.
    let drift = check_dir(&dir, &[contract_of::<OutV1Plus>("probe", 1)]);
    assert_eq!(drift.len(), 1, "{drift:?}");
    assert!(drift[0].contains("probe.1.schema.json") && drift[0].contains("differs"));
}

#[test]
fn a_new_version_needs_its_own_file() {
    let dir = scratch("new-version");
    write_dir(&dir, &[contract_of::<OutV1>("probe", 1)]).unwrap();
    let drift = check_dir(&dir, &[contract_of::<OutV1Plus>("probe", 2)]);
    // The version-2 file is missing, and the version-1 file no longer has a contract.
    assert!(
        drift
            .iter()
            .any(|d| d.contains("probe.2.schema.json") && d.contains("missing"))
    );
    assert!(
        drift
            .iter()
            .any(|d| d.contains("probe.1.schema.json") && d.contains("no contract"))
    );
}

#[test]
fn a_hand_edited_file_fails_the_check() {
    let dir = scratch("hand-edit");
    let cs = [contract_of::<OutV1>("probe", 1)];
    write_dir(&dir, &cs).unwrap();
    let path = dir.join("probe.1.schema.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("width_m", "width")).unwrap();
    assert_eq!(check_dir(&dir, &cs).len(), 1);
}

#[test]
fn a_missing_file_fails_the_check() {
    let dir = scratch("missing");
    let cs = [contract_of::<OutV1>("probe", 1)];
    assert!(check_dir(&dir, &cs)[0].contains("missing"));
}

#[test]
fn a_file_with_no_contract_fails_the_check() {
    let dir = scratch("extra");
    write_dir(&dir, &[contract_of::<OutV1>("probe", 1)]).unwrap();
    std::fs::write(dir.join("ghost.1.schema.json"), "{}\n").unwrap();
    let drift = check_dir(&dir, &[contract_of::<OutV1>("probe", 1)]);
    assert_eq!(drift.len(), 1, "{drift:?}");
    assert!(drift[0].contains("ghost.1.schema.json") && drift[0].contains("no contract"));
}

#[test]
fn a_file_that_is_not_a_schema_is_left_alone() {
    let dir = scratch("other-file");
    let cs = [contract_of::<OutV1>("probe", 1)];
    write_dir(&dir, &cs).unwrap();
    std::fs::write(dir.join("README.md"), "notes\n").unwrap();
    assert!(check_dir(&dir, &cs).is_empty());
}

#[test]
fn files_are_named_stage_dot_version() {
    assert_eq!(file_name("river", 3), "river.3.schema.json");
}

#[test]
fn every_contract_has_a_distinct_file() {
    let names: Vec<String> = contracts().iter().map(|c| c.file_name()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "{names:?}");
}

#[test]
fn a_stage_record_has_the_four_parts_and_round_trips() {
    let text = json!({
        "version": 1,
        "depends_on": "abc123",
        "settings": {"k": 1},
        "output": {"width_m": 12.5}
    });
    let r: StageRecord<Value, OutV1> = serde_json::from_value(text.clone()).unwrap();
    assert_eq!(r.version, 1);
    assert_eq!(r.output.as_ref().unwrap().width_m, 12.5);
    assert_eq!(serde_json::to_value(&r).unwrap(), text);
}

#[test]
fn a_stage_that_has_not_run_has_no_output_and_no_hash() {
    let r: StageRecord<Value, OutV1> = serde_json::from_value(json!({
        "version": 1, "depends_on": null, "settings": null, "output": null
    }))
    .unwrap();
    assert!(r.output.is_none() && r.depends_on.is_none());
}

#[test]
fn a_record_with_a_wrong_output_is_refused() {
    let bad =
        json!({"version": 1, "depends_on": null, "settings": null, "output": {"width_m": "wide"}});
    assert!(serde_json::from_value::<StageRecord<Value, OutV1>>(bad).is_err());
}

#[test]
fn the_session_reads_each_stage_as_a_record() {
    let mut s = Session::new("t", "2026-01-01");
    let fresh = s.stage_record(Stage::Alignment);
    assert_eq!(fresh.version, STAGE_CONTRACT_VERSION);
    assert!(fresh.output.is_none() && fresh.depends_on.is_none());

    s.set_vessel("CEMT_Va", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let sized = s.stage_record(Stage::Sizing);
    assert!(
        sized.output.is_some(),
        "the sizing result is the record's output"
    );
    assert!(sized.depends_on.is_some());
    assert_eq!(
        sized.output.as_ref(),
        Some(&s.document()["stages"]["sizing"]["results"])
    );
    assert_eq!(sized.settings, s.document()["stages"]["sizing"]["settings"]);
}
