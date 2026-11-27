//! Buildings, points of interest and settlements as inputs (BLD-1): read from a file, recorded by
//! reference, checked against their hash like the DEM. Synthetic data only.
//!
//! Label: property

use owd_format::design::Session;
use owd_format::design::crs::Utm;
use owd_format::design::features::{BuildingFilters, FeatureInput, map_geojson};
use serde_json::{Value, json};

const UTM30N: Utm = Utm {
    zone: 30,
    south: false,
};

fn example() -> Session {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd"),
    )
    .unwrap();
    Session::open(&text).unwrap()
}

/// `n` square buildings of 10 m in a row east of (500100, 600000), in WGS84, with an area and a
/// confidence that steps down from 0.95.
fn buildings(n: usize) -> String {
    let inv = UTM30N.inverse_fn().unwrap();
    let feats: Vec<Value> = (0..n)
        .map(|i| {
            let (x, y) = (
                500_100.0 + 20.0 * (i % 500) as f64,
                600_000.0 + 20.0 * (i / 500) as f64,
            );
            let ring: Vec<Value> = [
                [x, y],
                [x + 10.0, y],
                [x + 10.0, y + 10.0],
                [x, y + 10.0],
                [x, y],
            ]
            .iter()
            .map(|p| {
                let q = inv(*p).unwrap();
                json!([q[0], q[1]])
            })
            .collect();
            json!({"type": "Feature",
                "properties": {"area_m2": 100.0, "confidence": 0.95 - 0.1 * (i % 4) as f64},
                "geometry": {"type": "Polygon", "coordinates": [ring]}})
        })
        .collect();
    json!({"type": "FeatureCollection", "features": feats}).to_string()
}

#[test]
fn buildings_are_recorded_by_reference_and_round_trip() {
    let mut s = example();
    let text = buildings(12);
    let set = s
        .set_features(
            FeatureInput::Buildings,
            "../inputs/buildings.geojson",
            text.as_bytes(),
        )
        .unwrap();
    assert_eq!(set.features.len(), 12);
    let b = &set.features[0];
    // The centroid of the first square, in the project CRS, to a millimetre.
    assert!(
        (b.at[0] - 500_105.0).abs() < 1e-3 && (b.at[1] - 600_005.0).abs() < 1e-3,
        "{:?}",
        b.at
    );
    assert_eq!(b.area_m2, 100.0);
    assert_eq!(b.confidence, 0.95);
    let rec = s.document()["inputs"]["buildings"].clone();
    assert_eq!(rec["path"], "../inputs/buildings.geojson");
    assert_eq!(rec["count"], 12);
    assert_eq!(rec["crs"], "WGS 84");
    assert_eq!(rec["min_confidence"], 0.7);
    assert_eq!(rec["sha256"].as_str().unwrap().len(), 64);
    // Counted apart, not dropped: confidence 0.95, 0.85, 0.75, 0.65 in turn.
    let shown = s.feature_inputs();
    assert_eq!(shown["buildings"]["counted"], 9);
    assert_eq!(shown["buildings"]["below_confidence"], 3);

    // Saved and opened again: the record stays, the features are not in the file, and the file
    // validates. Attached again from the same bytes, they are back.
    let saved = s.to_text();
    assert!(
        !saved.contains("0.65"),
        "no feature is copied into the design"
    );
    let mut back = Session::open(&saved).unwrap();
    assert!(back.validate().is_empty());
    assert_eq!(back.document()["inputs"]["buildings"], rec);
    assert!(back.features(FeatureInput::Buildings).is_none());
    back.attach_features(FeatureInput::Buildings, text.as_bytes())
        .unwrap();
    assert_eq!(
        back.features(FeatureInput::Buildings)
            .unwrap()
            .features
            .len(),
        12
    );
}

#[test]
fn the_design_file_does_not_grow_with_the_number_of_buildings() {
    let sizes: Vec<usize> = [10, 5_000]
        .iter()
        .map(|&n| {
            let mut s = example();
            s.set_features(
                FeatureInput::Buildings,
                "b.geojson",
                buildings(n).as_bytes(),
            )
            .unwrap();
            s.to_text().len()
        })
        .collect();
    // Only the count's digits differ.
    assert!(sizes[1] - sizes[0] <= 3, "{sizes:?}");
}

#[test]
fn a_missing_or_changed_file_is_said_as_for_the_dem_and_a_changed_one_is_refused() {
    let mut s = example();
    let text = buildings(3);
    s.set_features(FeatureInput::Buildings, "b.geojson", text.as_bytes())
        .unwrap();
    let back = Session::open(&s.to_text()).unwrap();
    let missing = back
        .features_warning(FeatureInput::Buildings, None)
        .unwrap();
    assert_eq!(
        missing,
        "the buildings file `b.geojson` is missing: load it again, or the map and the reports go without it"
    );
    let other = buildings(4);
    let changed = back
        .features_warning(FeatureInput::Buildings, Some(other.as_bytes()))
        .unwrap();
    assert!(
        changed.contains("has changed since its hash was recorded"),
        "{changed}"
    );
    assert!(
        back.features_warning(FeatureInput::Buildings, Some(text.as_bytes()))
            .is_none()
    );
    let mut b2 = back.clone();
    let e = b2
        .attach_features(FeatureInput::Buildings, other.as_bytes())
        .unwrap_err()
        .to_string();
    assert!(e.contains("has changed since its hash was recorded"), "{e}");
    assert!(b2.features(FeatureInput::Buildings).is_none());
}

