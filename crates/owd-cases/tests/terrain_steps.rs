//! The case runner's terrain steps: `import` with terrain, `water`, `volumes` and `run`.
//!
//! These tests drive the built binary on the synthetic twin in
//! `tests/alignment_cases/terrain-flat`: a straight river over flat terrain, under a level water
//! surface. Its expected volume is worked by hand in the case's `SOURCE.md`.
//!
//! Label: property

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Section area 72.8 m2 times 3,200 m of river (`SOURCE.md`, "The volume").
const DREDGE_M3: f64 = 232_960.0;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p =
            std::env::temp_dir().join(format!("owd-cases-terrain-{tag}-{}", std::process::id()));
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

/// Every file under `dir` with its bytes, in name order.
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            let bytes = std::fs::read(&path).unwrap();
            out.push((path, bytes));
        }
    }
    out.sort();
    out
}

/// A case store in `dir` that holds the twin, with `change` applied to its `case.json`.
fn store(dir: &Dir, change: impl FnOnce(&mut Value)) -> PathBuf {
    let cases = dir.0.join("cases");
    copy_dir(
        &repo().join("tests/alignment_cases/terrain-flat"),
        &cases.join("terrain-flat"),
    );
    let file = cases.join("terrain-flat/case.json");
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

fn steps(case: &mut Value) -> &mut Vec<Value> {
    case["steps"].as_array_mut().unwrap()
}

#[test]
fn the_terrain_twin_passes_on_both_surfaces_and_the_surfaces_agree() {
    let dir = Dir::new("pass");
    let cases = store(&dir, |_| {});
    let before = files(&cases);
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = report.expect("a report is written");
    assert_eq!(report["cases"][0]["id"], "terrain-flat");

    // The volume, worked by hand: both surfaces are inside the 2 % band.
    let dredge = row(&report, "dredge_m3");
    assert_eq!(dredge["class"], "outcome");
    assert_eq!(dredge["verdict"], "pass");
    assert_eq!(dredge["expected"], DREDGE_M3);
    for surface in ["cli", "shell"] {
        let got = dredge[surface].as_f64().unwrap();
        assert!(
            (got - DREDGE_M3).abs() <= 0.02 * DREDGE_M3,
            "{surface}: {got}"
        );
    }
    assert!(dredge["gap"]["cli"].is_number() && dredge["gap"]["shell"].is_number());
    // All of the cut is inside the river, and below the water.
    for name in ["dry_cut_m3", "tallest_face_m"] {
        let value = row(&report, name);
        assert_eq!(value["verdict"], "pass", "{name}");
        assert!(value["cli"].as_f64().unwrap().abs() <= 1.0, "{name}");
        assert!(value["shell"].as_f64().unwrap().abs() <= 1.0, "{name}");
    }
    assert_eq!(row(&report, "design_depth_m")["shell"], 3.25);

    // The one-path comparison: the two documents are equal. They name the terrain by the same
    // path, so the runner gave both surfaces the same file in the same place.
    assert_eq!(report["cases"][0]["one_path"]["equal"], true);
    assert_eq!(
        report["totals"],
        json!({"pass": 5, "fail": 0, "not_yet_possible": 0, "one_path_failures": 0})
    );
    // The runner writes nothing into the case folder.
    assert!(files(&cases) == before, "the case store changed");
}

#[test]
fn a_wrong_expected_volume_fails_the_run() {
    let dir = Dir::new("fail");
    let cases = store(&dir, |case| {
        // A section of 80 m2, not 72.8 m2: 10 % too much, outside the 2 % band.
        expect_row(case, "dredge_m3")["value"] = json!(256_000.0);
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(1));
    let report = report.expect("a failing run still writes its report");
    assert_eq!(row(&report, "dredge_m3")["verdict"], "fail");
    assert_eq!(row(&report, "dry_cut_m3")["verdict"], "pass");
    assert_eq!(report["totals"]["fail"], 1);
}

#[test]
fn the_run_step_reruns_what_a_new_water_level_made_stale() {
    // The water rises by 1 m, so the channel bottom rises by 1 m and the cut is 1.00 m deep:
    // 30.4 x 1.00 + 3 x 1.00^2 = 33.4 m2, times 3,200 m. `run` runs the stale volumes stage again.
    let shallower = 33.4 * 3200.0;
    let dir = Dir::new("run");
    let cases = store(&dir, |case| {
        steps(case).push(json!({"water": {"level_m": 11.0}}));
        steps(case).push(json!({"run": {}}));
        expect_row(case, "dredge_m3")["value"] = json!(shallower);
    });
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = report.unwrap();
    let dredge = row(&report, "dredge_m3");
    assert_eq!(dredge["verdict"], "pass");
    for surface in ["cli", "shell"] {
        let got = dredge[surface].as_f64().unwrap();
        assert!(
            (got - shallower).abs() <= 0.02 * shallower,
            "{surface}: {got}"
        );
    }
    assert_eq!(report["cases"][0]["one_path"]["equal"], true);

    // Without the `run` step the volumes are the old ones: the same expected value fails.
    let dir = Dir::new("no-run");
    let cases = store(&dir, |case| {
        steps(case).push(json!({"water": {"level_m": 11.0}}));
        expect_row(case, "dredge_m3")["value"] = json!(shallower);
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(row(&report.unwrap(), "dredge_m3")["verdict"], "fail");
}

#[test]
fn the_water_step_takes_a_fall_per_metre() {
    // 10.00 m at the river's start, falling 0.0005 per metre over 3,200 m: 8.40 m at the end. The
    // river centreline may stop short of the end by a step of 20 m, which is 0.01 m of level.
    let dir = Dir::new("fall");
    let cases = store(&dir, |case| {
        *steps(case) = vec![
            steps(case)[0].clone(),
            steps(case)[1].clone(),
            json!({"water": {"level_m": 10.0, "fall_per_m": 0.0005}}),
        ];
        case["expect"] = json!([
            {
                "name": "water_level_points",
                "read": {"pointer": "/inputs/water_level/points", "measure": "count"},
                "value": 2,
                "class": "exact",
                "tolerance": {"absolute": 0},
                "source": "SOURCE.md#inputs"
            },
            {
                "name": "water_level_at_the_end_m",
                "read": {"pointer": "/inputs/water_level/points/1/1"},
                "value": 8.4,
                "class": "outcome",
                "tolerance": {"absolute": 0.011},
                "source": "SOURCE.md#inputs"
            }
        ]);
    });
    let (out, report) = run(&dir, &cases);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = report.unwrap();
    assert_eq!(row(&report, "water_level_points")["cli"], 2.0);
    assert_eq!(row(&report, "water_level_points")["shell"], 2.0);
    assert_eq!(row(&report, "water_level_at_the_end_m")["verdict"], "pass");
    assert_eq!(report["cases"][0]["one_path"]["equal"], true);
}

#[test]
fn a_step_the_runner_does_not_know_is_refused() {
    let dir = Dir::new("unknown");
    let cases = store(&dir, |case| {
        steps(case).push(json!({"dredge": {}}));
    });
    let (out, report) = run(&dir, &cases);
    assert_eq!(out.status.code(), Some(2));
    assert!(report.is_none(), "a refused case store writes no report");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("terrain-flat") && err.contains("dredge"),
        "{err}"
    );
}
