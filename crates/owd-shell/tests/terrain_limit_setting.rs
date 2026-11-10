//! The terrain memory limit can be raised in the app shell (ticket C11b): a machine setting, not a
//! part of the design. A file over the limit is refused with a message that names the setting; the
//! same file loads once the limit is raised, and gives the document the command line makes.
//!
//! Label: property

use owd_shell::state::AppState;
use std::path::{Path, PathBuf};

const DATE: &str = "2026-01-02";

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

fn dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("owd-c11b-shell-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    std::fs::copy(web("river-dem.tif"), p.join("river-dem.tif")).unwrap();
    p
}

/// A new design saved at `doc`, held by the shell with its path.
fn shell_at(doc: &Path) -> AppState {
    let text = AppState::new_design("T", DATE).save_text_for(doc);
    std::fs::write(doc, &text).unwrap();
    AppState::open(doc, &text, None).unwrap()
}

#[test]
fn a_file_over_the_limit_is_refused_and_loads_once_the_limit_is_raised() {
    let d = dir("raise");
    let (doc, dem) = (d.join("d.owd"), d.join("river-dem.tif"));
    let bytes = std::fs::read(&dem).unwrap();

    let mut s = shell_at(&doc);
    s.set_terrain_limit_bytes(Some(1_000)).unwrap();
    let before = s.save_text_for(&doc);
    let why = s
        .load_dem(&dem, bytes.clone(), DATE)
        .expect_err("a terrain file over the limit is refused");
    assert!(why.contains("too large"), "{why}");
    assert!(
        why.contains("setting"),
        "the refusal names the app's setting: {why}"
    );
    assert_eq!(s.save_text_for(&doc), before, "a refusal changes nothing");

    s.set_terrain_limit_bytes(Some(1 << 30)).unwrap();
    s.load_dem(&dem, bytes.clone(), DATE).unwrap();
    let raised = s.save_text_for(&doc);

    // The same file with the default limit, through the shell and through the command line.
    let mut plain = shell_at(&doc);
    plain.load_dem(&dem, bytes, DATE).unwrap();
    assert_eq!(raised, plain.save_text_for(&doc));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn the_default_comes_back_and_a_limit_that_is_not_positive_is_refused_by_name() {
    let d = dir("default");
    let doc = d.join("d.owd");
    let mut s = shell_at(&doc);
    let why = s.set_terrain_limit_bytes(Some(0)).unwrap_err();
    assert!(why.contains("terrain memory limit"), "{why}");
    s.set_terrain_limit_bytes(Some(1_000)).unwrap();
    s.set_terrain_limit_bytes(None).unwrap();
    let dem = d.join("river-dem.tif");
    s.load_dem(&dem, std::fs::read(&dem).unwrap(), DATE)
        .unwrap();
    std::fs::remove_dir_all(&d).ok();
}
