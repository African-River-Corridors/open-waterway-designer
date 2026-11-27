//! Constraints (spec 0017, ticket 13) set through the command line (`owd constraints`) and through
//! the app shell (the calls the web app makes) give the same document, and the same refusal: both
//! call `Session::set_constraints` (the one path rule of ADR 0015).
//!
//! Label: property

use owd_cli::constraints::{ConstraintFlags, constraints};
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
    let mut s = Session::new("Constraints", "2026-11-20");
    s.import(&[(Role::Centreline, &line)]).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.to_text()
}

/// A small synthetic multiplier raster in the project CRS: 3 in every cell.
fn raster() -> Vec<u8> {
    let d = owd_format::geotiff::Dem {
        width: 4,
        height: 4,
        data: vec![3.0; 16],
        geotransform: [100.0, 0.0, 500_000.0, 0.0, -100.0, 600_200.0],
        nodata: None,
        epsg: Some(32630),
    };
    owd_format::design::dem_source::write_geotiff(&d).unwrap()
}

fn no_go(names: &[&str]) -> Value {
    let f = |n: &str| {
        json!({"type": "Feature", "properties": {"name": n, "kind": "farmland"},
               "geometry": {"type": "Polygon", "coordinates": [[
                   [501_000.0, 599_950.0], [501_200.0, 599_950.0], [501_200.0, 600_050.0],
                   [501_000.0, 600_050.0], [501_000.0, 599_950.0]]]}})
    };
    json!({"type": "FeatureCollection", "features": names.iter().map(|n| f(n)).collect::<Vec<_>>()})
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn both(name: &str, names: &[&str]) -> (Result<Value, String>, Result<Value, String>) {
    let dir = Dir(std::env::temp_dir().join(format!("owd-cons-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    std::fs::write(&doc, &text).unwrap();
    let areas = dir.0.join("no-go.geojson");
    std::fs::write(&areas, no_go(names).to_string()).unwrap();
    let weights = dir.0.join("weights.tif");
    std::fs::write(&weights, raster()).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    let from_shell = shell
        .set_no_go(&no_go(names).to_string())
        .and_then(|()| shell.set_cut_rate_multiplier(&weights, raster()))
        .map(|()| serde_json::from_str(&shell.save_text_for(&doc)).unwrap());

    let flags = ConstraintFlags {
        no_go: Some(areas.to_string_lossy().into_owned()),
        multiplier: Some(weights.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let from_cli = constraints(&doc.to_string_lossy(), &flags, &None, true)
        .map(|()| serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap())
        .map_err(|f| f.message);
    (from_cli, from_shell)
}

#[test]
fn the_same_constraints_give_the_same_document_through_both_surfaces() {
    let (cli, shell) = both("same", &["field"]);
    let (cli, shell) = (cli.unwrap(), shell.unwrap());
    assert_eq!(cli, shell);
    assert_eq!(cli["inputs"]["constraints"]["no_go"][0]["name"], "field");
    assert_eq!(
        cli["inputs"]["constraints"]["cut_rate_multiplier"]["path"],
        "weights.tif"
    );
}

#[test]
fn the_same_refusal_comes_through_both_surfaces() {
    let (cli, shell) = both("dup", &["a", "a"]);
    let (cli, shell) = (cli.unwrap_err(), shell.unwrap_err());
    assert!(cli.contains("CONSTRAINTS_NAME_DUPLICATE"), "{cli}");
    assert!(shell.contains("CONSTRAINTS_NAME_DUPLICATE"), "{shell}");
}

#[test]
fn clearing_through_the_shell_removes_the_input() {
    let dir = Dir(std::env::temp_dir().join(format!("owd-cons-clear-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let doc = dir.0.join("d.owd");
    let text = design();
    let mut shell = AppState::open(&doc, &text, None).unwrap();
    shell.set_no_go(&no_go(&["field"]).to_string()).unwrap();
    shell
        .set_cut_rate_multiplier(&dir.0.join("w.tif"), raster())
        .unwrap();
    shell.set_no_go("").unwrap();
    shell.clear_cut_rate_multiplier().unwrap();
    assert_eq!(shell.save_text_for(&doc), text);
}
