//! `owd run-inputs` (spec 0017, ticket 11): the flags set the run's objective, limits and target
//! level; with no flag it prints them and changes nothing; a refusal exits 2 with its code.
//!
//! Label: property

use owd_format::design::{Role, Session};
use serde_json::{Value, json};
use std::process::Command;

fn owd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .unwrap()
}

fn doc(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("owd-run-inputs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let line: Vec<[f64; 2]> = (0..=10)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Run inputs", "2026-11-20");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{}}}"#,
            json!(line)
        ),
    )])
    .unwrap();
    let p = dir.join("d.owd");
    let text = s.to_text();
    std::fs::write(&p, &text).unwrap();
    (p, text)
}

#[test]
fn the_flags_set_the_run_inputs_and_no_flag_prints_them() {
    let (p, _) = doc("set");
    let path = p.to_str().unwrap();
    let o = owd(&[
        "run-inputs",
        path,
        "--objective",
        "minimise:total_cut",
        "--limit",
        "line_length>=500",
        "--limit",
        "cost<=1000000 USD",
        "--target-level",
        "concept",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(d["inputs"]["run"]["objective"]["measure"], "total_cut");
    assert_eq!(d["inputs"]["run"]["limits"].as_array().unwrap().len(), 2);
    let saved = std::fs::read_to_string(&p).unwrap();
    let o = owd(&["run-inputs", path, "--json"]);
    assert!(o.status.success());
    let out: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(out["run"], d["inputs"]["run"]);
    assert_eq!(out["target_level"], "concept");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), saved, "a read changed the file");
}

#[test]
fn an_unknown_measure_exits_2_with_its_code_and_changes_nothing() {
    let (p, text) = doc("bad");
    let o = owd(&["run-inputs", p.to_str().unwrap(), "--limit", "beauty>=1"]);
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("RUN_MEASURE_UNKNOWN"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
}
