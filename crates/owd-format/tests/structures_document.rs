//! Given structures in the document (spec 0017, ticket 14): the input set through one function,
//! refused by code with nothing changed; the volumes run places them, stores the plan, reads its
//! pools for the water, and saves the two pool checks; the cost gains a structures line. With no
//! structures, a design behaves exactly as before.
//!
//! Label: property

use owd_format::design::cost::{RatesTable, cost};
use owd_format::design::structures;
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Stage, StageStatus};
use owd_format::geotiff;
use owd_format::types::TerrainSampler;
use serde_json::{Value, json};
use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn example_text() -> String {
    std::fs::read_to_string(examples().join("web/river-example.owd")).unwrap()
}

fn run_volumes(s: &mut Session) {
    let dem = std::fs::read(examples().join("web/river-dem.tif")).unwrap();
    let grid = geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
}

/// One weir halfway up the web example's river (about 3 km long, water at 0 m): it holds 2 m.
fn one_weir(below: f64) -> Value {
    json!({
        "mode": "given",
        "items": [{"name": "weir A", "kind": "weir", "site": {"chainage": 1500.0},
                   "upstream_level_m": 2.0, "height_m": 4.0,
                   "cost": {"amount": 1000000.0, "currency": "USD"}}],
        "pool_levels": {"below_m": below, "above_m": 0.5}
    })
}

#[test]
fn a_design_without_structures_is_byte_for_byte_as_before() {
    for name in [
        "web/river-example.owd",
        "web/river-reroute.owd",
        "web/river-rolling-circle.owd",
        "synthetic-river.owd",
    ] {
        let text = std::fs::read_to_string(examples().join(name)).unwrap();
        let mut s = Session::open(&text).unwrap();
        assert_eq!(s.structures().unwrap(), None, "{name}");
        assert_eq!(s.to_text(), text, "{name}");
        // Clearing what is not there changes nothing.
        s.set_structures_text("").unwrap();
        assert_eq!(s.to_text(), text, "{name}");
    }
}

#[test]
fn volumes_run_again_without_structures_move_no_number() {
    let text = example_text();
    let before: Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::open(&text).unwrap();
    run_volumes(&mut s);
    let d = s.document();
    assert!(d["stages"].get(structures::STAGE).is_none());
    let row = &d["stages"]["volumes"]["results"];
    assert!(row.get("structure_checks").is_none());
    for k in [
        "dredge_m3",
        "excav_m3",
        "total_m3",
        "max_face_h_m",
        "stations",
        "water_source",
    ] {
        assert_eq!(row[k], before["stages"]["volumes"]["results"][k], "{k}");
    }
    for p in d["stages"]["water"]["results"]["points"].as_array().unwrap() {
        assert!(p.get("min_level_m").is_none() && p.get("max_level_m").is_none());
    }
}

#[test]
fn structures_are_set_by_one_function_and_refused_by_code_with_nothing_changed() {
    let text = example_text();
    let mut s = Session::open(&text).unwrap();
    s.set_structures_text(&one_weir(0.5).to_string()).unwrap();
    assert_eq!(s.document()["inputs"]["structures"], one_weir(0.5));
    // The volumes read the structures; the alignment and the optimise stage do not.
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    let set = s.to_text();

    let refused = |s: &mut Session, v: Value, code: &str| {
        let e = s.set_structures_text(&v.to_string()).unwrap_err().to_string();
        assert!(e.contains(code), "{code}: {e}");
        assert_eq!(s.to_text(), set, "{code}: nothing changed");
    };
    let item = |c: f64, level: f64, cur: &str| {
        json!({"site": {"chainage": c}, "upstream_level_m": level, "height_m": 4.0,
               "cost": {"amount": 1.0, "currency": cur}})
    };
    let given = |items: Vec<Value>| json!({"mode": "given", "items": items});
    refused(
        &mut s,
        given(vec![item(1000.0, 3.0, "USD"), item(2000.0, 2.0, "USD")]),
        "WEIRS_LEVELS_NOT_STEPPED",
    );
    refused(
        &mut s,
        given(vec![item(1000.0, 2.0, "USD"), item(1000.0, 3.0, "USD")]),
        "WEIRS_SITES_COINCIDE",
    );
    refused(
        &mut s,
        given(vec![item(1000.0, 2.0, "USD"), item(2000.0, 3.0, "EUR")]),
        "WEIRS_MIXED_CURRENCY",
    );
    refused(
        &mut s,
        given(vec![item(9000.0, 2.0, "USD")]),
        "WEIRS_SITE_OFF_LINE",
    );
    refused(&mut s, json!({"mode": "search"}), "WEIRS_INVALID");

    // Clearing takes the design back to its text before any structure was set.
    s.set_structures_text("  ").unwrap();
    assert_eq!(s.to_text(), text);
}

