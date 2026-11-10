//! The proxy volume through both surfaces: with no terrain, a volume is the plan cut area times a
//! stated height, labelled "proxy". The command line and the app shell leave the same document.
//!
//! The expected volume is closed-form: (W - w) x L x h, with W worked out here from the sizing in
//! the document: W = bottom width + 2 x side slope x design depth.
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

/// The document each surface leaves for the same text and the same height: (command line, shell).
fn both(tag: &str, text: &str, height_m: f64) -> (Value, Value) {
    let dir = Dir::new(tag);
    let doc = dir.0.join("d.owd");
    std::fs::write(&doc, text).unwrap();

    let mut shell = AppState::open(&doc, text, None).unwrap();
    shell
        .run_plan_cut(&json!({"proxy_height_m": height_m}))
        .expect("the shell runs the plan cut with a proxy height");
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    // `owd plan-cut d.owd --proxy-height-m H --json`
    let flags = PlanCutFlags {
        proxy_height_m: Some(height_m),
        ..Default::default()
    };
    if plan_cut(&doc.to_string_lossy(), flags, &None, true).is_err() {
        panic!("`owd plan-cut --proxy-height-m` failed on {tag}");
    }
    let from_cli: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    (from_cli, from_shell)
}

fn close(got: &Value, want: f64) -> bool {
    let got = got.as_f64().unwrap_or(f64::NAN);
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

#[test]
fn the_proxy_volume_is_the_plan_area_times_the_stated_height_on_both_surfaces() {
    let (w, h) = (30.0, 4.0);
    let text = design(w, &[]);
    let (cli, shell) = both("proxy", &text, h);
    assert_eq!(shell, cli, "the two surfaces left different documents");

    let sizing = &cli["stages"]["sizing"]["results"];
    let wide = sizing["bottom_width_straight_m"].as_f64().unwrap()
        + 2.0 * DEFAULT_WET_SLOPE * sizing["design_depth_m"].as_f64().unwrap();
    let area = (wide - w) * L;
    let r = &cli["stages"]["plan_cut"]["results"]["options"][0];
    assert!(close(&r["area_m2"], area));
    let p = &r["proxy_volume"];
    assert!(
        close(&p["volume_m3"], area * h),
        "{} != {}",
        p["volume_m3"],
        area * h
    );
    assert!(close(&p["by_bank"]["left_m3"], area * h / 2.0));
    assert!(close(&p["by_bank"]["right_m3"], area * h / 2.0));
    // The height is stored with the number, and in the settings.
    assert_eq!(p["height_m"], h);
    assert_eq!(cli["stages"]["plan_cut"]["settings"]["proxy_height_m"], h);
    assert!(
        p["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("proxy")
    );
}
