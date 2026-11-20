//! Old files keep their work, on the command line (the owner's rule of 20 November 2026): a format
//! 0.4 file whose stages were up to date when it was saved opens up to date, `owd volumes` runs on it
//! without touching its line and gives its stored numbers again, and the drawings then read the
//! stations. With no water level, the DEM beside the file says which end is the mouth.
//!
//! The fixtures are `owd-format`'s `tests/data/format-0.4-*.owd` (see `owd-format/tests/old_files.rs`
//! for how they were made), with the web example's DEM beside them.
//!
//! Label: property

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A folder holding the fixture `name` (edited by `edit`) as `old.owd`, with the DEM beside it.
fn work(name: &str, edit: impl Fn(&mut Value)) -> (PathBuf, Value) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-old-files-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = std::fs::read_to_string(
        root().join(format!("crates/owd-format/tests/data/format-0.4-{name}.owd")),
    )
    .unwrap();
    let mut doc: Value = serde_json::from_str(&text).unwrap();
    edit(&mut doc);
    std::fs::write(dir.join("old.owd"), doc.to_string()).unwrap();
    std::fs::copy(
        root().join("examples/web/river-dem.tif"),
        dir.join("river-dem.tif"),
    )
    .unwrap();
    (dir.join("old.owd"), doc)
}

fn read(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn a_fresh_old_file_opens_fresh_and_its_volumes_run_again_with_the_same_numbers() {
    let (path, before) = work("rolling-circle", |_| {});
    let doc = path.to_str().unwrap();
    let st = run(&["status", doc]);
    assert_eq!(code(&st), 0, "{}", err(&st));
    let status = out(&st);
    assert!(status.contains("alignment: fresh"), "{status}");
    assert!(status.contains("volumes: fresh"), "{status}");
    assert!(err(&st).contains("kept fresh"), "{}", err(&st));

    let v = run(&["volumes", doc]);
    assert_eq!(code(&v), 0, "{}", err(&v));
    let after = read(&path);
    assert_eq!(after["format_version"], "0.5");
    let rc = before["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "rolling_circle")
        .unwrap();
    assert_eq!(
        after["stages"]["alignment"]["results"]["line"], rc["line"],
        "the volumes run does not touch the line"
    );
    let then = &before["stages"]["volumes"]["results"]["options"][0];
    let now = &after["stages"]["volumes"]["results"];
    for k in ["dredge_m3", "excav_m3", "total_m3", "max_face_h_m"] {
        let (a, b) = (now[k].as_f64().unwrap(), then[k].as_f64().unwrap());
        assert!((a - b).abs() <= 1e-9 * b.abs(), "{k}: {a} now, {b} stored");
    }
    assert!(now["sections"].is_object(), "the stations are stored");

    // The drawings read them: the long section and the cross sections.
    let d = run(&["draw", doc, "--document"]);
    assert_eq!(code(&d), 0, "{}", err(&d));
    let sheets: Value = serde_json::from_str(&out(&d)).unwrap();
    let names: Vec<&String> = sheets.as_object().unwrap().keys().collect();
    assert!(names.iter().any(|n| n.contains("-XS-")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("-LS-")), "{names:?}");
}

#[test]
fn with_no_water_level_the_dem_beside_the_file_says_which_end_is_the_mouth() {
    let (path, _) = work("rolling-circle", |d| d["inputs"]["water_level"] = Value::Null);
    let st = run(&["status", path.to_str().unwrap()]);
    assert_eq!(code(&st), 0, "{}", err(&st));
    let e = err(&st);
    assert!(e.contains("elevation model"), "{e}");
    assert!(!e.contains("state the mouth"), "{e}");
}
