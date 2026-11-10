//! The app can load a Rule Set file (ticket #499): after the shell loads a file, the sizing
//! choices list its set, marked as from a file; the design sizes with it and every stage runs, the
//! plan cut included; and the document equals the command line's with `--rule-set-file`. A plan cut
//! of a design sized with a file that is not loaded is refused with the core's message.
//!
//! The design is the committed web example `examples/web/river-example.owd` (the repository's
//! synthetic river, its terrain, the rolling circle). The Rule Set file is made here from the
//! built-in PIANC set: a wider bottom (k 3.6 for 3.2), at a version that is not built in.
//!
//! Label: property

use owd_cli::commands::{SizeFlags, run, size, use_rule_set_files};
use owd_cli::plan_cut::{PlanCutFlags, plan_cut};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const ID: &str = "pianc-wg141";
const VERSION: &str = "0.9.0";
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
    let p = std::env::temp_dir().join(format!("owd-rsf-app-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    std::fs::copy(web("river-dem.tif"), p.join("river-dem.tif")).unwrap();
    Dir(p)
}

fn rule_set() -> String {
    let text = owd_core::rules::engine::BUILT_IN[0]
        .1
        .replace("\"k\": 3.2,", "\"k\": 3.6,")
        .replace("\"version\": \"0.3.0\"", "\"version\": \"0.9.0\"");
    assert!(
        text.contains("\"k\": 3.6,"),
        "the bottom factor was replaced"
    );
    text
}

fn example() -> (String, Vec<u8>) {
    (
        std::fs::read_to_string(web("river-example.owd")).unwrap(),
        std::fs::read(web("river-dem.tif")).unwrap(),
    )
}

/// The Rule Set entries of the shell's sizing choices.
fn listed(shell: &AppState) -> Vec<Value> {
    shell.sizing_choices()["rule_sets"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_choices_list_a_loaded_set_marked_as_from_a_file() {
    let d = dir("choices");
    let doc = d.0.join("d.owd");
    let (text, dem) = example();
    let mut shell = AppState::open(&doc, &text, Some(&dem)).unwrap();

    // Before: the built-in sets, as the static listing has them.
    let before = listed(&shell);
    let built_in = owd_format::design::Session::sizing_choices();
    assert_eq!(json!(before), built_in["rule_sets"]);
    assert!(before.iter().all(|s| s.get("from_file").is_none()));

    assert_eq!(shell.load_rule_set_text(&rule_set()).unwrap(), SET);
    let after = listed(&shell);
    assert_eq!(after.len(), before.len() + 1, "{after:#?}");
    // The built-in sets are listed as before, and the file's set apart from them.
    for s in &before {
        assert!(after.contains(s), "{s} is no longer listed as it was");
    }
    let mine = after
        .iter()
        .find(|s| s["version"] == VERSION)
        .expect("the loaded set is listed");
    assert_eq!(mine["id"], json!(ID));
    assert_eq!(mine["from_file"], json!(true));
    assert!(mine["title"].as_str().is_some_and(|t| !t.is_empty()));
    assert!(
        mine["label"].as_str().unwrap().contains(SET),
        "{}",
        mine["label"]
    );
    // It reads the same set inputs as the built-in set it was made from.
    let pianc = before.iter().find(|s| s["id"] == ID).unwrap();
    assert_eq!(mine["inputs"], pianc["inputs"]);

    // A refused file changes nothing.
    assert!(shell.load_rule_set_text("{ not a rule set").is_err());
    assert_eq!(listed(&shell), after);
}

#[test]
fn every_stage_and_the_plan_cut_give_the_command_lines_document() {
    let d = dir("equal");
    let doc = d.0.join("d.owd");
    let (text, dem) = example();
    let file = d.0.join("mine.json");
    std::fs::write(&file, rule_set()).unwrap();
    std::fs::write(&doc, &text).unwrap();

    // The shell: load the file, choose its set, run every stage, then the plan cut.
    let mut shell = AppState::open(&doc, &text, Some(&dem)).unwrap();
    shell.load_rule_set_text(&rule_set()).unwrap();
    shell.set_rule_sets(&[SET], &[("se_quality", "B")]).unwrap();
    shell.run_stale().expect("every later stage runs");
    shell
        .run_plan_cut(&json!({}))
        .expect("the plan cut runs with the loaded set");
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    // The command line, the same steps with `--rule-set-file mine.json`.
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
    assert!(
        plan_cut(&d_str, PlanCutFlags::default(), &None, true).is_ok(),
        "owd plan-cut failed"
    );
    let from_cli: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();

    let s = &from_cli["stages"]["sizing"]["results"];
    assert_eq!(
        s["bottom_width_straight_m"].as_f64().unwrap(),
        3.6 * s["beam_m"].as_f64().unwrap(),
        "the sized numbers are the file's"
    );
    assert!(!from_cli["stages"]["plan_cut"]["results"].is_null());
    assert_eq!(
        from_shell, from_cli,
        "the two surfaces left different documents"
    );
}

#[test]
fn a_plan_cut_without_the_file_is_refused_with_the_cores_message() {
    let d = dir("refused");
    let doc = d.0.join("d.owd");
    let (text, dem) = example();
    let mut sized = AppState::open(&doc, &text, Some(&dem)).unwrap();
    sized.load_rule_set_text(&rule_set()).unwrap();
    sized.set_rule_sets(&[SET], &[("se_quality", "B")]).unwrap();
    sized.run_stale().unwrap();
    let sized_text = sized.save_text_for(&doc);

    // A new shell with no file loaded.
    let mut fresh = AppState::open(&doc, &sized_text, Some(&dem)).unwrap();
    let core = owd_format::design::Session::open(&sized_text)
        .unwrap()
        .rule_set_problem()
        .expect("the core names the missing set")
        .to_string();
    let e = fresh.run_plan_cut(&json!({})).unwrap_err();
    assert!(e.contains(SET), "{e}");
    assert!(e.contains("Load Rule Set file"), "{e}");
    assert!(!e.contains("is stale"), "{e}");
    assert_eq!(e, core, "the core's message, as it is");
    // Nothing changed.
    assert_eq!(fresh.save_text_for(&doc), sized_text);
    // The snapshot carries the same message, for the app to show.
    assert_eq!(fresh.snapshot()["rule_set_problem"], json!(e));
    assert_eq!(sized.snapshot()["rule_set_problem"], Value::Null);
}