#[test]
fn the_volumes_run_places_the_structures_fills_min_and_max_and_saves_the_checks() {
    let mut s = Session::open(&example_text()).unwrap();
    s.set_structures_text(&one_weir(0.5).to_string()).unwrap();
    run_volumes(&mut s);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    let plan = s.structure_plan().expect("the plan is stored");
    assert_eq!(plan.structures.len(), 1);
    let w = &plan.structures[0];
    assert_eq!(w.upstream_level_m, 2.0);
    assert_eq!(w.downstream_level_m, 0.0);
    assert_eq!(w.lift_m, 2.0);
    assert_eq!(plan.pools[0].min_level_m, 1.5);
    assert_eq!(plan.pools[0].max_level_m, 2.5);

    let d = s.document();
    // The water stage read the plan's pools: levels and limits in the pool, open river below.
    let pts = d["stages"]["water"]["results"]["points"].as_array().unwrap();
    let in_pool: Vec<&Value> = pts.iter().filter(|p| p["source"] == "pool").collect();
    assert!(!in_pool.is_empty());
    for p in &in_pool {
        assert_eq!(p["level_m"], 2.0);
        assert_eq!(p["min_level_m"], 1.5);
        assert_eq!(p["max_level_m"], 2.5);
    }
    assert!(pts.iter().any(|p| p["source"] == "open_river"));
    let row = &d["stages"]["volumes"]["results"];
    assert_eq!(row["water_source"], "structures");

    // Dug from the pool's design level, the channel is short of its design depth at the minimum
    // level wherever the ground on the line is not deeper: by the 0.5 m allowance at most.
    let checks = &row["structure_checks"];
    assert_eq!(checks["navigation_depth"]["ok"], false, "{checks}");
    for f in checks["navigation_depth"]["shortfalls"].as_array().unwrap() {
        assert!(f["short_by_m"].as_f64().unwrap() <= 0.5 + 1e-9, "{f}");
    }
    let flood = &checks["flood_risk"];
    assert!(flood["checked_stations"].is_u64() && flood["unchecked_stations"].is_u64());
    assert!(structures::failed_checks(row).contains(&"navigation_depth"));

    // The plan and the checks survive a save and reopen, fresh.
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.stage_status(Stage::Volumes), StageStatus::Fresh);
    assert_eq!(back.structure_plan(), Some(plan));

    // With no allowance below, the minimum is the design level: the depth is met.
    let mut s = Session::open(&example_text()).unwrap();
    s.set_structures_text(&one_weir(0.0).to_string()).unwrap();
    run_volumes(&mut s);
    let row = &s.document()["stages"]["volumes"]["results"];
    assert_eq!(row["structure_checks"]["navigation_depth"]["ok"], true);
}

#[test]
fn the_cost_has_a_structures_line_in_the_pool_it_holds_and_in_total() {
    let mut s = Session::open(&example_text()).unwrap();
    s.set_structures_text(&one_weir(0.5).to_string()).unwrap();
    run_volumes(&mut s);
    let table: RatesTable = serde_json::from_value(json!({
        "dredge_per_m3": {"amount": 10.0, "currency": "USD"},
        "dry_cut_per_m3": {"amount": 5.0, "currency": "USD"},
        "disposal_per_m3": {"amount": 2.0, "currency": "USD"},
        "haul_per_m3_km": {"amount": 1.0, "currency": "USD"},
        "haul_km": 3.0
    }))
    .unwrap();
    s.set_rates(Some(table)).unwrap();
    let c = cost(&s.cost_input().unwrap()).unwrap();
    let with: Vec<f64> = c
        .by_reach
        .iter()
        .map(|r| r.items.structures.as_ref().unwrap().amount)
        .collect();
    assert_eq!(with.iter().filter(|a| **a > 0.0).count(), 1, "{with:?}");
    assert_eq!(with.iter().sum::<f64>(), 1_000_000.0);
    assert_eq!(c.totals.structures.as_ref().unwrap().amount, 1_000_000.0);
    let reach = c
        .by_reach
        .iter()
        .find(|r| r.items.structures.as_ref().unwrap().amount > 0.0)
        .unwrap();
    // The weir holds the pool that starts at it.
    assert!((reach.from.0 - s.structure_plan().unwrap().structures[0].chainage.0).abs() < 1e-6);

    // A change to the structures makes the volumes stale; the cost refuses stale volumes.
    s.set_structures_text(&one_weir(0.2).to_string()).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
    assert_eq!(cost(&s.cost_input().unwrap()).unwrap_err().code, "COST_VOLUMES_STALE");
}

#[test]
fn removing_the_structures_and_running_again_removes_the_plan() {
    let mut s = Session::open(&example_text()).unwrap();
    s.set_structures_text(&one_weir(0.5).to_string()).unwrap();
    run_volumes(&mut s);
    s.set_structures_text("").unwrap();
    run_volumes(&mut s);
    let d = s.document();
    assert!(d["stages"].get(structures::STAGE).is_none());
    assert!(d["stages"]["volumes"]["results"].get("structure_checks").is_none());
    assert_eq!(s.structure_plan(), None);
}
