//! The case runner measures a case whose option fails a check, sizes for a custom vessel, and
//! keeps its stdout to its own table.
//!
//! An option that fails a check is still a result: both surfaces save it, marked failed. The
//! runner reads it from both documents and lists the failed checks in the report. Only a step
//! that cannot run is an error. These tests drive the built binary on the two synthetic twins in
//! `tests/alignment_cases`: `line-tight-bend` (a half turn too tight for the vessel) and
//! `line-straight`.
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
        let p = std::env::temp_dir().join(format!("owd-cases-fix-{tag}-{}", std::process::id()));
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

/// A case store in `dir` that holds the repository's case `id`, with `change` applied to its
/// `case.json`.
fn store(dir: &Dir, id: &str, change: impl FnOnce(&mut Value)) -> PathBuf {
    let cases = dir.0.join("cases");
    copy_dir(
        &repo().join("tests/alignment_cases").join(id),
        &cases.join(id),
    );
    let file = cases.join(id).join("case.json");
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

/// What the run printed, for a failed assertion.
fn said(out: &Output) -> String {
    format!(
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The case's `size` step.
fn size_step(case: &mut Value) -> &mut Value {
    case["steps"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s.get("size").is_some())
        .unwrap()
}

/// A class's own figures as a custom vessel: an 85 × 9.5 × 2.5 m motor vessel.
fn class_figures() -> Value {
    json!({"name": "Class figures", "loa_m": 85.0, "beam_m": 9.5, "draught_m": 2.5,
           "formation": "motor"})
}

/// The failed checks one surface lists for the case: one entry for each `align` step whose option
/// fails a check.
fn failed<'a>(report: &'a Value, surface: &str) -> &'a Vec<Value> {
    report["cases"][0]["failed_checks"][surface]
        .as_array()
        .unwrap_or_else(|| panic!("the case has no `failed_checks.{surface}` list: {report:#}"))
}

#[test]
fn an_option_that_fails_its_check_is_measured_on_both_surfaces() {
    let dir = Dir::new("tight");
    let cases = store(&dir, "line-tight-bend", |_| {});
    let (out, report) = run(&dir, &cases);
    let report = report.expect("a report is written");
    let case = &report["cases"][0];
    assert_eq!(case["id"], "line-tight-bend");

    // The command line saved the document before it reported the failed check: it is not lost.
    assert!(
        case.get("errors").is_none(),
        "a failed check is not an error: {}\n{}",
        case["errors"],
        said(&out)
    );
    assert_eq!(row(&report, "min_bend_radius_m")["cli"], 255.0);
    assert_eq!(row(&report, "min_bend_radius_m")["shell"], 255.0);

    // A boolean is read as 1 (true) or 0 (false).
    let ok = row(&report, "radius_ok");
    assert_eq!(ok["class"], "exact");
    assert_eq!(ok["expected"], 0.0);
    assert_eq!(ok["cli"], 0.0, "{ok}");
    assert_eq!(ok["shell"], 0.0, "{ok}");
    assert_eq!(ok["verdict"], "pass");

    // The report lists the failed checks, for each surface, by step.
    for surface in ["cli", "shell"] {
        let list = failed(&report, surface);
        assert_eq!(list.len(), 1, "{surface}: {list:?}");
        assert_eq!(list[0]["step"], 4, "{surface}");
        assert_eq!(list[0]["method"], "rolling_circle", "{surface}");
        let checks = list[0]["checks"].as_array().unwrap();
        assert!(checks.contains(&json!("R_min")), "{surface}: {checks:?}");
    }
    assert_eq!(case["failed_checks"]["cli"], case["failed_checks"]["shell"]);

    // Both documents are there and equal, and a failed check does not fail the run.
    assert_eq!(case["one_path"]["equal"], true, "{}", case["one_path"]);
    assert_eq!(
        report["totals"],
        json!({"pass": 4, "fail": 0, "not_yet_possible": 0, "one_path_failures": 0})
    );
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
}

#[test]
fn the_steps_after_a_failed_check_still_run() {
    let dir = Dir::new("later");
    let cases = store(&dir, "line-tight-bend", |case| {
        // After the option that fails: size again, for a deeper draught.
        let mut again = size_step(case).clone();
        again["size"]["draught_m"] = json!(3.0);
        case["steps"].as_array_mut().unwrap().push(again);
        case["expect"].as_array_mut().unwrap().push(json!({
            "name": "draught_m",
            "read": {"pointer": "/stages/sizing/settings/draught_m"},
            "value": 3.0,
            "class": "exact",
            "tolerance": {"absolute": 0},
            "source": "SOURCE.md#required-radius"
        }));
    });
    let (out, report) = run(&dir, &cases);
    let report = report.expect("a report is written");
    let draught = row(&report, "draught_m");
    assert_eq!(draught["cli"], 3.0, "{draught}\n{}", said(&out));
    assert_eq!(draught["shell"], 3.0, "{draught}");
    assert_eq!(draught["verdict"], "pass");
    assert_eq!(failed(&report, "cli").len(), 1);
    assert_eq!(report["cases"][0]["one_path"]["equal"], true);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
}

#[test]
fn a_step_that_cannot_run_is_an_error_and_not_a_failed_check() {
    let dir = Dir::new("cannot");
    let cases = store(&dir, "line-tight-bend", |case| {
        case["steps"][1]["import"]["left_bank"] = json!("inputs/no-such-file.geojson");
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(1), "{}", said(&out));
    let report = report.expect("a failing run still writes its report");
    let case = &report["cases"][0];
    let errors = case["errors"].as_array().expect("the case has errors");
    assert_eq!(errors.len(), 2, "one for each surface: {errors:?}");
    assert!(
        errors
            .iter()
            .all(|e| e.as_str().unwrap().contains("step 2")),
        "{errors:?}"
    );
    // Nothing ran as far as a check: the lists are there, and empty.
    assert_eq!(case["failed_checks"], json!({"cli": [], "shell": []}));
    assert_eq!(case["one_path"]["equal"], false);
}

#[test]
fn a_custom_vessel_with_a_classs_figures_sizes_as_the_class_on_both_surfaces() {
    let dir = Dir::new("custom");
    let cases = store(&dir, "line-straight", |case| {
        let size = size_step(case)["size"].as_object_mut().unwrap();
        size.remove("vessel");
        size.remove("draught_m");
        size.insert("custom_vessel".into(), class_figures());
        case["expect"].as_array_mut().unwrap().push(json!({
            "name": "custom_vessel_loa_m",
            "read": {"pointer": "/stages/sizing/settings/custom_vessel/loa_m"},
            "value": 85.0,
            "class": "exact",
            "tolerance": {"absolute": 0},
            "source": "SOURCE.md#required-radius"
        }));
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(0), "{}", said(&out));
    let report = report.expect("a report is written");
    // Three vessel lengths, as for the class: 3 × 85 = 255 m.
    let radius = row(&report, "min_bend_radius_m");
    assert_eq!(radius["cli"], 255.0, "{radius}");
    assert_eq!(radius["shell"], 255.0, "{radius}");
    assert_eq!(radius["verdict"], "pass");
    let loa = row(&report, "custom_vessel_loa_m");
    assert_eq!(loa["cli"], 85.0, "{loa}");
    assert_eq!(loa["shell"], 85.0, "{loa}");
    let case = &report["cases"][0];
    assert_eq!(case["one_path"]["equal"], true, "{}", case["one_path"]);
    // The straight line passes its checks.
    assert_eq!(case["failed_checks"], json!({"cli": [], "shell": []}));
    assert_eq!(
        report["totals"],
        json!({"pass": 5, "fail": 0, "not_yet_possible": 0, "one_path_failures": 0})
    );
}

#[test]
fn a_class_and_a_custom_vessel_together_are_refused() {
    let dir = Dir::new("both");
    let cases = store(&dir, "line-straight", |case| {
        size_step(case)["size"]["custom_vessel"] = class_figures();
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(2), "{}", said(&out));
    assert!(report.is_none(), "a refused case store writes no report");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("line-straight") && err.contains("custom_vessel") && err.contains("vessel"),
        "{err}"
    );
}

#[test]
fn stdout_holds_the_runners_table_and_nothing_from_the_commands() {
    let dir = Dir::new("quiet");
    let cases = store(&dir, "line-straight", |_| {});
    let (out, _) = run(&dir, &cases);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("case line-straight"), "{stdout}");
    assert!(stdout.contains("report: "), "{stdout}");
    for chatter in [
        "\"ok\":true",
        "project CRS",
        "created ",
        "bottom width",
        "Rolling circle",
    ] {
        assert!(
            !stdout.contains(chatter),
            "stdout holds a command's own line (`{chatter}`):\n{stdout}"
        );
    }
    // Every line is the runner's: a case heading, an indented table line, or the report's path.
    for line in stdout.lines() {
        assert!(
            line.starts_with("case ") || line.starts_with("  ") || line.starts_with("report: "),
            "not a line of the runner's table: `{line}`"
        );
    }
}
