//! The structures stage (spec 0017, ticket 14): weirs and locks the engineer gives, the pools they
//! hold, and the structures seam. Tested alone from the fixtures in `tests/fixtures/structures/`
//! (a `StructuresIn`, what the stage must return or the code it must refuse with, and why) and in
//! `tests/fixtures/pool_checks/` (stored stations and water stretches, and the two checks' answer).
//! No session is built for them.
//!
//! Label: property

use owd_format::design::structures::{
    self, FloodRiskCheck, GivenTool, PoolChecksIn, StructureRequest, StructureResponse,
    StructureSource, Structures, StructuresIn, StructuresTool, given, pool_checks, read_structures,
    structures as structures_stage,
};
use owd_format::design::water_stage::{BeyondPools, WaterIn, WaterSource, water};
use owd_format::stage::contracts;
use owd_format::types::{Chainage, Money, Polyline, RiverPolygon, StageError};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;

fn fixtures(dir: &str) -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir);
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let v = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (name, v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn fixture(dir: &str, name: &str) -> Value {
    fixtures(dir)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture {dir}/{name}"))
        .1
}

/// `want` is contained in `got`: every key of an object, every element of an array (same length),
/// every number to 1e-9 relative. Returns the first place it is not.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            for (k, wv) in w {
                let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
                contained(gv, wv, &format!("{at}/{k}"))?;
            }
            Ok(())
        }
        (Value::Array(g), Value::Array(w)) => {
            if g.len() != w.len() {
                return Err(format!("{at}: {} items, want {}", g.len(), w.len()));
            }
            g.iter()
                .zip(w)
                .enumerate()
                .try_for_each(|(i, (a, b))| contained(a, b, &format!("{at}/{i}")))
        }
        (Value::Number(g), Value::Number(w)) => {
            let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
            if (g - w).abs() <= 1e-9 * w.abs().max(1.0) {
                Ok(())
            } else {
                Err(format!("{at}: {g} is not {w}"))
            }
        }
        (g, w) if g == w => Ok(()),
        (g, w) => Err(format!("{at}: got {g}, want {w}")),
    }
}

#[test]
fn every_structures_fixture_gives_its_plan_or_its_code() {
    let all = fixtures("structures");
    assert!(all.len() >= 10, "the fixtures are there: {}", all.len());
    for (name, f) in all {
        let input: StructuresIn = serde_json::from_value(f["input"].clone())
            .unwrap_or_else(|e| panic!("{name}: the input reads as a StructuresIn: {e}"));
        let got = given(&input);
        let expect = &f["expect"];
        match (got, expect["error"].as_str()) {
            (Ok(plan), None) => {
                let v = serde_json::to_value(&plan).unwrap();
                contained(&v, &expect["output"], &name).unwrap();
            }
            (Err(e), Some(code)) => {
                assert_eq!(e.code, code, "{name}: {}", e.message);
                assert_eq!(e.stage, structures::STAGE, "{name}");
                if let Some(at) = expect["at"].as_f64() {
                    assert_eq!(e.at, Some(Chainage(at)), "{name}: {}", e.message);
                }
            }
            (Ok(_), Some(code)) => panic!("{name}: planned, but should be refused with {code}"),
            (Err(e), None) => panic!("{name}: refused with {e}, but should plan"),
        }
    }
}

#[test]
fn every_pool_checks_fixture_gives_its_answer() {
    let all = fixtures("pool_checks");
    assert!(all.len() >= 3, "the fixtures are there: {}", all.len());
    for (name, f) in all {
        let input: PoolChecksIn = serde_json::from_value(f["input"].clone())
            .unwrap_or_else(|e| panic!("{name}: the input reads as a PoolChecksIn: {e}"));
        let got = serde_json::to_value(pool_checks(&input)).unwrap();
        contained(&got, &f["expect"]["output"], &name).unwrap();
    }
}

#[test]
fn a_pool_too_shallow_at_its_minimum_is_flagged_in_plain_words_with_chainage() {
    let f = fixture("pool_checks", "pool-too-shallow-at-its-minimum");
    let input: PoolChecksIn = serde_json::from_value(f["input"].clone()).unwrap();
    let c = pool_checks(&input);
    assert!(!c.navigation_depth.ok);
    assert_eq!(c.failed(), vec!["navigation_depth"]);
    let words = c.messages().join("\n");
    assert!(
        words.contains("500 m") && words.contains("750 m"),
        "{words}"
    );
    assert!(words.contains("0.50 m short"), "{words}");
    // Each check says what it reads and what it cannot know.
    assert!(!c.navigation_depth.note.is_empty());
    assert!(!c.flood_risk.note.is_empty());
}

#[test]
fn a_pool_whose_maximum_tops_the_bank_is_flagged() {
    let f = fixture("pool_checks", "pool-maximum-tops-the-bank");
    let input: PoolChecksIn = serde_json::from_value(f["input"].clone()).unwrap();
    let c = pool_checks(&input);
    let FloodRiskCheck { ok, flags, .. } = &c.flood_risk;
    assert!(!ok);
    assert_eq!(flags.len(), 1);
    assert_eq!(c.failed(), vec!["flood_risk"]);
    let words = c.messages().join("\n");
    assert!(words.contains("500 m") && words.contains("left"), "{words}");
}

/// The structures the fixtures' first golden case places, for the seam and water tests.
fn two_structures() -> StructuresIn {
    serde_json::from_value(
        fixture("structures", "two-structures-given-by-chainage")["input"].clone(),
    )
    .unwrap()
}

/// A structures tool that is not the given input: it records what it was asked and prices each
/// structure at 7 m high and 3 000 000 EUR with a square footprint.
struct FakeTool {
    asked: RefCell<Vec<StructureRequest>>,
    answers: usize,
}

