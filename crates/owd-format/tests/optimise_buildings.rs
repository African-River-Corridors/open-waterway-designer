//! Buildings in the weir search (BLD-1): the building cost and the building report of the optimise
//! stage, on the synthetic river of `optimise.rs` (3 km, UTM 30N), with synthetic buildings beside
//! it. No real river or building is read.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_format::design::features::FeatureInput;
use owd_format::design::{AlignMethod, OptimiseSettings, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn params() -> RiverParams {
    RiverParams {
        length_m: 3000.0,
        wavelength_m: 2400.0,
        amplitude_m: 120.0,
        hill_at: [1500.0, -60.0],
        hill_height_m: 10.0,
        hill_radius_m: 80.0,
        ..RiverParams::default()
    }
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = synthetic_river(params()).unwrap();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    }
}

fn settings() -> Value {
    json!({
        "candidate_sites_m": [500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        "lifts_m": [2.0, 4.0],
        "terminal_min_level_m": 4.0,
        "min_spacing_m": 1000.0,
        "dry_weight": 1.5,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "cut_model": "b",
        "maxw_m": 120.0,
        "rake_half_m": 80.0,
        "rake_step_m": 5.0,
        "ds_step_m": 10.0,
        "pitch_m": 20.0,
        "n_heading": 24,
        "radius_multiples": [1.0, 2.0, 4.0],
        "corridor_half_m": 180.0,
        "cell_m": 5.0,
        "flood_cell_m": 10.0,
    })
}

fn design() -> Session {
    design_with_dem(true)
}

fn design_with_dem(with_dem: bool) -> Session {
    let r = synthetic_river(params()).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let line = [shift(&[0.0, 0.0]), shift(&[3000.0, 0.0])];
    let mut s = Session::new("Buildings", "2026-11-26");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(line)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    if with_dem {
        s.set_dem("dem.tif", b"dem", &dem());
    }
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s
}

/// Buildings every 40 m along the river, 140 m to 400 m from its line on both sides, in the project
/// CRS; every fifth below the confidence threshold.
fn buildings() -> String {
    let mut feats = Vec::new();
    let mut k = 0;
    for i in 0..75 {
        let x = 20.0 + 40.0 * f64::from(i);
        for off in [140.0, 200.0, 260.0, 330.0, 400.0] {
            for side in [1.0, -1.0] {
                let p = shift(&[x, side * off]);
                let conf = if k % 5 == 0 { 0.5 } else { 0.9 };
                feats.push(json!({"type": "Feature", "id": format!("b{k}"), "properties": {"area_m2": 90.0, "confidence": conf}, "geometry": {"type": "Point", "coordinates": p}}));
                k += 1;
            }
        }
    }
    json!({"type": "FeatureCollection", "crs": {"type": "name", "properties": {"name": "EPSG:32630"}}, "features": feats}).to_string()
}

#[test]
fn the_building_settings_are_off_by_default_round_trip_and_are_validated() {
    let d = OptimiseSettings::default();
    assert_eq!(d.building_cost, 0.0);
    assert_eq!(d.building_min_depth_m, 0.3);
    for k in ["building_cost", "building_min_depth_m"] {
        assert!(
            d.to_value().get(k).is_none(),
            "{k} is written at its default"
        );
    }
    let mut s = design();
    s.set_optimise_settings(&json!({"building_cost": 5.0e4, "building_min_depth_m": 0.5}))
        .unwrap();
    let st = &s.document()["stages"]["optimise"]["settings"];
    assert_eq!(
        (
            st["building_cost"].clone(),
            st["building_min_depth_m"].clone()
        ),
        (json!(5.0e4), json!(0.5))
    );
    for (k, bad) in [
        ("building_cost", json!(-1.0)),
        ("building_min_depth_m", json!(-0.1)),
        ("building_cost", json!("a lot")),
    ] {
        let before = s.to_text();
        let e = s
            .set_optimise_settings(&json!({ k: bad }))
            .unwrap_err()
            .to_string();
        assert!(e.contains(k), "{e}");
        assert_eq!(s.to_text(), before);
    }
    s.set_optimise_settings(&json!({"building_cost": 0.0, "building_min_depth_m": 0.3}))
        .unwrap();
    assert_eq!(s.document()["stages"]["optimise"]["settings"], d.to_value());
}

#[test]
fn a_building_cost_with_no_buildings_or_no_dem_is_refused_in_plain_words() {
    let mut s = design();
    let before = s.to_text();
    let e = s
        .run_optimise(&json!({"building_cost": 1.0e5}), Some(&terrain))
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "a building cost (`building_cost`) needs buildings to count: load a buildings file (`owd import --buildings FILE`, or Buildings in the web app's Data), or set the building cost to 0"
    );
    assert_eq!(s.to_text(), before);
    // Named, but the file is not attached (missing or changed).
    s.set_features(FeatureInput::Buildings, "b.geojson", buildings().as_bytes())
        .unwrap();
    let mut named = Session::open(&s.to_text()).unwrap();
    named.set_dem("dem.tif", b"dem", &dem());
    let e = named
        .run_optimise(&json!({"building_cost": 1.0e5}), Some(&terrain))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("needs the buildings file `b.geojson`, which is missing or has changed"),
        "{e}"
    );
    // No DEM: the plan-area refusal names the building cost.
    let mut flat = design_with_dem(false);
    let e = flat
        .run_optimise(&json!({"building_cost": 1.0e5}), None)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("a building cost (`building_cost`) cannot be worked out without a DEM"),
        "{e}"
    );
}