#[test]
fn loading_buildings_leaves_every_stage_as_it_was() {
    let s0 = example();
    let mut s = s0.clone();
    s.set_features(
        FeatureInput::Buildings,
        "b.geojson",
        buildings(3).as_bytes(),
    )
    .unwrap();
    assert_eq!(s.status(), s0.status());
    for st in owd_format::design::Stage::ALL {
        assert_eq!(s.stage_hash(st), s0.stage_hash(st), "{st:?}");
    }
}

#[test]
fn a_file_in_the_project_crs_and_points_are_read_and_bad_values_refused() {
    let mut s = example();
    let utm = json!({"type": "FeatureCollection",
    "crs": {"type": "name", "properties": {"name": "EPSG:32630"}},
    "features": [
        {"type": "Feature", "properties": {"confidence": 0.8}, "geometry": {"type": "Point", "coordinates": [500200.0, 600100.0]}},
        {"type": "Feature", "properties": {}, "geometry": null}
    ]});
    let set = s
        .set_features(
            FeatureInput::Buildings,
            "b.geojson",
            utm.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(set.features[0].at, [500200.0, 600100.0]);
    assert!(set.features[0].area_m2.is_nan());
    assert_eq!(set.skipped, 1);
    assert_eq!(set.crs, "WGS 84 / UTM zone 30N");

    let bad = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"confidence": 1.5}, "geometry": {"type": "Point", "coordinates": [-3.0, 5.4]}}]});
    let e = s
        .clone()
        .set_features(
            FeatureInput::Buildings,
            "b.geojson",
            bad.to_string().as_bytes(),
        )
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "the buildings file: feature 1 has a confidence of 1.5: it must be from 0 to 1"
    );
    // A new design has no project CRS: refused with what to do.
    let mut fresh = Session::new("T", "2026-11-26");
    let e = fresh
        .set_features(
            FeatureInput::Buildings,
            "b.geojson",
            utm.to_string().as_bytes(),
        )
        .unwrap_err()
        .to_string();
    assert!(e.starts_with("load the river first"), "{e}");
    // A filter out of range is refused by name.
    let e = s
        .set_building_filters(BuildingFilters {
            min_confidence: 2.0,
            ..Default::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("from 0 to 1"), "{e}");
}

#[test]
fn places_keep_their_properties_and_settlements_their_polygons() {
    let mut s = example();
    let poi = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"name": "Riverside", "kind": "community", "area_ha": 12.5, "parts": 3},
         "geometry": {"type": "Point", "coordinates": [-3.0, 5.43]}},
        {"type": "Feature", "properties": {"name": "Old ferry", "kind": "ferry"},
         "geometry": {"type": "LineString", "coordinates": [[-3.0, 5.43], [-3.0, 5.44]]}}]});
    let set = s
        .set_features(
            FeatureInput::PointsOfInterest,
            "poi.geojson",
            poi.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(set.features[0].name, "Riverside");
    assert_eq!(set.features[0].props["parts"], 3);
    // A line is labelled at its middle.
    assert!((set.features[1].lon_lat[1] - 5.435).abs() < 1e-6);
    let e = s
        .clone()
        .set_features(FeatureInput::PointsOfInterest, "p.geojson",
            json!({"type": "Feature", "properties": {"name": 7}, "geometry": {"type": "Point", "coordinates": [-3.0, 5.43]}}).to_string().as_bytes())
        .unwrap_err()
        .to_string();
    assert!(e.contains("`name` 7: it must be text"), "{e}");

    let settle = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"name": ""},
         "geometry": {"type": "Polygon", "coordinates": [[[-3.0, 5.43], [-2.999, 5.43], [-2.999, 5.431], [-3.0, 5.431], [-3.0, 5.43]]]}}]});
    let set = s
        .set_features(
            FeatureInput::Settlements,
            "s.geojson",
            settle.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(set.features[0].polygons.len(), 1);
    assert!(set.features[0].area_m2 > 12_000.0 && set.features[0].area_m2 < 12_500.0);
    let e = s
        .clone()
        .set_features(
            FeatureInput::Settlements,
            "s.geojson",
            poi.to_string().as_bytes(),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("a settlement must be a polygon"), "{e}");
}

#[test]
fn the_map_file_numbers_features_as_the_session_does() {
    let text = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"confidence": 0.9}, "geometry": null},
        {"type": "Feature", "id": "b-7", "properties": {"confidence": 0.9, "area_m2": 40},
         "geometry": {"type": "Point", "coordinates": [-3.0, 5.43]}}]})
    .to_string();
    let v = map_geojson(FeatureInput::Buildings, &text).unwrap();
    assert_eq!(v["features"].as_array().unwrap().len(), 1);
    assert_eq!(v["features"][0]["properties"]["fid"], 0);
    assert_eq!(v["features"][0]["properties"]["id"], "b-7");
    assert_eq!(v["features"][0]["properties"]["area_m2"], 40);
}
