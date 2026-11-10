//! The terrain rule in one place.
//!
//! A document names its terrain file and records the file's hash. Every stage that needs terrain
//! uses that file when it is there with the recorded hash, and refuses it by name when it is
//! missing or has changed. `owd align --method reroute` follows the same rule as the other stages:
//! it needs no `--dem` when the document already names its terrain.
//!
//! Label: property

use owd_cli::commands::{AlignFlags, align};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh folder with the example re-route design and its terrain file beside it.
fn design(tag: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-terrain-rule-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (doc, dem) = (dir.join("river-reroute.owd"), dir.join("river-dem.tif"));
    std::fs::copy(root().join("examples/web/river-reroute.owd"), &doc).unwrap();
    std::fs::copy(root().join("examples/web/river-dem.tif"), &dem).unwrap();
    (doc, dem)
}

/// `owd align --method reroute`, with `--dem` or without. `Ok` when the stage ran and wrote its
/// option (exit code 0, or 1 when the option fails a design check). `Err` holds the message of a
/// refusal.
fn reroute(doc: &Path, dem: Option<&Path>) -> Result<(), String> {
    let flags = AlignFlags {
        step: None,
        passes: None,
        chord: None,
        dem: dem.map(|p| p.to_string_lossy().into_owned()),
        water_level: None,
        water_slope: None,
        corridors: None,
        fairness_weight: None,
        use_water: None,
    };
    match align(&doc.to_string_lossy(), "reroute", flags, &None, true) {
        Ok(()) => Ok(()),
        Err(f) if f.code == 1 => Ok(()),
        Err(f) => Err(f.message),
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The same document with its re-route option taken out, so a test can tell that the command wrote
/// it again.
fn without_reroute(doc: &Path) {
    let mut d = read(doc);
    d["options"] = Value::Array(vec![]);
    std::fs::write(doc, serde_json::to_string_pretty(&d).unwrap()).unwrap();
}

#[test]
fn the_reroute_uses_the_terrain_the_document_names() {
    let (with_flag, dem) = design("flag");
    let (no_flag, _) = design("no-flag");
    without_reroute(&with_flag);
    without_reroute(&no_flag);

    reroute(&with_flag, Some(&dem)).expect("the re-route runs with `--dem`");
    if let Err(why) = reroute(&no_flag, None) {
        panic!("the re-route did not use the terrain the document names: {why}");
    }
    let d = read(&no_flag);
    assert!(
        d["options"]
            .as_array()
            .is_some_and(|o| o.iter().any(|o| o["method"] == "reroute")),
        "the document holds no re-route option"
    );
    assert_eq!(
        d,
        read(&with_flag),
        "the document's own terrain and the same file given with `--dem` give different documents"
    );
}

#[test]
fn the_reroute_names_terrain_that_is_missing_or_has_changed() {
    let (doc, dem) = design("changed");
    let mut bytes = std::fs::read(&dem).unwrap();
    bytes.push(0);
    std::fs::write(&dem, bytes).unwrap();
    let why = reroute(&doc, None).expect_err("a changed terrain file is refused");
    assert!(
        why.contains("river-dem.tif") && why.contains("has changed since its hash was recorded"),
        "the refusal does not say the terrain file has changed: {why}"
    );

    std::fs::remove_file(&dem).unwrap();
    let why = reroute(&doc, None).expect_err("a missing terrain file is refused");
    assert!(
        why.contains("river-dem.tif") && why.contains("is missing"),
        "the refusal does not say the terrain file is missing: {why}"
    );
}

#[test]
fn the_rule_is_written_once() {
    // `dem_check` is the hash comparison. One function reads its answer and hands out the terrain;
    // no command and no shell call reads it again.
    let mut calls = Vec::new();
    for dir in ["crates/owd-cli/src", "crates/owd-shell/src"] {
        for entry in std::fs::read_dir(root().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let n = std::fs::read_to_string(&path)
                    .unwrap()
                    .matches(".dem_check(")
                    .count();
                if n > 0 {
                    calls.push(format!(
                        "{dir}/{} ({n})",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                }
            }
        }
    }
    calls.sort();
    let total: usize = calls
        .iter()
        .map(|c| {
            c.rsplit_once('(')
                .unwrap()
                .1
                .trim_end_matches(')')
                .parse::<usize>()
                .unwrap()
        })
        .sum();
    assert!(
        total <= 1,
        "the terrain rule is read in {total} places: {}",
        calls.join(", ")
    );
}
