//! The alignment case runner (`owd-cases`).
//!
//! A case is a folder with a `case.json`: the steps to run and the values to expect. The runner
//! plays the steps through the command line's functions and through the app shell, reads each
//! expected value from both documents, and writes one report. These tests drive the built binary
//! on the synthetic twin in `tests/alignment_cases/line-straight`.
//!
//! Label: property

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-cases-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// A case store in `dir` that holds the twin, with `change` applied to its `case.json`.
fn store(dir: &Dir, change: impl FnOnce(&mut Value)) -> PathBuf {
    let cases = dir.0.join("cases");
    copy_dir(
        &repo().join("tests/alignment_cases/line-straight"),
        &cases.join("line-straight"),
    );
    let file = cases.join("line-straight/case.json");
    let mut case: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    change(&mut case);
    std::fs::write(&file, serde_json::to_string_pretty(&case).unwrap()).unwrap();
    cases
}

/// Run `owd-cases run --cases <cases> --report <dir>/report.json`.
fn run(dir: &Dir, cases: &Path) -> (Output, Option<Value>) {
    let report = dir.0.join("report.json");
    let out = Command::new(env!("CARGO_BIN_EXE_owd-cases"))
        .arg("run")
        .arg("--cases")
        .arg(cases)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    let value = std::fs::read_to_string(&report)
        .ok()
        .map(|t| serde_json::from_str(&t).unwrap());
    (out, value)
}

fn row<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["cases"][0]["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("the report has no value `{name}`"))
}

fn expect_row<'a>(case: &'a mut Value, name: &str) -> &'a mut Value {
    case["expect"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|v| v["name"] == name)
        .unwrap()
}

#[test]
fn the_twin_passes_on_both_surfaces_and_the_surfaces_agree() {
    let dir = Dir::new("pass");
    let cases = store(&dir, |_| {});
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = report.expect("a report is written");
    assert_eq!(report["cases"][0]["id"], "line-straight");

    let line = row(&report, "design_line_length_m");
    assert_eq!(line["class"], "outcome");
    assert_eq!(line["verdict"], "pass");
    assert_eq!(line["expected"], 3200.0);
    for surface in ["cli", "shell"] {
        let got = line[surface].as_f64().unwrap();
        assert!((got - 3200.0).abs() <= 32.0, "{surface}: {got}");
    }
    assert!(line["gap"]["cli"].is_number() && line["gap"]["shell"].is_number());
    assert_eq!(row(&report, "min_bend_radius_m")["cli"], 255.0);
    assert_eq!(row(&report, "left_bank_positions")["shell"], 33.0);

    // The one-path comparison: the two documents are equal.
    assert_eq!(report["cases"][0]["one_path"]["equal"], true);
    assert_eq!(
        report["totals"],
        json!({"pass": 4, "fail": 0, "not_yet_possible": 0, "one_path_failures": 0})
    );
}

#[test]
fn a_value_outside_its_tolerance_fails_the_run() {
    let dir = Dir::new("fail");
    let cases = store(&dir, |case| {
        expect_row(case, "design_line_length_m")["value"] = json!(4000.0);
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(1));
    let report = report.expect("a failing run still writes its report");
    assert_eq!(row(&report, "design_line_length_m")["verdict"], "fail");
    assert_eq!(row(&report, "min_bend_radius_m")["verdict"], "pass");
    assert_eq!(report["totals"]["fail"], 1);
}

#[test]
fn a_value_the_tool_cannot_give_yet_is_counted_and_does_not_fail_the_run() {
    let dir = Dir::new("gap");
    let cases = store(&dir, |case| {
        case["expect"].as_array_mut().unwrap().push(json!({
            "name": "plan_cut_area_m2",
            "read": {"pointer": "/stages/plan_cut/results/area_m2"},
            "value": 0.0,
            "class": "not_yet_possible",
            "source": "SOURCE.md#design-line"
        }));
    });
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = report.unwrap();
    assert_eq!(
        row(&report, "plan_cut_area_m2")["verdict"],
        "not_yet_possible"
    );
    assert_eq!(report["totals"]["not_yet_possible"], 1);
    assert_eq!(report["totals"]["fail"], 0);
}

#[test]
fn a_known_defect_passes_when_the_rule_is_met_not_when_the_old_value_is_matched() {
    // The reference line read 240 m where the rule asks for 255 m. The tool must meet the rule.
    let dir = Dir::new("defect");
    let cases = store(&dir, |case| {
        case["expect"].as_array_mut().unwrap().push(json!({
            "name": "required_radius_is_met",
            "read": {"pointer": "/options/0/checks/r_min_m"},
            "value": 240.0,
            "class": "known_defect",
            "rule": {"at_least": 255.0},
            "source": "SOURCE.md#required-radius"
        }));
    });
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        row(&report.unwrap(), "required_radius_is_met")["verdict"],
        "pass"
    );

    let dir = Dir::new("defect-unmet");
    let cases = store(&dir, |case| {
        case["expect"].as_array_mut().unwrap().push(json!({
            "name": "required_radius_is_met",
            "read": {"pointer": "/options/0/checks/r_min_m"},
            "value": 240.0,
            "class": "known_defect",
            "rule": {"at_least": 300.0},
            "source": "SOURCE.md#required-radius"
        }));
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        row(&report.unwrap(), "required_radius_is_met")["verdict"],
        "fail"
    );
}

#[test]
fn an_expected_value_with_no_source_is_refused() {
    let dir = Dir::new("no-source");
    let cases = store(&dir, |case| {
        expect_row(case, "design_line_length_m")
            .as_object_mut()
            .unwrap()
            .remove("source");
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(2));
    assert!(report.is_none(), "a refused case store writes no report");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("design_line_length_m") && err.contains("source"),
        "{err}"
    );
}

#[test]
fn the_repository_twins_run_when_no_case_store_is_named() {
    let dir = Dir::new("default");
    let report = dir.0.join("report.json");
    let out = Command::new(env!("CARGO_BIN_EXE_owd-cases"))
        .current_dir(repo())
        .env_remove("OWD_ALIGNMENT_CASES")
        .arg("run")
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert!(
        report["cases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "line-straight")
    );
}
