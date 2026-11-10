//! Ticket C29: the alignment method `centreline` gives the same option from the app's shell and
//! from `owd align --method centreline`, and its line is the river centreline of the document.
//!
//! Label: property (the web example: a given centreline)

use owd_cli::commands::AlignFlags;
use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

fn centreline_option(doc: &Value) -> Value {
    doc["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "centreline")
        .expect("no option of the method `centreline`")
        .clone()
}

#[test]
fn the_shell_and_the_cli_give_the_same_centreline_option() {
    let text = std::fs::read_to_string(example()).unwrap();

    // The shell, as the worker runs a stage.
    let mut s = AppState::open(&example(), &text, None).unwrap();
    let (job, _) = s
        .begin_run("alignment", Some("centreline"))
        .unwrap_or_else(|e| panic!("the shell: {e}"));
    let outcome = job.execute(&|_| {});
    assert_eq!(s.finish_run(outcome), Ok("done"));
    let from_shell = centreline_option(&s.snapshot()["document"]);

    // The CLI, on a copy of the same file.
    let dir = std::env::temp_dir().join(format!("owd-c29-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (doc, out) = (dir.join("river.owd"), dir.join("out.owd"));
    std::fs::write(&doc, &text).unwrap();
    let flags = AlignFlags {
        step: None,
        passes: None,
        chord: None,
        dem: None,
        water_level: None,
        water_slope: None,
        corridors: None,
        fairness_weight: None,
    };
    owd_cli::commands::align(
        &doc.to_string_lossy(),
        "centreline",
        flags,
        &Some(out.to_string_lossy().into_owned()),
        false,
    )
    .unwrap_or_else(|f| panic!("the CLI: {}", f.message));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let from_cli = centreline_option(&written);
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(from_shell, from_cli);
    assert_eq!(
        from_shell["line"]["coordinates"], written["inputs"]["centreline"]["coordinates"],
        "the design line is not the centreline of the document"
    );
    // The option the example already had is kept.
    assert_eq!(written["options"].as_array().unwrap().len(), 2);
}