impl StructuresTool for FakeTool {
    fn price(&self, req: &[StructureRequest]) -> Result<Vec<StructureResponse>, StageError> {
        self.asked.borrow_mut().extend(req.iter().cloned());
        Ok(req
            .iter()
            .take(self.answers)
            .map(|r| StructureResponse {
                id: r.id,
                height_m: 7.0,
                footprint: Some(RiverPolygon {
                    shell: Polyline(vec![
                        [r.point[0] - 5.0, r.point[1] - 5.0],
                        [r.point[0] + 5.0, r.point[1] - 5.0],
                        [r.point[0] + 5.0, r.point[1] + 5.0],
                    ]),
                    holes: vec![],
                }),
                cost: Money::new(3_000_000.0, "EUR").unwrap(),
            })
            .collect())
    }
}

#[test]
fn the_seam_sends_site_levels_and_width_and_its_answer_is_used() {
    let input = two_structures();
    let tool = FakeTool {
        asked: RefCell::new(vec![]),
        answers: usize::MAX,
    };
    let plan = structures_stage(&input, &tool).unwrap();
    let asked = tool.asked.borrow();
    assert_eq!(asked.len(), 2);
    // Sent from the mouth: weir A (input 1) first.
    let a = &asked[0];
    assert_eq!(a.id, 1);
    assert_eq!(a.point, [300.0, 10.0]);
    assert_eq!(a.river_chainage, Chainage(300.0));
    assert_eq!(a.chainage, Chainage(300.0));
    assert_eq!(a.upstream_level_m, 10.0);
    assert!((a.downstream_level_m - 8.3).abs() < 1e-12);
    assert_eq!(a.channel_width_m, 20.0);
    assert_eq!(asked[1].downstream_level_m, 10.0);
    // The tool's answer, not the given height and cost.
    for s in &plan.structures {
        assert_eq!(s.height_m, 7.0);
        assert_eq!(s.cost.currency, "EUR");
        assert_eq!(s.source, StructureSource::Seam);
        assert!(s.footprint.is_some());
    }
    assert_eq!(plan.currency, "EUR");
}

#[test]
fn a_seam_that_answers_for_too_few_structures_is_refused() {
    let tool = FakeTool {
        asked: RefCell::new(vec![]),
        answers: 1,
    };
    let e = structures_stage(&two_structures(), &tool).unwrap_err();
    assert_eq!(e.code, "WEIRS_SEAM");
}

#[test]
fn the_pass_through_returns_what_the_engineer_gave() {
    let input = two_structures();
    let items = &input.structures.items;
    let tool = GivenTool::new(items);
    let req = StructureRequest {
        id: 0,
        point: [600.0, 10.0],
        river_chainage: Chainage(600.0),
        chainage: Chainage(600.0),
        upstream_level_m: 12.0,
        downstream_level_m: 10.0,
        channel_width_m: 20.0,
    };
    let got = tool.price(&[req]).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, 0);
    assert_eq!(got[0].height_m, items[0].height_m);
    assert_eq!(got[0].cost, items[0].cost);
    assert_eq!(got[0].footprint, None);
    assert_eq!(tool.source(), StructureSource::Given);
}

#[test]
fn the_plans_pools_feed_the_water_stage_and_fill_min_and_max() {
    let input = two_structures();
    let plan = given(&input).unwrap();
    let w = water(&WaterIn {
        centreline: input.centreline.clone(),
        design_line: input.design_line.clone(),
        given: input.given_water.clone(),
        pools: plan.water_pools(),
        beyond_pools: BeyondPools::OpenRiver,
        step_m: 250.0,
    })
    .unwrap();
    let at = |c: f64| {
        w.points
            .iter()
            .rev()
            .find(|p| p.chainage.0 <= c)
            .unwrap()
            .clone()
    };
    let below = at(100.0);
    assert_eq!(below.source, WaterSource::OpenRiver);
    assert_eq!(below.min_level_m, None);
    let p = at(500.0);
    assert_eq!(p.source, WaterSource::Pool);
    assert_eq!(
        (p.level_m, p.min_level_m, p.max_level_m),
        (10.0, Some(9.5), Some(11.0))
    );
    let p = at(750.0);
    assert_eq!(
        (p.level_m, p.min_level_m, p.max_level_m),
        (12.0, Some(11.5), Some(13.0))
    );
}

#[test]
fn the_input_reads_from_json_writes_its_allowances_and_refuses_by_code() {
    let v = json!({"mode": "given", "items": [{"site": {"chainage": 100.0},
        "upstream_level_m": 5.0, "height_m": 3.0, "cost": {"amount": 1.0, "currency": "USD"}}]});
    let s: Structures = read_structures(&v).unwrap();
    // The allowances are written out, so a later change of default never changes what a stored
    // design meant.
    let back = serde_json::to_value(&s).unwrap();
    assert_eq!(back["pool_levels"], json!({"below_m": 0.5, "above_m": 0.5}));
    assert_eq!(back["items"][0]["kind"], "weir");
    let mut bad = v.clone();
    bad["items"][0]["colour"] = json!("red");
    assert_eq!(read_structures(&bad).unwrap_err().code, "WEIRS_INVALID");
    let mut bad = v.clone();
    bad["items"][0]["upstream_level_m"] = json!("high");
    assert_eq!(read_structures(&bad).unwrap_err().code, "WEIRS_INVALID");
}

#[test]
fn the_stage_and_its_input_have_schema_files() {
    let names: Vec<String> = contracts().iter().map(|c| c.file_name()).collect();
    assert!(
        names.contains(&"structures.1.schema.json".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"inputs.structures.1.schema.json".to_string()),
        "{names:?}"
    );
}
