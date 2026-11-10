//! `owd plan-cut`: the command exists, runs with no terrain, and says the result is plan only.
//!
//! Label: property

use std::path::{Path, PathBuf};
use std::process::Command;

fn owd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

/// A copy of the web example with its terrain and its water level taken away.
fn no_terrain(tag: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(src).unwrap()).unwrap();
    doc["inputs"]["dem"] = serde_json::Value::Null;
    doc["inputs"]["water_level"] = serde_json::Value::Null;
    let dir = std::env::temp_dir().join(format!("owd-plan-cut-cmd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.owd");
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    path
}

#[test]
fn plan_cut_runs_with_no_terrain_and_prints_a_plan_only_result() {
    let path = no_terrain("run");
    let out = owd(&["plan-cut", path.to_str().unwrap(), "--json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "owd plan-cut failed: {stderr}");

    // The printed result is the stored one.
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let stored = &doc["stages"]["plan_cut"]["results"];
    assert_eq!(&printed, stored);
    assert!(
        stored["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("plan only")
    );
    let options = stored["options"].as_array().unwrap();
    assert_eq!(options.len(), doc["options"].as_array().unwrap().len());
    for o in options {
        let (left, right) = (
            o["by_bank"]["left_m2"].as_f64().unwrap(),
            o["by_bank"]["right_m2"].as_f64().unwrap(),
        );
        let area = o["area_m2"].as_f64().unwrap();
        assert!((left + right - area).abs() <= 1e-6 * area.max(1.0));
    }

    // The table, for a person, says plan only too.
    let out = owd(&["plan-cut", path.to_str().unwrap()]);
    assert!(out.status.success());
    let table = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(table.contains("plan only"), "{table}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
