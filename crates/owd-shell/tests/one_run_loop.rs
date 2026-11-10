//! One "run what is due" loop behind every surface.
//!
//! The command line and the app shell must run the same stages, on the same terrain, and leave the
//! same document. No surface may decide for itself which stages are due.
//!
//! Label: property

use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

fn repo(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-one-run-loop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The document `owd run` leaves, and the document the shell's run leaves, for the same text and
/// the same terrain file.
fn both(tag: &str, text: &str) -> (Value, Value, Vec<&'static str>) {
    let dir = Dir::new(tag);
    let doc = dir.0.join("d.owd");
    std::fs::write(&doc, text).unwrap();
    std::fs::copy(web("river-dem.tif"), dir.0.join("river-dem.tif")).unwrap();
    let dem = std::fs::read(web("river-dem.tif")).unwrap();

    // The app shell, first: it reads the same text and the same terrain bytes.
    let mut shell = AppState::open(&doc, text, Some(&dem)).unwrap();
    let ran = shell
        .run_stale()
        .expect("the shell runs every stage that is due, on the document's terrain");
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    // The command line, on the file.
    if owd_cli::commands::run(&doc.to_string_lossy(), &None, true).is_err() {
        panic!("`owd run` failed on {tag}");
    }
    let from_cli: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    (from_cli, from_shell, ran)
}

#[test]
fn stale_stages_that_need_terrain_run_the_same_through_the_shell_and_the_command_line() {
    for name in ["river-reroute.owd", "river-rolling-circle.owd"] {
        let text = std::fs::read_to_string(web(name)).unwrap();
        let (cli, shell, ran) = both(name, &text);
        assert!(
            ran.contains(&"volumes"),
            "{name}: the shell ran {ran:?}; volumes was stale and needs the terrain"
        );
        assert_eq!(
            shell, cli,
            "{name}: the two surfaces left different documents"
        );
    }
}

#[test]
fn a_stage_with_settings_and_no_results_is_due_on_both_surfaces() {
    // The example with its volumes results taken away: the stage has settings, and has not run.
    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(web("river-example.owd")).unwrap()).unwrap();
    assert!(doc["stages"]["volumes"]["results"].is_object());
    doc["stages"]["volumes"]["results"] = Value::Null;
    let text = serde_json::to_string_pretty(&doc).unwrap();

    let (cli, shell, ran) = both("not-run", &text);
    assert_eq!(ran, vec!["volumes"]);
    assert!(shell["stages"]["volumes"]["results"].is_object());
    assert_eq!(shell, cli);
}

#[test]
fn no_surface_holds_its_own_loop_over_the_stage_statuses() {
    // The loop lives in one place. A surface asks for "run what is due"; it does not read each
    // stage's status to decide.
    for (file, loop_text) in [
        ("app/src/App.svelte", r#"s.status !== "stale""#),
        ("tools/web_parity.mjs", r#"st.status === "stale""#),
    ] {
        assert!(
            !repo(file).contains(loop_text),
            "{file} still decides which stages are due (`{loop_text}`)"
        );
    }
}
