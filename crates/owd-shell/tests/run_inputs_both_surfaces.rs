//! Run inputs (spec 0017, ticket 11) set through the command line (`owd run-inputs`) and through the
//! app shell (its run-inputs fields) give the same document, and the same refusal: both call
//! `Session::edit_run_inputs` (the one path rule of ADR 0015).
//!
//! Label: property

use owd_cli::run_inputs::{RunFlags, run_inputs};
use owd_format::design::{Role, Session, Stage};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn design() -> String {
    let line: Vec<[f64; 2]> = (0..=30)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0])
        .collect();
    let line = format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(line)
    );
    let mut s = Session::new("Run inputs", "2026-11-20");
    s.import(&[(Role::Centreline, &line)]).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.to_text()
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The documents each surface leaves: (command line, shell). The app sends its limits as one text,
/// a limit a line.
fn both(
    name: &str,
    objective: &str,
    limits: &[&str],
    level: &str,
) -> (Result<Value, String>, Result<Value, String>) {
    let dir = Dir(std::env::temp_dir().join(format!("owd-run-in-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    std::fs::write(&doc, &text).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    let from_shell = shell
        .set_run_inputs(objective, &limits.join("\n"), level)
        .map(|()| serde_json::from_str(&shell.save_text_for(&doc)).unwrap());

    let flags = RunFlags {
        objective: Some(objective.to_string()),
        limits: limits.iter().map(|l| l.to_string()).collect(),
        no_limits: limits.is_empty(),
        target_level: Some(level.to_string()),
    };
    let from_cli = run_inputs(&doc.to_string_lossy(), &flags, &None, true)
        .map(|()| serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap())
        .map_err(|f| f.message);
    (from_cli, from_shell)
}

#[test]
fn the_same_inputs_give_the_same_document_through_both_surfaces() {
    let (cli, shell) = both(
        "same",
        "minimise:cost",
        &["total_cut<=500000", "cost<=2000000 USD"],
        "pre-feasibility",
    );
    let (cli, shell) = (cli.unwrap(), shell.unwrap());
    assert_eq!(cli, shell);
    assert_eq!(cli["inputs"]["run"]["objective"]["measure"], "cost");
    assert_eq!(cli["inputs"]["run"]["target_level"], "pre_feasibility");
    assert_eq!(cli["inputs"]["run"]["limits"][1]["currency"], "USD");
}

#[test]
fn an_unknown_measure_is_refused_the_same_way_by_both() {
    let (cli, shell) = both("unknown", "minimise:beauty", &[], "concept");
    let (cli, shell) = (cli.unwrap_err(), shell.unwrap_err());
    assert!(cli.contains("RUN_MEASURE_UNKNOWN"), "{cli}");
    assert_eq!(cli, shell);
}
