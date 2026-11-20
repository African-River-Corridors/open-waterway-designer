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
    // A multiplier raster on the example DEM's grid: 2 in every cell.
    let mut grid =
        owd_format::geotiff::read_bytes(&std::fs::read(web.join("river-dem.tif")).unwrap())
            .unwrap();
    grid.data.iter_mut().for_each(|v| *v = 2.0);
    grid.nodata = None;
    std::fs::write(
        dir.join("weights.tif"),
        owd_format::design::dem_source::write_geotiff(&grid).unwrap(),
    )
    .unwrap();
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
    let o = owd(&["align", doc_s, "--method", "rolling-circle"]);
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
    assert!(
        err.contains("VOLUMES_NO_MULTIPLIER") && err.contains("weights.tif"),
        "{err}"
    );

    // Two areas of one name are refused, and the stored ones stay.
    std::fs::write(
        &no_go,
        json!({"type": "FeatureCollection", "features": [box_("a"), box_("a")]}).to_string(),
    )
    .unwrap();
    let o = owd(&["constraints", doc_s, "--no-go", no_go.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("CONSTRAINTS_NAME_DUPLICATE"));
    assert_eq!(
        read(&doc)["inputs"]["constraints"]["no_go"][0]["name"],
        "field"
    );

    // Clearing both parts removes the input.
    let o = owd(&["constraints", doc_s, "--no-no-go", "--no-multiplier"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(read(&doc)["inputs"].get("constraints").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A design with a given line from `a` to `b` and a no-go box `half` metres each side of `centre`.
fn given_line_design(tag: &str) -> (std::path::PathBuf, [f64; 2], [f64; 2]) {
    let dir = std::env::temp_dir().join(format!("owd-constraints-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let doc = dir.join("river-example.owd");
    std::fs::copy(web.join("river-example.owd"), &doc).unwrap();
    let d = read(&doc);
    let c: Vec<[f64; 2]> =
        serde_json::from_value(d["inputs"]["centreline"]["coordinates"].clone()).unwrap();
    (doc, c[c.len() / 4], c[c.len() / 2])
}

fn write_no_go(doc: &std::path::Path, at: [f64; 2], half: f64) {
    let [x, y] = at;
    let f = doc.with_file_name("no-go.geojson");
    std::fs::write(
        &f,
        json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"name": "orchard", "kind": "farmland"},
             "geometry": {"type": "Polygon", "coordinates": [[
                 [x - half, y - half], [x + half, y - half], [x + half, y + half],
                 [x - half, y + half], [x - half, y - half]]]}}]})
        .to_string(),
    )
    .unwrap();
    let o = owd(&[
        "constraints",
        doc.to_str().unwrap(),
        "--no-go",
        f.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn write_line(doc: &std::path::Path, a: [f64; 2], b: [f64; 2]) -> std::path::PathBuf {
    let f = doc.with_file_name("line.geojson");
    std::fs::write(
        &f,
        format!(
            r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{}}}"#,
            json!([a, b])
        ),
    )
    .unwrap();
    f
}

#[test]
fn a_given_line_through_a_no_go_area_is_saved_marked_failed_and_fails_under_strict() {
    let (doc, a, b) = given_line_design("given-fails");
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    write_no_go(&doc, mid, 30.0);
    let line = write_line(&doc, a, b);
    let doc_s = doc.to_str().unwrap();
    let line_s = line.to_str().unwrap();

    // Without --strict: saved, the failed check printed with what it touches, exit 0.
    let o = owd(&["design-line", doc_s, "--line", line_s, "--name", "through"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains("no_go_clear"), "{out}");
    assert!(out.contains("orchard"), "{out}");
    assert!(out.contains(" m to "), "the chainage from and to: {out}");
    let d = read(&doc);
    let r = &d["stages"]["alignment"]["results"];
    assert_eq!(r["method"], "given");
    assert_eq!(r["checks"]["no_go_clear"]["ok"], false);
    assert_eq!(r["constraints_touched"][0]["name"], "orchard");

    // With --strict: still saved, exit 1.
    let o = owd(&[
        "--strict",
        "design-line",
        doc_s,
        "--line",
        line_s,
        "--name",
        "through",
    ]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("no_go_clear"));
    let _ = std::fs::remove_dir_all(doc.parent().unwrap());
}

#[test]
fn a_given_line_clear_of_every_no_go_area_passes_under_strict() {
    let (doc, a, b) = given_line_design("given-clear");
    // A box well off the line: 2 km away.
    write_no_go(&doc, [a[0] + 2000.0, a[1] + 2000.0], 30.0);
    let line = write_line(&doc, a, b);
    let o = owd(&[
        "--strict",
        "design-line",
        doc.to_str().unwrap(),
        "--line",
        line.to_str().unwrap(),
        "--name",
        "clear",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let d = read(&doc);
    assert_eq!(
        d["stages"]["alignment"]["results"]["checks"]["no_go_clear"]["ok"],
        true
    );
    let _ = std::fs::remove_dir_all(doc.parent().unwrap());
}
