//! `owd import --buildings` (BLD-1): the file is recorded by reference, read back when the design
//! opens, and a missing or changed file is said as for the DEM. Synthetic data only.
//!
//! Label: property

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn work_dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("inputs")).unwrap();
    d
}

const BUILDINGS: &str = r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"EPSG:32630"}},"features":[
 {"type":"Feature","properties":{"area_m2":80,"confidence":0.9},"geometry":{"type":"Point","coordinates":[500100,600050]}},
 {"type":"Feature","properties":{"area_m2":30,"confidence":0.6},"geometry":{"type":"Point","coordinates":[500150,600050]}}]}"#;

#[test]
fn import_records_the_buildings_and_open_checks_them() {
    let dir = work_dir("buildings-cli");
    let doc = dir.join("design.owd");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd"),
        &doc,
    )
    .unwrap();
    let file = dir.join("inputs/buildings.geojson");
    std::fs::write(&file, BUILDINGS).unwrap();
    let (d, f) = (doc.to_str().unwrap(), file.to_str().unwrap());
    let o = run(&["import", d, "--buildings", f, "--building-min-area", "50"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let said = String::from_utf8_lossy(&o.stdout);
    assert!(
        said.contains("buildings: 2 features, from WGS 84 / UTM zone 30N"),
        "{said}"
    );
    assert!(
        said.contains("buildings counted: 1 (below the confidence threshold 0.7: 1; below the smallest area 50.0 m2: 0)"),
        "{said}"
    );
    let text = std::fs::read_to_string(&doc).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["inputs"]["buildings"]["path"], "inputs/buildings.geojson");
    assert_eq!(v["inputs"]["buildings"]["min_area_m2"], 50.0);

    // Opened with the file there: no warning. Changed: the DEM's words. Missing: the same.
    let o = run(&["status", d]);
    assert!(!String::from_utf8_lossy(&o.stderr).contains("buildings"));
    std::fs::write(&file, BUILDINGS.replace("0.9", "0.95")).unwrap();
    let o = run(&["status", d]);
    assert!(
        String::from_utf8_lossy(&o.stderr).contains(
            "the buildings file `inputs/buildings.geojson` has changed since its hash was recorded"
        ),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    std::fs::remove_file(&file).unwrap();
    let o = run(&["status", d]);
    assert!(
        String::from_utf8_lossy(&o.stderr)
            .contains("the buildings file `inputs/buildings.geojson` is missing"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
