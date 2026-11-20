//! The shared value types of spec 0017 (stage contracts): each round-trips through JSON as written,
//! has a generated JSON Schema, and refuses what it must.
//!
//! Label: property

use owd_format::types::{
    Chainage, ElevationModel, Money, Polyline, Reach, RiverPolygon, StageError, WaterPoint,
    WaterProfile,
};
use schemars::schema_for;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

/// Parse `text`, write it back, and expect the same JSON value; return the typed value.
fn round_trip<T: Serialize + DeserializeOwned>(text: Value) -> T {
    let v: T = serde_json::from_value(text.clone()).expect("reads");
    assert_eq!(
        serde_json::to_value(&v).unwrap(),
        text,
        "writes back as read"
    );
    v
}

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).unwrap()
}

#[test]
fn chainage_is_a_bare_number() {
    let c: Chainage = round_trip(json!(1250.5));
    assert_eq!(c.metres(), 1250.5);
    assert_eq!(schema_of::<Chainage>()["type"], "number");
}

#[test]
fn polyline_is_a_list_of_xy_pairs() {
    let l: Polyline = round_trip(json!([[0.0, 0.0], [3.0, 4.0], [3.0, 10.0]]));
    assert_eq!(l.len(), 3);
    assert_eq!(l.length_m(), 11.0);
    let s = schema_of::<Polyline>();
    assert_eq!(s["type"], "array");
}

#[test]
fn polyline_refuses_a_point_with_three_numbers() {
    assert!(serde_json::from_value::<Polyline>(json!([[0.0, 0.0, 1.0]])).is_err());
}

