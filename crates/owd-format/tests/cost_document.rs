//! The rates table in the document and the cost of its stored volumes (spec 0017, ticket 12).
//!
//! The web example's volumes are run again on its DEM so the stations and the volumes by reach are
//! stored; the cost of the document is then the stage's cost of what the document holds.
//!
//! Label: property

use owd_format::design::cost::{RatesTable, cost};
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Stage};
use owd_format::geotiff;
use owd_format::types::TerrainSampler;
use serde_json::{Value, json};
use std::path::PathBuf;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn rates() -> Value {
    json!({
        "dredge_per_m3": {"amount": 10.0, "currency": "USD"},
        "dry_cut_per_m3": {"amount": 5.0, "currency": "USD"},
        "disposal_per_m3": {"amount": 2.0, "currency": "USD"},
        "haul_per_m3_km": {"amount": 1.0, "currency": "USD"},
        "haul_km": 3.0
    })
}

/// The example with its volumes run again, so the volumes by reach are stored.
fn with_volumes() -> Session {
    let text = std::fs::read_to_string(web().join("river-example.owd")).unwrap();
    let dem = std::fs::read(web().join("river-dem.tif")).unwrap();
    let mut s = Session::open(&text).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    s
}

#[test]
fn a_document_without_rates_opens_and_saves_as_before_and_has_no_cost() {
    let text = std::fs::read_to_string(web().join("river-example.owd")).unwrap();
    let s = Session::open(&text).unwrap();
    assert_eq!(s.rates().unwrap(), None);
    assert_eq!(s.to_text(), text);
    let e = cost(&s.cost_input().unwrap()).unwrap_err();
    assert_eq!(e.code, "COST_NO_RATES");
}

#[test]
fn the_cost_of_the_document_is_the_rates_times_its_stored_volumes_by_reach() {
    let mut s = with_volumes();
    let stages = s.document()["stages"].clone();
    let table: RatesTable = serde_json::from_value(rates()).unwrap();
    s.set_rates(Some(table)).unwrap();
    // Rates are new input that no stored stage read: nothing moves.
    assert_eq!(s.document()["stages"], stages);
    assert_eq!(s.document()["inputs"]["rates"], rates());
    let back = Session::open(&s.to_text()).unwrap();
    assert!(back.rates().unwrap().is_some());

    let c = cost(&back.cost_input().unwrap()).unwrap();
    let by_reach = &s.document()["stages"]["volumes"]["results"]["sections"]["by_reach"];
    let reaches = by_reach.as_array().unwrap();
    assert_eq!(c.by_reach.len(), reaches.len());
    for (r, v) in c.by_reach.iter().zip(reaches) {
        let (d, x, t) = (
            v["dredge_m3"].as_f64().unwrap(),
            v["excav_m3"].as_f64().unwrap(),
            v["total_m3"].as_f64().unwrap(),
        );
        assert_eq!(r.items.dredge.amount, 10.0 * d);
        assert_eq!(r.items.dry_cut.amount, 5.0 * x);
        assert_eq!(r.items.disposal.amount, 2.0 * t);
        assert_eq!(r.items.haul.amount, 1.0 * t * 3.0);
    }
    assert!(c.totals.total.amount > 0.0);
}

#[test]
fn stale_volumes_give_no_cost_and_a_bad_rates_table_is_refused() {
    let mut s = with_volumes();
    s.set_rates(Some(serde_json::from_value(rates()).unwrap()))
        .unwrap();
    s.set_water_profile(&[[0.0, 37.0]]).unwrap();
    let e = cost(&s.cost_input().unwrap()).unwrap_err();
    assert_eq!(e.code, "COST_VOLUMES_STALE");

    // A table in two currencies is refused when it is set, and nothing changes.
    let text = s.to_text();
    let mut bad = rates();
    bad["haul_per_m3_km"]["currency"] = json!("EUR");
    let e = s
        .set_rates(Some(serde_json::from_value(bad.clone()).unwrap()))
        .unwrap_err();
    assert!(e.to_string().contains("COST_MIXED_CURRENCY"), "{e}");
    assert_eq!(s.to_text(), text);
    // And a document that holds one does not open.
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    doc["inputs"]["rates"] = bad;
    let e = Session::open(&doc.to_string()).err().unwrap();
    assert!(e.to_string().contains("COST_MIXED_CURRENCY"), "{e}");
}
