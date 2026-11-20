//! `owd constraints` (spec 0017, ticket 13): `--no-go FILE` and `--multiplier FILE` store the
//! constraints through the one session function; the multiplier is found beside the design by its
//! recorded hash, and a stage that reads it refuses by name when it is not there.
//!
//! Label: property

use serde_json::{Value, json};
use std::process::Command;

fn owd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .unwrap()
}

fn read(p: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn constraints_are_stored_read_by_the_stages_and_a_missing_multiplier_is_refused_by_name() {
    let dir = std::env::temp_dir().join(format!("owd-constraints-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let doc = dir.join("river-example.owd");
    std::fs::copy(web.join("river-example.owd"), &doc).unwrap();
    std::fs::copy(web.join("river-dem.tif"), dir.join("river-dem.tif")).unwrap();
    // The example's own DEM stands in for a multiplier raster: every cell is a positive number.
    std::fs::copy(web.join("river-dem.tif"), dir.join("weights.tif")).unwrap();
    let d = read(&doc);
    let pts: Vec<[f64; 2]> =
        serde_json::from_value(d["stages"]["alignment"]["results"]["line"]["coordinates"].clone())
            .unwrap();
    let [x, y] = pts[pts.len() / 2];
    let box_ = |name: &str| {
        json!({"type": "Feature", "properties": {"name": name, "kind": "farmland"},
               "geometry": {"type": "Polygon", "coordinates": [[
                   [x - 30.0, y - 30.0], [x + 30.0, y - 30.0], [x + 30.0, y + 30.0],
                   [x - 30.0, y + 30.0], [x - 30.0, y - 30.0]]]}})
    };
    let no_go = dir.join("no-go.geojson");
    std::fs::write(
        &no_go,
        json!({"type": "FeatureCollection", "features": [box_("field")]}).to_string(),
    )
    .unwrap();
    let doc_s = doc.to_str().unwrap();
    let o = owd(&[
        "constraints",
        doc_s,
        "--no-go",
        no_go.to_str().unwrap(),
        "--multiplier",
        dir.join("weights.tif").to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let d = read(&doc);
    let c = &d["inputs"]["constraints"];
    assert_eq!(c["no_go"][0]["name"], "field");
    assert_eq!(c["cut_rate_multiplier"]["path"], "weights.tif");

    // The design line reports the area it touches; the volumes hold the weighted cut.
    let o = owd(&["align", doc_s]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = owd(&["volumes", doc_s]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let d = read(&doc);
    assert_eq!(
        d["stages"]["alignment"]["results"]["constraints_touched"][0]["name"],
        "field"
    );
    let st = &d["stages"]["volumes"]["results"]["sections"]["stations"][0];
    assert!(st["weighted_dredge_area_m2"].is_number(), "{st}");

    // Without the raster beside the design, the volumes refuse by name.
    std::fs::remove_file(dir.join("weights.tif")).unwrap();
    let o = owd(&["volumes", doc_s]);
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("VOLUMES_NO_MULTIPLIER") && err.contains("weights.tif"), "{err}");

    // Two areas of one name are refused, and the stored ones stay.
    std::fs::write(
        &no_go,
        json!({"type": "FeatureCollection", "features": [box_("a"), box_("a")]}).to_string(),
    )
    .unwrap();
    let o = owd(&["constraints", doc_s, "--no-go", no_go.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("CONSTRAINTS_NAME_DUPLICATE"));
    assert_eq!(read(&doc)["inputs"]["constraints"]["no_go"][0]["name"], "field");

    // Clearing both parts removes the input.
    let o = owd(&["constraints", doc_s, "--no-no-go", "--no-multiplier"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(read(&doc)["inputs"].get("constraints").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
