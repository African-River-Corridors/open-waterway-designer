//! `use_water` set through the command line (`owd align --use-water`) and through the app shell
//! (its alignment settings, then a run of the stage) gives the same document (ticket #488).
//!
//! The river is two closed-form sine banks (see `owd-format/tests/use_water_default.rs`): the
//! import builds the polygon, so `true` and `false` give different lines.
//!
//! Label: property

use owd_cli::commands::{AlignFlags, align};
use owd_format::design::{Role, Session, Stage};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn sine(off: f64) -> Vec<Pt> {
    (0..=600)
        .map(|i| {
            let x = 10.0 * f64::from(i);
            let y = 600.0 * (2.0 * std::f64::consts::PI * x / 1200.0).sin() + off;
            [500_000.0 + x, 600_000.0 + y]
        })
        .collect()
}

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn design() -> String {
    let (left, right) = (line_text(&sine(75.0)), line_text(&sine(-75.0)));
    let mut s = Session::new("Use water", "2026-11-10");
    s.import(&[(Role::LeftBank, &left), (Role::RightBank, &right)])
        .unwrap();
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

/// The documents each surface leaves with `use_water` set to `on`: (command line, shell).
fn both(on: bool) -> (Value, Value) {
    let dir = Dir(std::env::temp_dir().join(format!("owd-use-water-{on}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    std::fs::write(&doc, &text).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    shell
        .set_alignment_settings("rolling_circle", &json!({"use_water": on}))
        .unwrap();
    let (job, _) = shell
        .begin_run("alignment", Some("rolling_circle"))
        .unwrap();
    let outcome = job.execute(&|_| {});
    assert_eq!(shell.finish_run(outcome), Ok("done"));
    let shell_doc: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    let flags = AlignFlags {
        use_water: Some(on.to_string()),
        ..Default::default()
    };
    // A failed check is exit 1 with the document written; the comparison below catches the rest.
    let _ = align(&doc.to_string_lossy(), "rolling-circle", flags, &None, true);
    let cli_doc: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    (cli_doc, shell_doc)
}

#[test]
fn false_gives_the_same_document_through_both_surfaces() {
    let (cli, shell) = both(false);
    assert_eq!(
        shell["stages"]["alignment"]["settings"]["rolling_circle"]["use_water"],
        json!(false)
    );
    assert_eq!(cli, shell);
}

#[test]
fn true_gives_the_same_document_through_both_surfaces() {
    let (cli, shell) = both(true);
    assert_eq!(
        shell["stages"]["alignment"]["settings"]["rolling_circle"]["use_water"],
        json!(true)
    );
    assert_eq!(cli, shell);
    assert_ne!(
        cli["options"][0]["line"],
        both(false).0["options"][0]["line"]
    );
}
