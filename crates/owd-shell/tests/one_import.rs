//! One import behind every surface.
//!
//! The same files, imported through the command line and through the app shell, must give the same
//! document: the same inputs and the same record of where each input came from.
//!
//! Label: property

use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-one-import-{tag}-{}", std::process::id()));
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

/// A new design in `dir`, with the two example banks and the example terrain beside it.
fn new_design(dir: &Dir) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let doc = dir.0.join("d.owd");
    let (left, right, dem) = (
        dir.0.join("left.geojson"),
        dir.0.join("right.geojson"),
        dir.0.join("river-dem.tif"),
    );
    std::fs::copy(web("river-bank-left.geojson"), &left).unwrap();
    std::fs::copy(web("river-bank-right.geojson"), &right).unwrap();
    std::fs::copy(web("river-dem.tif"), &dem).unwrap();
    if owd_cli::commands::new(
        &doc.to_string_lossy(),
        "One import",
        Some("2026-01-02"),
        true,
    )
    .is_err()
    {
        panic!("`owd new` failed");
    }
    (doc, left, right, dem)
}

fn cli_import(doc: &Path, left: &Path, right: &Path, dem: Option<&Path>) {
    let s = |p: &Path| Some(p.to_string_lossy().into_owned());
    let done = owd_cli::commands::import(
        &doc.to_string_lossy(),
        [
            ("left_bank", s(left)),
            ("right_bank", s(right)),
            ("centreline", None),
            ("river_polygon", None),
        ],
        dem.and_then(s),
        owd_cli::commands::WaterFlags::default(),
        &None,
        true,
    );
    if done.is_err() {
        panic!("`owd import` failed");
    }
}

#[test]
fn the_command_line_records_where_each_imported_input_came_from() {
    let dir = Dir::new("provenance");
    let (doc, left, right, dem) = new_design(&dir);
    cli_import(&doc, &left, &right, Some(&dem));
    let d = read(&doc);
    for input in ["left_bank", "right_bank", "dem"] {
        assert!(
            d["provenance"][input].is_object(),
            "`owd import` left no provenance record for `{input}`"
        );
    }
}

#[test]
fn the_same_files_give_the_same_document_through_the_shell_and_the_command_line() {
    let dir = Dir::new("equal");
    let (doc, left, right, dem) = new_design(&dir);
    let new_text = std::fs::read_to_string(&doc).unwrap();

    cli_import(&doc, &left, &right, Some(&dem));
    let from_cli = read(&doc);
    // Both surfaces date a record with the day of the import. Read the day the command line
    // used, so the test does not depend on the clock.
    let date = from_cli["provenance"]["dem"]["date"]
        .as_str()
        .expect("`owd import --dem` dates its provenance record")
        .to_string();

    let mut shell = AppState::open(&doc, &new_text, None).unwrap();
    shell
        .load_inputs(
            &[
                (Role::LeftBank, std::fs::read_to_string(&left).unwrap()),
                (Role::RightBank, std::fs::read_to_string(&right).unwrap()),
            ],
            &date,
        )
        .unwrap();
    shell
        .load_dem(&dem, std::fs::read(&dem).unwrap(), &date)
        .unwrap();
    let from_shell: Value = serde_json::from_str(&shell.save_text_for(&doc)).unwrap();

    assert_eq!(from_shell["inputs"], from_cli["inputs"]);
    assert_eq!(from_shell["provenance"], from_cli["provenance"]);
    assert_eq!(from_shell, from_cli);
}
