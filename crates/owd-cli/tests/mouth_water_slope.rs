//! QA on PR 558, item 9: with a mouth, chainage 0 is the mouth, so `--water-slope` (the fall per
//! metre of chainage) is negative for a river that falls towards its mouth. The help says so, and
//! the stored level rises upstream from the mouth.
//!
//! Label: property

use owd_format::design::crs::Crs;
use owd_format::design::{Role, Session};
use serde_json::{Value, json};
use std::process::Command;

fn owd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn the_help_gives_the_sign_rule_with_a_mouth() {
    let help = String::from_utf8(owd(&["import", "--help"]).stdout).unwrap();
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        help.contains("chainage 0 is the mouth") && help.contains("negative"),
        "{help}"
    );
}

#[test]
fn a_negative_slope_from_the_mouth_rises_upstream() {
    let dir = std::env::temp_dir().join(format!("owd-mouth-slope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A straight river 3 km long, given from upstream (x = 503 000) to its mouth (x = 500 000).
    let line: Vec<[f64; 2]> = (0..=30)
        .map(|i| [503_000.0 - 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Slope", "2026-11-20");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{}}}"#,
            json!(line)
        ),
    )])
    .unwrap();
    let doc = dir.join("d.owd");
    std::fs::write(&doc, s.to_text()).unwrap();
    let [lon, lat] = match Crs::from_epsg(32630).unwrap() {
        Crs::Utm(u) => u.inverse(500_000.0, 600_000.0).unwrap(),
        Crs::Wgs84 => unreachable!(),
    };
    let mouth = format!("{lon},{lat}");
    let p = doc.to_str().unwrap();
    let o = owd(&[
        "import",
        p,
        "--mouth",
        &mouth,
        "--water-level",
        "10",
        "--water-slope",
        "-0.001",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    let pts = d["inputs"]["water_level"]["points"].as_array().unwrap();
    assert_eq!(pts[0], json!([0.0, 10.0]), "10 m at the mouth");
    assert!((pts[1][0].as_f64().unwrap() - 3000.0).abs() < 1e-6);
    assert!(
        (pts[1][1].as_f64().unwrap() - 13.0).abs() < 1e-9,
        "{}",
        pts[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
