//! The river mouth stated through the command line (`owd import --mouth`) and through the app shell
//! (its mouth field) gives the same document (spec 0017; the one path rule of ADR 0015): both call
//! `owd_shell::import::set_mouth`.
//!
//! The river is a straight synthetic river in UTM 30N, given from upstream, with a water level that
//! falls along it, so `downstream` can tell the mouth; a point at its far end says the same.
//!
//! Label: property

use owd_cli::commands::{self, WaterFlags};
use owd_format::design::{Role, Session, Stage};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A straight river 3 km long, given from its upstream end (x = 503 000) to its mouth.
fn centre() -> Vec<Pt> {
    (0..=30)
        .map(|i| [503_000.0 - 100.0 * f64::from(i), 600_000.0])
        .collect()
}

fn design() -> String {
    let line = format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(centre())
    );
    let mut s = Session::new("Mouth", "2026-11-20");
    s.import(&[(Role::Centreline, &line)]).unwrap();
    // The water falls along the line as given: its end is the lower one.
    s.set_water_profile(&[[0.0, 12.0], [3000.0, 9.0]]).unwrap();
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

/// The documents each surface leaves with the mouth `mouth`: (command line, shell).
fn both(name: &str, mouth: &str) -> (Result<Value, String>, Result<Value, String>) {
    let dir = Dir(std::env::temp_dir().join(format!("owd-mouth-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    std::fs::write(&doc, &text).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    let from_shell = shell
        .set_mouth(mouth)
        .map(|()| serde_json::from_str(&shell.save_text_for(&doc)).unwrap());

    let from_cli = commands::import_toward(
        &doc.to_string_lossy(),
        [
            ("left_bank", None),
            ("right_bank", None),
            ("centreline", None),
            ("river_polygon", None),
        ],
        None,
        Some(mouth),
        None,
        WaterFlags::default(),
        Default::default(),
        &None,
        true,
    )
    .map(|()| serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap())
    .map_err(|f| f.message);
    (from_cli, from_shell)
}

#[test]
fn downstream_gives_the_same_document_through_both_surfaces() {
    let (cli, shell) = both("downstream", "downstream");
    let (cli, shell) = (cli.unwrap(), shell.unwrap());
    assert_eq!(cli, shell);
    // Stored from the mouth: the far end of the line as given, with the water turned with it.
    assert_eq!(
        cli["inputs"]["centreline"]["coordinates"][0],
        json!([500_000.0, 600_000.0])
    );
    assert_eq!(cli["inputs"]["water_level"]["points"][0], json!([0.0, 9.0]));
    assert_eq!(cli["inputs"]["mouth"]["stated_as"], "downstream");
}

#[test]
fn a_point_gives_the_same_document_through_both_surfaces() {
    // The mouth's longitude and latitude, from the project's own zone.
    let [lon, lat] = match owd_format::design::crs::Crs::from_epsg(32630).unwrap() {
        owd_format::design::crs::Crs::Utm(u) => u.inverse(500_010.0, 600_005.0).unwrap(),
        owd_format::design::crs::Crs::Wgs84 => unreachable!(),
    };
    let (cli, shell) = both("point", &format!("{lon},{lat}"));
    let (cli, shell) = (cli.unwrap(), shell.unwrap());
    assert_eq!(cli, shell);
    assert_eq!(
        cli["inputs"]["centreline"]["coordinates"][0],
        json!([500_000.0, 600_000.0])
    );
    assert_eq!(cli["inputs"]["mouth"]["stated_as"], "point");
}

#[test]
fn a_mouth_that_is_not_a_place_is_refused_the_same_way_by_both() {
    let (cli, shell) = both("bad", "upstream");
    let (cli, shell) = (cli.unwrap_err(), shell.unwrap_err());
    assert!(cli.contains("`downstream` or `LON,LAT`"), "{cli}");
    assert_eq!(cli, shell);
}
