//! The cost stage (spec 0017, ticket 12), tested alone from the fixtures in `tests/fixtures/cost/`:
//! the volumes by reach, whether they are fresh, and the rates table; what the stage must return (or
//! the code it must refuse with, and where); a label and why, with the numbers worked by hand. No
//! session is built.
//!
//! Label: golden (the hand-worked table), property (the rest)

use owd_format::design::cost::{self, CostIn, RatesTable, cost};
use owd_format::stage::contracts;
use owd_format::types::{Chainage, Money};
use serde_json::{Value, json};
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cost");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (
                name,
                serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `want` is contained in `got`: every key of an object, every item of a list, every number to
/// 1e-9.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => w.iter().try_for_each(|(k, wv)| {
            let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
            contained(gv, wv, &format!("{at}/{k}"))
        }),
        (Value::Array(g), Value::Array(w)) => {
            if g.len() != w.len() {
                return Err(format!("{at}: {} items, want {}", g.len(), w.len()));
            }
            g.iter()
                .zip(w)
                .enumerate()
                .try_for_each(|(i, (g, w))| contained(g, w, &format!("{at}/{i}")))
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
fn each_fixture_gives_what_it_says() {
    let all = fixtures();
    assert!(all.len() >= 10, "the fixtures are missing");
    for (name, f) in all {
        let input: CostIn = serde_json::from_value(f["input"].clone())
            .unwrap_or_else(|e| panic!("{name}: the input does not read: {e}"));
        let want = &f["expect"];
        match (cost(&input), want["code"].as_str()) {
            (Ok(got), None) => {
                let got = serde_json::to_value(&got).unwrap();
                contained(&got, &want["cost"], &name).unwrap();
            }
            (Err(e), Some(code)) => {
                assert_eq!(e.code, code, "{name}: {}", e.message);
                assert_eq!(e.stage, cost::STAGE, "{name}");
                if let Some(at) = want["at"].as_f64() {
                    assert_eq!(e.at, Some(Chainage(at)), "{name}");
                }
            }
            (Ok(_), Some(code)) => panic!("{name}: costed, but should be refused with {code}"),
            (Err(e), None) => panic!("{name}: refused with {e}, but should cost"),
        }
    }
}

#[test]
fn the_totals_add_the_reach_figures_and_each_figure_has_room_for_its_level() {
    let f: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cost/two-reaches-by-hand.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let input: CostIn = serde_json::from_value(f["input"].clone()).unwrap();
    let c = cost(&input).unwrap();
    let sum = |pick: &dyn Fn(&cost::CostItems) -> &Money| -> f64 {
        c.by_reach.iter().map(|r| pick(&r.items).amount).sum()
    };
    assert_eq!(sum(&|i| &i.dredge), c.totals.dredge.amount);
    assert_eq!(sum(&|i| &i.haul), c.totals.haul.amount);
    assert_eq!(sum(&|i| &i.total), c.totals.total.amount);
    for r in &c.by_reach {
        let i = &r.items;
        assert_eq!(
            i.total.amount,
            i.dredge.amount + i.dry_cut.amount + i.disposal.amount + i.haul.amount
        );
        assert!(
            [&i.dredge, &i.dry_cut, &i.disposal, &i.haul, &i.total]
                .iter()
                .all(|m| m.currency == "USD")
        );
        // No output level yet (ticket 17 sets it); the field is there and is not written.
        assert_eq!(r.level, None);
    }
    assert_eq!(c.level, None);
    let v = serde_json::to_value(&c).unwrap();
    assert!(v.get("level").is_none());
    // The output reads back as it was written.
    let back: cost::Cost = serde_json::from_value(v).unwrap();
    assert_eq!(back, c);
}

#[test]
fn a_rates_table_reads_from_json_and_refuses_a_misspelt_key() {
    let t: RatesTable = serde_json::from_value(json!({
        "dredge_per_m3": {"amount": 10.0, "currency": "USD"},
        "dry_cut_per_m3": {"amount": 5.0, "currency": "USD"},
        "disposal_per_m3": {"amount": 2.0, "currency": "USD"},
        "haul_per_m3_km": {"amount": 1.0, "currency": "USD"},
        "haul_km": 3.0
    }))
    .unwrap();
    assert_eq!(t.currency().unwrap(), "USD");
    assert!(
        serde_json::from_value::<RatesTable>(json!({"dredge_per_m3": {"amount": 10.0, "currency": "USD"}, "dredge_rate": 1}))
            .is_err()
    );
}

#[test]
fn the_cost_stage_and_the_rates_table_have_schema_files() {
    let names: Vec<String> = contracts().iter().map(|c| c.file_name()).collect();
    assert!(names.contains(&"cost.1.schema.json".to_string()), "{names:?}");
    assert!(
        names.contains(&"inputs.rates.1.schema.json".to_string()),
        "{names:?}"
    );
}
