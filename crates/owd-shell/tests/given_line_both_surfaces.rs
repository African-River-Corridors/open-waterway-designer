//! A given design line gives the same document through the command line and through the app shell,
//! before and after the volumes stage runs on it.
//!
//! Both start from the web example and its terrain file. The command line reads the line from a
//! file (`owd design-line DOC --line FILE --name NAME`); the shell takes the same text.
//!
//! Label: property

use owd_cli::commands;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const NAME: &str = "Straight across";

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-given-line-{tag}-{}", std::process::id()));
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

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn ok(step: &str, done: commands::Result) {
    if let Err(f) = done {
        panic!("`owd {step}` failed: {}", f.message);
    }
}

/// A straight line between two points of the example's river centreline, as GeoJSON in the
/// example's CRS (UTM 30N).
fn line_text(example: &Value) -> String {
    let centre = example["inputs"]["centreline"]["coordinates"]
        .as_array()
        .unwrap();
    let pts = json!([centre[centre.len() / 4], centre[centre.len() / 2]]);
    format!(
        r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{pts}}}"#
    )
}

fn shell_volumes(s: &mut AppState) {
    let (job, _) = s.begin_run("volumes", None).unwrap();
    let outcome = job.execute(&|_| {});
    s.finish_run(outcome).unwrap();
}

#[test]
fn a_given_line_gives_equal_documents_through_the_shell_and_the_command_line() {
    let dir = Dir::new("equal");
    let doc = dir.0.join("d.owd");
    let dem = dir.0.join("river-dem.tif");
    let line = dir.0.join("line.geojson");
    std::fs::copy(web("river-example.owd"), &doc).unwrap();
    std::fs::copy(web("river-dem.tif"), &dem).unwrap();
    let example_text = std::fs::read_to_string(&doc).unwrap();
    let text = line_text(&read(&doc));
    std::fs::write(&line, &text).unwrap();
    let dem_bytes = std::fs::read(&dem).unwrap();

    // The command line.
    ok(
        "design-line",
        owd_cli::design_line::design_line(
            &doc.to_string_lossy(),
            &line.to_string_lossy(),
            NAME,
            &None,
            true,
        ),
    );
    let from_cli = read(&doc);
    let options = from_cli["options"].as_array().unwrap();
    assert_eq!(options.len(), 2, "the example's option and the given line");
    assert_eq!(options[1]["method"], "given");
    assert_eq!(options[1]["name"], NAME);
    assert_eq!(options[1]["checks"]["radius_ok"], json!(true));
    let id = options[1]["id"].as_str().unwrap().to_string();

    // The shell.
    let mut shell = AppState::open(&doc, &example_text, Some(&dem_bytes)).unwrap();
    assert_eq!(shell.load_design_line(&text, NAME).unwrap(), id);
    assert!(shell.is_dirty());
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();
    assert_eq!(from_shell["options"], from_cli["options"]);
    assert_eq!(from_shell, from_cli);

    // The volumes stage is now stale on both. Each surface runs it on the document's terrain.
    ok(
        "run",
        commands::run(&doc.to_string_lossy(), &[], &None, true),
    );
    let from_cli = read(&doc);
    shell_volumes(&mut shell);
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();
    let rows = from_cli["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["id"], json!(id));
    assert!(rows[1]["total_m3"].as_f64().unwrap() > 0.0);
    assert_eq!(from_shell, from_cli);
}

#[test]
fn a_refused_line_changes_nothing_on_either_surface() {
    let dir = Dir::new("refused");
    let doc = dir.0.join("d.owd");
    let line = dir.0.join("line.geojson");
    std::fs::copy(web("river-example.owd"), &doc).unwrap();
    let example_text = std::fs::read_to_string(&doc).unwrap();
    let before = read(&doc);
    // A point is not a line.
    let text = r#"{"type":"Point","coordinates":[0.5,0.5]}"#;
    std::fs::write(&line, text).unwrap();

    let refused = owd_cli::design_line::design_line(
        &doc.to_string_lossy(),
        &line.to_string_lossy(),
        NAME,
        &None,
        true,
    )
    .expect_err("the command line takes a point as a design line");
    assert!(
        refused.message.contains("LineString"),
        "{}",
        refused.message
    );
    assert_eq!(read(&doc), before);

    let mut shell = AppState::open(&doc, &example_text, None).unwrap();
    // Opening brings an older document up to date, so compare with the shell's own text.
    let opened = shell.save_text_for(&doc);
    let e = shell
        .load_design_line(text, NAME)
        .expect_err("the shell takes a point as a design line");
    assert!(e.contains("LineString"), "{e}");
    assert!(!shell.is_dirty());
    assert_eq!(shell.save_text_for(&doc), opened);
}
