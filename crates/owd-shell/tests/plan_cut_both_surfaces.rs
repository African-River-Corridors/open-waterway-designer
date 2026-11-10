//! The plan cut gives the same document through the command line and through the app shell.
//!
//! The design is a straight synthetic river in UTM 30N with no terrain and no water level. The
//! expected area is closed-form: (W - w) x L, with W worked out here from the sizing in the
//! document: W = bottom width + 2 x side slope x design depth.
//!
//! Label: property

use owd_cli::plan_cut::{PlanCutFlags, plan_cut};
use owd_core::geom::Pt;
use owd_core::volumes::sections::DEFAULT_WET_SLOPE;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const L: f64 = 2000.0;

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![
        [X0 + x0, Y0 + y0],
        [X0 + x1, Y0 + y0],
        [X0 + x1, Y0 + y1],
        [X0 + x0, Y0 + y1],
        [X0 + x0, Y0 + y0],
    ]
}

/// The `.owd` text of a straight river `w` wide with the given islands: a centreline and a river
/// polygon, sized for CEMT IV, with one design line option on the centreline.
fn design(w: f64, islands: &[Vec<Pt>]) -> String {
    let centre: Vec<Pt> = (0..=40).map(|i| [X0 + 50.0 * i as f64, Y0]).collect();
    let mut rings = vec![rect(-100.0, L + 100.0, -w / 2.0, w / 2.0)];
    rings.extend(islands.iter().cloned());
    let mut s = Session::new("Straight river", "2026-11-10");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!(rings)
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    let d = s.document();
    assert!(d["inputs"]["dem"].is_null() && d["inputs"]["water_level"].is_null());
    let line = d["options"][0]["line"]["coordinates"].as_array().unwrap();
    assert!(line.iter().all(|p| p[1].as_f64() == Some(Y0)));
    assert_eq!(line[line.len() - 1][0].as_f64(), Some(X0 + L));
    s.to_text()
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-plan-cut-{tag}-{}", std::process::id()));
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

/// What each surface leaves for the same text and the same smallest piece.
struct Both {
    cli_doc: Value,
    shell_doc: Value,
    cli_geojson: String,
    shell_geojson: String,
}

fn both(tag: &str, text: &str, min_piece_m2: Option<f64>) -> Both {
    let dir = Dir::new(tag);
    let doc = dir.0.join("d.owd");
    std::fs::write(&doc, text).unwrap();
    let cut_file = dir.0.join("cut.geojson");

    // The app shell.
    let mut shell = AppState::open(&doc, text, None).unwrap();
    let settings = match min_piece_m2 {
        Some(m) => json!({"min_piece_m2": m}),
        None => json!({}),
    };
    shell
        .run_plan_cut(&settings)
        .expect("the shell runs the plan cut with no terrain");
    let shell_geojson = shell.plan_cut_geojson().unwrap();
    let shell_doc: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    // The command line, on the file: `owd plan-cut d.owd --geojson cut.geojson --wgs84 --json`.
    let flags = PlanCutFlags {
        min_piece_m2,
        geojson: Some(cut_file.to_string_lossy().into_owned()),
        wgs84: true,
    };
    if plan_cut(&doc.to_string_lossy(), flags, &None, true).is_err() {
        panic!("`owd plan-cut` failed on {tag}");
    }
    let cli_doc: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    Both {
        cli_doc,
        shell_doc,
        cli_geojson: std::fs::read_to_string(&cut_file).unwrap(),
        shell_geojson,
    }
}

fn row(doc: &Value) -> &Value {
    &doc["stages"]["plan_cut"]["results"]["options"][0]
}

fn close(got: &Value, want: f64) -> bool {
    let got = got.as_f64().unwrap_or(f64::NAN);
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

#[test]
fn a_narrow_river_gives_the_same_cut_through_the_shell_and_the_command_line() {
    let w = 30.0;
    let text = design(w, &[]);
    let b = both("narrow", &text, None);
    assert_eq!(
        b.shell_doc, b.cli_doc,
        "the two surfaces left different documents"
    );

    let sizing = &b.cli_doc["stages"]["sizing"]["results"];
    let wide = sizing["bottom_width_straight_m"].as_f64().unwrap()
        + 2.0 * DEFAULT_WET_SLOPE * sizing["design_depth_m"].as_f64().unwrap();
    let want = (wide - w) * L;
    let r = row(&b.cli_doc);
    assert!(close(&r["area_m2"], want), "{} != {want}", r["area_m2"]);
    assert!(close(&r["by_bank"]["left_m2"], want / 2.0));
    assert!(close(&r["by_bank"]["right_m2"], want / 2.0));
    assert_eq!(r["pieces"], 2);
    assert!(
        r["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("plan only")
    );

    // The cut polygons, in longitude and latitude, are the same text from both surfaces.
    assert_eq!(b.shell_geojson, b.cli_geojson);
    let fc: Value = serde_json::from_str(&b.cli_geojson).unwrap();
    assert_eq!(fc["features"].as_array().unwrap().len(), 2);
}

#[test]
fn the_smallest_piece_is_the_same_setting_on_both_surfaces() {
    // A 6 m by 6 m island in a wide river: 36 m2.
    let text = design(200.0, &[rect(400.0, 406.0, 5.0, 11.0)]);

    // The default of 50 m2 comes from one place, so both drop the island.
    let b = both("island-default", &text, None);
    assert_eq!(b.shell_doc, b.cli_doc);
    assert_eq!(row(&b.cli_doc)["pieces"], 0);
    assert_eq!(
        b.cli_doc["stages"]["plan_cut"]["settings"]["min_piece_m2"],
        50.0
    );

    // With 10 m2, both keep it.
    let b = both("island-kept", &text, Some(10.0));
    assert_eq!(b.shell_doc, b.cli_doc);
    assert_eq!(row(&b.cli_doc)["pieces"], 1);
    assert!(close(&row(&b.cli_doc)["area_m2"], 36.0));
}
