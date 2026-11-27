//! `owd cost` (spec 0017, ticket 12): `--rates FILE` stores the rates table; the command prints the
//! cost of the stored volumes by reach and in total, or refuses with the cost stage's code.
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

#[test]
fn rates_are_stored_and_a_design_without_stored_volumes_is_refused_by_code() {
    let dir = std::env::temp_dir().join(format!("owd-cost-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let doc = dir.join("river-example.owd");
    std::fs::copy(web.join("river-example.owd"), &doc).unwrap();
    let rates = dir.join("rates.json");
    let table = json!({
        "dredge_per_m3": {"amount": 10.0, "currency": "USD"},
        "dry_cut_per_m3": {"amount": 5.0, "currency": "USD"},
        "disposal_per_m3": {"amount": 2.0, "currency": "USD"},
        "haul_per_m3_km": {"amount": 1.0, "currency": "USD"},
        "haul_km": 3.0
    });
    std::fs::write(&rates, table.to_string()).unwrap();
    let o = owd(&[
        "cost",
        doc.to_str().unwrap(),
        "--rates",
        rates.to_str().unwrap(),
    ]);
    // The example's volumes record holds no volumes by reach (saved before they were stored).
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("COST_NO_VOLUMES"), "{err}");
    // The rates were stored all the same.
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["inputs"]["rates"], table);

    // A table in two currencies is refused, and the stored one stays.
    let mut bad = table.clone();
    bad["dredge_per_m3"]["currency"] = json!("EUR");
    std::fs::write(&rates, bad.to_string()).unwrap();
    let o = owd(&[
        "cost",
        doc.to_str().unwrap(),
        "--rates",
        rates.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("COST_MIXED_CURRENCY"));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    assert_eq!(d["inputs"]["rates"], table);
    let _ = std::fs::remove_dir_all(&dir);
}