/// At building cost 0 the search is the one with no buildings at all, byte for byte; the report
/// then only adds what the pools flood. As the cost rises, fewer buildings flood, never more.
#[test]
fn at_zero_the_search_is_unchanged_and_a_rising_cost_floods_fewer_buildings() {
    let mut plain = design();
    let mut set = settings();
    set["flood_report"] = json!(true);
    plain.run_optimise(&set, Some(&terrain)).unwrap();
    let mut zero = design();
    zero.set_features(FeatureInput::Buildings, "b.geojson", buildings().as_bytes())
        .unwrap();
    zero.run_optimise(&set, Some(&terrain)).unwrap();
    let a = plain.document()["stages"]["optimise"]["results"].clone();
    let mut b = zero.document()["stages"]["optimise"]["results"].clone();
    let report = b["cascade"]["flood"]["buildings"].clone();
    // The buildings are in the report only: take them out and the results are the same.
    b["cascade"]["flood"]
        .as_object_mut()
        .unwrap()
        .remove("buildings");
    for p in b["cascade"]["flood"]["pools"].as_array_mut().unwrap() {
        for k in ["buildings_flooded", "buildings_apart", "building_cost"] {
            p.as_object_mut().unwrap().remove(k);
        }
    }
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    let flooded0 = report["flooded"].as_u64().unwrap();
    assert!(flooded0 > 0, "the 4 m pools flood buildings: {report}");
    assert_eq!(report["building_cost"], json!(0.0));
    assert!(
        report["flooded_below_confidence"].as_u64().unwrap() > 0,
        "counted apart, not dropped"
    );
    assert!(report["first"].as_array().unwrap().len() <= 20);
    assert!(report["first"][0]["lon_lat"].is_array());
    let bands: u64 = report["by_confidence"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(
        bands,
        flooded0
            + report["flooded_below_confidence"].as_u64().unwrap()
            + report["flooded_below_area"].as_u64().unwrap()
    );

    let mut last = flooded0;
    for cost in [1.0e5, 1.0e6, 1.0e8] {
        let mut s = zero.clone();
        let mut set2 = set.clone();
        set2["building_cost"] = json!(cost);
        s.run_optimise(&set2, Some(&terrain)).unwrap();
        let c = &s.document()["stages"]["optimise"]["results"]["cascade"];
        let n = c["flood"]["buildings"]["flooded"].as_u64().unwrap();
        println!(
            "building cost {cost}: {n} buildings flooded; weirs {}",
            c["weirs"]
        );
        assert!(n <= last, "cost {cost}: {n} > {last}");
        assert!(
            (c["flood"]["buildings"]["building_cost"].as_f64().unwrap() - cost * n as f64).abs()
                < 1e-6
        );
        let total = c["total_cost"].as_f64().unwrap();
        let parts = c["flood"]["cut_cost"].as_f64().unwrap()
            + c["flood"]["flood_cost"].as_f64().unwrap()
            + cost * n as f64
            + c["structure_cost"].as_f64().unwrap();
        assert!(
            (total - parts).abs() < 1e-6 * total.max(1.0),
            "{total} against {parts}"
        );
        last = n;
    }
    assert!(
        last < flooded0,
        "a high building cost moves the search: {last} against {flooded0}"
    );
}

/// With a building cost, the buildings file and its filters are part of the optimise stage's hash;
/// without one, loading buildings leaves the stage fresh.
#[test]
fn the_buildings_are_hashed_only_when_the_search_reads_them() {
    let mut s = design();
    s.run_optimise(&settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    s.set_features(FeatureInput::Buildings, "b.geojson", buildings().as_bytes())
        .unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    s.set_optimise_settings(&json!({"building_cost": 1.0e5}))
        .unwrap();
    s.run_optimise(&Value::Null, Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    let mut f = s.building_filters();
    f.min_confidence = 0.95;
    s.set_building_filters(f).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Stale);
    // Moving the file (its path only) leaves it fresh again once the filters are back.
    f.min_confidence = 0.7;
    s.set_building_filters(f).unwrap();
    s.set_features_path(FeatureInput::Buildings, "../elsewhere/b.geojson");
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
}