#[test]
fn river_polygon_keeps_its_holes() {
    let p: RiverPolygon = round_trip(json!({
        "shell": [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        "holes": [[[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]]
    }));
    assert_eq!(p.holes.len(), 1);
    // An island is dry: the area is the shell less the hole.
    assert_eq!(p.area_m2(), 100.0 - 4.0);
}

#[test]
fn river_polygon_without_holes_reads_with_none() {
    let p: RiverPolygon = serde_json::from_value(json!({
        "shell": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]
    }))
    .unwrap();
    assert!(p.holes.is_empty());
    let s = schema_of::<RiverPolygon>();
    assert!(s["required"].as_array().unwrap().contains(&json!("shell")));
}

#[test]
fn elevation_model_matches_the_document_dem_entry() {
    let m: ElevationModel = round_trip(json!({
        "path": "terrain.tif",
        "sha256": "0".repeat(64),
        "crs": "EPSG:32630",
        "nodata": -9999.0,
    }));
    assert!(!m.holds_bathymetry);
    let m: ElevationModel = round_trip(json!({
        "path": "terrain.tif",
        "sha256": "0".repeat(64),
        "crs": "EPSG:32630",
        "nodata": null,
        "holds_bathymetry": true,
    }));
    assert!(m.holds_bathymetry);
}

#[test]
fn water_profile_is_the_pairs_the_document_holds() {
    let p: WaterProfile = round_trip(json!([[0.0, 10.0], [500.0, 9.5], [500.0, 12.0]]));
    assert_eq!(
        p.points()[2],
        WaterPoint {
            chainage: Chainage(500.0),
            level_m: 12.0
        }
    );
    // Linear between points, flat beyond the ends, and a step takes the later level.
    assert_eq!(p.level_at(Chainage(-5.0)), 10.0);
    assert_eq!(p.level_at(Chainage(250.0)), 9.75);
    assert_eq!(p.level_at(Chainage(500.0)), 12.0);
    assert_eq!(p.level_at(Chainage(900.0)), 12.0);
    assert!(p.level_at(Chainage(f64::NAN)).is_nan());
}

#[test]
fn an_empty_water_profile_has_no_level() {
    let p = WaterProfile::default();
    assert!(p.is_empty());
    assert!(p.level_at(Chainage(0.0)).is_nan());
}

#[test]
fn water_profile_converts_to_and_from_pairs() {
    let pairs = [[0.0, 1.0], [10.0, 2.0]];
    let p = WaterProfile::from_pairs(&pairs);
    assert_eq!(p.to_pairs(), pairs.to_vec());
}

#[test]
fn reach_has_a_length_and_refuses_a_backwards_reach() {
    let r: Reach = round_trip(json!({"from": 100.0, "to": 350.0}));
    assert_eq!(r.length_m(), 250.0);
    assert!(r.contains(Chainage(100.0)) && r.contains(Chainage(350.0)));
    assert!(!r.contains(Chainage(350.1)));
    assert!(Reach::new(Chainage(5.0), Chainage(1.0)).is_err());
}

#[test]
fn money_is_an_amount_and_a_currency_code() {
    let m: Money = round_trip(json!({"amount": 1250.5, "currency": "USD"}));
    assert_eq!(m.currency, "USD");
    assert!(Money::new(1.0, "usd").is_err(), "a code is three capitals");
    assert!(Money::new(1.0, "US").is_err());
    assert!(Money::new(f64::NAN, "USD").is_err());
    assert_eq!(Money::new(1.0, "EUR").unwrap().currency, "EUR");
}

#[test]
fn stage_error_has_a_stable_code_and_an_optional_place() {
    let e: StageError = round_trip(json!({
        "stage": "alignment",
        "code": "ALIGN_RADIUS_UNMET",
        "message": "the line is tighter than R_min",
        "at": 1200.0
    }));
    assert_eq!(e.at, Some(Chainage(1200.0)));
    // `at` is left out when there is no place.
    let e = StageError::new("river", "RIVER_NO_GEOMETRY", "no banks and no centreline");
    let v = serde_json::to_value(&e).unwrap();
    assert!(v.get("at").is_none());
    assert_eq!(
        e.to_string(),
        "river: RIVER_NO_GEOMETRY: no banks and no centreline"
    );
    let s = schema_of::<StageError>();
    for k in ["stage", "code", "message"] {
        assert!(s["required"].as_array().unwrap().contains(&json!(k)), "{k}");
    }
}

#[test]
fn every_type_has_a_schema_that_names_its_fields() {
    for (name, s) in [
        ("Reach", schema_of::<Reach>()),
        ("Money", schema_of::<Money>()),
        ("ElevationModel", schema_of::<ElevationModel>()),
        ("WaterProfile", schema_of::<WaterProfile>()),
    ] {
        assert!(s.is_object(), "{name}");
        assert!(s.get("$schema").is_some(), "{name}");
    }
}

// --- validation holds when a value is read, not only when it is built ------------------------------

#[test]
fn a_reach_that_ends_before_it_starts_is_refused_when_read() {
    use owd_format::types::Reach;
    assert!(serde_json::from_str::<Reach>(r#"{"from": 100.0, "to": 200.0}"#).is_ok());
    let e = serde_json::from_str::<Reach>(r#"{"from": 200.0, "to": 100.0}"#)
        .expect_err("a reach cannot end before it starts");
    assert!(e.to_string().contains("before it starts"), "{e}");
}

#[test]
fn money_with_a_bad_currency_code_is_refused_when_read() {
    use owd_format::types::Money;
    assert!(serde_json::from_str::<Money>(r#"{"amount": 5.0, "currency": "USD"}"#).is_ok());
    for bad in ["usd", "US", "USDX", "U1D", ""] {
        let text = format!(r#"{{"amount": 5.0, "currency": "{bad}"}}"#);
        assert!(
            serde_json::from_str::<Money>(&text).is_err(),
            "`{bad}` is not a currency code"
        );
    }
}

#[test]
fn the_schema_holds_the_bounds_the_language_can_say() {
    use owd_format::types::{Money, Reach};
    let money = serde_json::to_value(schemars::schema_for!(Money)).unwrap();
    assert_eq!(
        money["properties"]["currency"]["pattern"], "^[A-Z]{3}$",
        "{money}"
    );
    // `to` not below `from` is a relation between two fields, which JSON Schema cannot state: the
    // schema says it in words.
    let reach = serde_json::to_value(schemars::schema_for!(Reach)).unwrap();
    let text = reach.to_string();
    assert!(text.contains("Not below"), "{text}");
}
