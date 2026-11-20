//! `owd structures` (spec 0017, ticket 14): `--given FILE` stores given weirs and locks through the
//! one session function, a refusal exits 2 with its code and changes nothing, `--clear` removes
//! them; the volumes run saves the pool checks, prints them with chainage, and `--strict` exits 1
//! when one fails.
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

fn weir(level: f64, currency: &str) -> Value {
    json!({"mode": "given", "items": [
        {"name": "weir A", "kind": "weir", "site": {"chainage": 1500.0},
         "upstream_level_m": level, "height_m": 4.0,
         "cost": {"amount": 1000000.0, "currency": currency}}]})
}

#[test]
fn given_structures_are_stored_checked_by_the_volumes_and_cleared() {
    let dir = std::env::temp_dir().join(format!("owd-structures-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let doc = dir.join("river-example.owd");
    std::fs::copy(web.join("river-example.owd"), &doc).unwrap();
    std::fs::copy(web.join("river-dem.tif"), dir.join("river-dem.tif")).unwrap();
    let original = std::fs::read_to_string(&doc).unwrap();
    let doc_s = doc.to_str().unwrap();
    let file = dir.join("structures.json");
    let file_s = file.to_str().unwrap();

    // Without structures, the volumes print no pool check.
    let o = owd(&["volumes", doc_s]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!all.contains("navigation_depth"), "{all}");
    std::fs::write(&doc, &original).unwrap();

    std::fs::write(&file, weir(2.0, "USD").to_string()).unwrap();
    let o = owd(&["structures", doc_s, "--given", file_s]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let d = read(&doc);
    assert_eq!(d["inputs"]["structures"]["items"][0]["name"], "weir A");
    assert_eq!(d["inputs"]["structures"]["pool_levels"]["below_m"], 0.5);

    // A refusal exits 2 with its code; the stored structures stay.
    let mut two = weir(2.0, "USD");
    two["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"site": {"chainage": 2500.0}, "upstream_level_m": 3.0, "height_m": 4.0,
                     "cost": {"amount": 1.0, "currency": "EUR"}}));
    std::fs::write(&file, two.to_string()).unwrap();
    let o = owd(&["structures", doc_s, "--given", file_s]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("WEIRS_MIXED_CURRENCY"));
    assert_eq!(read(&doc)["inputs"]["structures"], d["inputs"]["structures"]);

    // The volumes place the weir and save the checks; strict exits 1 and names the check.
    let o = owd(&["--strict", "volumes", doc_s]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("navigation_depth"), "{err}");
    let d = read(&doc);
    assert_eq!(d["stages"]["structures"]["results"]["structures"][0]["lift_m"], 2.0);
    assert_eq!(
        d["stages"]["volumes"]["results"]["structure_checks"]["navigation_depth"]["ok"],
        false
    );

    // `owd structures` with no flag prints the plan and the checks, with chainage, and changes
    // nothing.
    let before = std::fs::read_to_string(&doc).unwrap();
    let o = owd(&["structures", doc_s]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("weir A") && out.contains(" m short"), "{out}");
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), before);

    // Clearing removes the input.
    let o = owd(&["structures", doc_s, "--clear"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(read(&doc)["inputs"].get("structures").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
