//! Ticket C3: the channel limits export gives the same text from the app's shell and from
//! `owd export --geojson --limits --wgs84` on the same file.
//!
//! Label: property (the web example: one option)

use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

#[test]
fn the_shell_and_the_cli_give_the_same_limits_text() {
    let text = std::fs::read_to_string(example()).unwrap();
    let s = AppState::open(&example(), &text, None).unwrap();
    let from_shell = s.export_limits_geojson().unwrap();

    let dir = std::env::temp_dir().join(format!("owd-c3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (doc, out) = (dir.join("river.owd"), dir.join("limits.geojson"));
    std::fs::write(&doc, &text).unwrap();
    owd_cli::commands::export_limits(&doc.to_string_lossy(), true, Some(&out.to_string_lossy()))
        .unwrap_or_else(|f| panic!("the CLI: {}", f.message));
    let from_cli = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(from_shell, from_cli);
    let fc: Value = serde_json::from_str(&from_shell).unwrap();
    assert_eq!(fc["screening"], "Screening \u{2014} not for construction");
    let options = s.snapshot()["document"]["options"]
        .as_array()
        .unwrap()
        .len();
    let sides: Vec<&str> = fc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["side"].as_str().unwrap())
        .collect();
    assert_eq!(
        sides.len(),
        2 * options,
        "a left and a right limit for each option"
    );
    assert_eq!(sides.iter().filter(|s| **s == "left").count(), options);
}
