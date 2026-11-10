//! The rolling circle on a river of reverse bends (issue #498) gives the same document through the
//! command line (`owd align rolling-circle`) and through the app shell (a run of the alignment
//! stage), and on both the rolled line holds R_min on the default chord.
//!
//! The river is a chain of circular arcs that turn left and right in turn, each tighter than the
//! vessel's R_min, with banks 40 m either side. The water-aware mode is off, so the line is the
//! plain roll of the centreline the import builds.
//!
//! Label: property

use owd_cli::commands::{AlignFlags, align};
use owd_format::design::{Role, Session, Stage};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// The chain's centreline in a local frame: a straight, `n` arcs of radius `r` and `sweep_deg`
/// turning left and right in turn (half sweeps at the ends), a straight; a vertex every 10 m.
fn chain(r: f64, sweep_deg: f64, n: usize) -> Vec<(Pt, f64)> {
    let ds = 10.0;
    let mut p: Pt = [0.0, 0.0];
    let mut h: f64 = 0.0;
    let mut out = vec![(p, h)];
    let walk = |out: &mut Vec<(Pt, f64)>, p: &mut Pt, h: f64| {
        *p = [p[0] + ds * h.cos(), p[1] + ds * h.sin()];
        out.push((*p, h));
    };
    for _ in 0..30 {
        walk(&mut out, &mut p, h);
    }
    for k in 0..n {
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        let part = if k == 0 || k == n - 1 { 0.5 } else { 1.0 };
        let total = sweep_deg.to_radians() * part;
        let steps = (total * r / ds).round() as usize;
        for _ in 0..steps {
            h += sign * total / steps as f64;
            walk(&mut out, &mut p, h);
        }
    }
    for _ in 0..30 {
        walk(&mut out, &mut p, h);
    }
    out
}

/// A bank `off` metres to the left of the centreline (negative: to the right), in UTM.
fn bank(off: f64) -> Vec<Pt> {
    chain(R_BEND, 120.0, 6)
        .iter()
        .map(|&([x, y], h)| [500_000.0 + x - off * h.sin(), 600_000.0 + y + off * h.cos()])
        .collect()
}

/// The radius of the chain's arcs (m): tighter than the vessel's R_min.
const R_BEND: f64 = 150.0;

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn design() -> String {
    let (left, right) = (line_text(&bank(40.0)), line_text(&bank(-40.0)));
    let mut s = Session::new("Reverse bends", "2026-11-10");
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

#[test]
fn the_rolled_reverse_bends_are_the_same_through_both_surfaces_and_hold_r_min() {
    let dir = Dir(std::env::temp_dir().join(format!("owd-reverse-bends-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    std::fs::write(&doc, &text).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    shell
        .set_alignment_settings("rolling_circle", &json!({"use_water": false}))
        .unwrap();
    let (job, _) = shell
        .begin_run("alignment", Some("rolling_circle"))
        .unwrap();
    let outcome = job.execute(&|_| {});
    assert_eq!(shell.finish_run(outcome), Ok("done"));
    let shell_doc: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    let flags = AlignFlags {
        use_water: Some("false".into()),
        ..Default::default()
    };
    let _ = align(&doc.to_string_lossy(), "rolling-circle", flags, &None, true);
    let cli_doc: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();

    assert_eq!(cli_doc, shell_doc);
    let option = &cli_doc["options"][0];
    assert_eq!(option["method"], json!("rolling_circle"), "{option}");
    let checks = &option["checks"];
    let r_min = checks["r_min_m"].as_f64().unwrap();
    assert!(
        R_BEND < r_min,
        "the bends must be tighter than R_min {r_min}"
    );
    assert_eq!(checks["radius_ok"], json!(true), "{checks}");
}
