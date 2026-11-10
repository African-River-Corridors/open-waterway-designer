//! A Rule Set file through the app shell and through the command line (ticket #491): loaded once
//! for the life of the shell's session, it sizes the design and every later stage runs, and the two
//! surfaces leave equal documents. Without it the shell refuses with the same plain message.
//!
//! The design is the committed web example `examples/web/river-example.owd` (the synthetic river,
//! its terrain, the rolling circle); the Rule Set file is the built-in PIANC set with a wider bottom
//! (k 3.6 for 3.2) at a version that is not built in.
//!
//! Label: property

use owd_cli::commands::{SizeFlags, run, size, use_rule_set_files};
use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

const SET: &str = "pianc-wg141@0.9.0";

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(tag: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("owd-rsf-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    std::fs::copy(web("river-dem.tif"), p.join("river-dem.tif")).unwrap();
    Dir(p)
}

fn rule_set() -> String {
    owd_core::rules::engine::BUILT_IN[0]
        .1
        .replace("\"k\": 3.2,", "\"k\": 3.6,")
        .replace("\"version\": \"0.3.0\"", "\"version\": \"0.9.0\"")
}

/// The shell: open the example, load the file once, choose the file's set, run what is due.
fn through_the_shell(doc: &Path, text: &str, dem: &[u8], file_text: &str) -> AppState {
    let mut shell = AppState::open(doc, text, Some(dem)).unwrap();
    shell.load_rule_set_text(file_text).unwrap();
    shell
        .set_rule_sets(&[SET], &[("se_quality", "B")])
        .unwrap();
    shell.run_stale().expect("every later stage runs");
    shell
}

#[test]
fn the_shell_and_the_command_line_give_equal_documents() {
    let d = dir("equal");
    let doc = d.0.join("d.owd");
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let dem = std::fs::read(web("river-dem.tif")).unwrap();
    let file = d.0.join("mine.json");
    std::fs::write(&file, rule_set()).unwrap();
    std::fs::write(&doc, &text).unwrap();

    let mut shell = through_the_shell(&doc, &text, &dem, &rule_set());
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    // The command line: `owd size d.owd --rule-set-file mine.json --rule-set pianc-wg141@0.9.0
    // --set-input se_quality=B`, then `owd run d.owd --rule-set-file mine.json`.
    std::fs::write(&doc, &text).unwrap();
    use_rule_set_files(vec![file.to_string_lossy().into_owned()]);
    let d_str = doc.to_string_lossy().into_owned();
    let flags = SizeFlags {
        rule_sets: vec![SET.into()],
        set_inputs: vec!["se_quality=B".into()],
        vessel: None,
        draught: None,
        custom: None,
    };
    assert!(size(&d_str, flags, &None, true).is_ok(), "owd size failed");
    let _ = run(&d_str, &None, true);
    let from_cli: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();

    let s = &from_cli["stages"]["sizing"]["results"];
    assert_eq!(
        s["bottom_width_straight_m"].as_f64().unwrap(),
        3.6 * s["beam_m"].as_f64().unwrap(),
        "the sized numbers are the file's"
    );
    assert!(!from_cli["stages"]["alignment"]["results"].is_null());
    assert_eq!(from_shell, from_cli, "the two surfaces left different documents");
}

#[test]
fn without_the_file_the_shell_names_the_set_and_its_control() {
    let d = dir("missing");
    let doc = d.0.join("d.owd");
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let dem = std::fs::read(web("river-dem.tif")).unwrap();
    let mut sized = through_the_shell(&doc, &text, &dem, &rule_set());
    let sized_text = sized.save_text_for(&doc);
    let hash = serde_json::from_str::<Value>(&sized_text).unwrap()["stages"]["sizing"]["settings"]
        ["rule_sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["version"] == "0.9.0")
        .unwrap()["hash"]
        .as_str()
        .unwrap()
        .to_string();

    // A new shell with no file loaded.
    let mut fresh = AppState::open(&doc, &sized_text, Some(&dem)).unwrap();
    let e = fresh
        .begin_run("alignment", Some("rolling_circle"))
        .map(|_| ())
        .unwrap_err();
    assert!(e.contains(SET), "{e}");
    assert!(e.contains(&hash), "{e}");
    assert!(e.contains("--rule-set-file"), "{e}");
    assert!(e.contains("Load Rule Set file"), "{e}");
    assert!(!e.contains("is stale"), "{e}");
    let e = fresh.run_stale().unwrap_err();
    assert!(e.contains(SET) && !e.contains("is stale"), "{e}");
}

#[test]
fn a_loaded_file_lasts_for_the_life_of_the_shell() {
    let d = dir("lasts");
    let doc = d.0.join("d.owd");
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let dem = std::fs::read(web("river-dem.tif")).unwrap();
    let mut shell = through_the_shell(&doc, &text, &dem, &rule_set());
    let sized_text = shell.save_text_for(&doc);
    // Another design opened in the same shell keeps the loaded file.
    let reopened = AppState::open(&doc, &sized_text, Some(&dem))
        .unwrap()
        .with_machine_settings_of(&shell);
    let mut reopened = reopened;
    assert!(reopened.begin_run("alignment", Some("rolling_circle")).is_ok());
}
