//! The terrain memory limit can be raised on the command line (ticket C11b): `--terrain-limit-gib`
//! on any command, with `OWD_TERRAIN_LIMIT_GIB` as its default. The option wins over the variable.
//! A file over the limit is refused, and the message names both ways to raise it. The tests set a
//! tiny limit through the variable, so no test needs gigabytes.
//!
//! Label: property

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A limit under the grid of the example's terrain (280 x 120 cells, 268,800 bytes).
const TINY_GIB: &str = "0.000001";

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

/// A folder holding the web example and its terrain file, removed at the end.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        // A number of its own for each folder: the tests run in parallel.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("owd-c11b-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::copy(web("river-example.owd"), p.join("d.owd")).unwrap();
        std::fs::copy(web("river-dem.tif"), p.join("river-dem.tif")).unwrap();
        Dir(p)
    }
    fn doc(&self) -> String {
        self.0.join("d.owd").to_string_lossy().into_owned()
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.0.join("d.owd")).unwrap()
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `owd ARGS`, with `OWD_TERRAIN_LIMIT_GIB` set to `env` or unset.
fn owd(args: &[&str], env: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_owd"));
    c.args(args).env_remove("OWD_TERRAIN_LIMIT_GIB");
    if let Some(v) = env {
        c.env("OWD_TERRAIN_LIMIT_GIB", v);
    }
    c.output().unwrap()
}

fn said(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The document `owd run` leaves with the default limit.
fn with_the_default() -> String {
    let d = Dir::new("default");
    let o = owd(&["run", &d.doc()], None);
    assert!(o.status.success(), "{}", said(&o));
    d.text()
}

#[test]
fn a_file_over_the_limit_is_refused_and_the_message_names_both_ways_to_raise_it() {
    let d = Dir::new("tiny");
    let before = d.text();
    let o = owd(&["run", &d.doc()], Some(TINY_GIB));
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    let msg = said(&o);
    assert!(msg.contains("too large"), "{msg}");
    assert!(msg.contains("--terrain-limit-gib"), "{msg}");
    assert!(msg.contains("OWD_TERRAIN_LIMIT_GIB"), "{msg}");
    assert_eq!(d.text(), before, "a refusal changes nothing");
}

#[test]
fn the_option_raises_the_limit_and_the_document_is_the_one_the_default_makes() {
    let d = Dir::new("option");
    let o = owd(
        &["run", &d.doc(), "--terrain-limit-gib", "1"],
        Some(TINY_GIB),
    );
    assert!(
        o.status.success(),
        "the option wins over the variable: {}",
        said(&o)
    );
    assert_eq!(d.text(), with_the_default());
}

#[test]
fn the_variable_raises_the_limit_on_its_own() {
    let d = Dir::new("variable");
    let o = owd(&["run", &d.doc()], Some("1"));
    assert!(o.status.success(), "{}", said(&o));
    assert_eq!(d.text(), with_the_default());
}

#[test]
fn the_option_wins_over_the_variable_both_ways() {
    let d = Dir::new("wins");
    let o = owd(
        &["run", &d.doc(), "--terrain-limit-gib", TINY_GIB],
        Some("1"),
    );
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    assert!(said(&o).contains("too large"), "{}", said(&o));
}

#[test]
fn a_limit_that_is_not_a_positive_number_is_refused_by_name() {
    let d = Dir::new("bad");
    for bad in ["abc", "0", "-1", "NaN"] {
        let o = owd(&["run", &d.doc(), "--terrain-limit-gib", bad], None);
        assert_eq!(o.status.code(), Some(2), "{bad}: {}", said(&o));
        assert!(
            said(&o).contains("terrain-limit-gib"),
            "{bad}: {}",
            said(&o)
        );
    }
    let o = owd(&["run", &d.doc()], Some("abc"));
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    assert!(said(&o).contains("OWD_TERRAIN_LIMIT_GIB"), "{}", said(&o));
}

#[test]
fn the_import_of_a_terrain_file_takes_the_limit_too() {
    let d = Dir::new("import");
    let doc = d.0.join("new.owd").to_string_lossy().into_owned();
    let dem = d.0.join("river-dem.tif").to_string_lossy().into_owned();
    let o = owd(&["new", &doc, "--title", "T", "--date", "2026-01-02"], None);
    assert!(o.status.success(), "{}", said(&o));
    let fresh = std::fs::read_to_string(&doc).unwrap();
    let o = owd(&["import", &doc, "--dem", &dem], Some(TINY_GIB));
    assert_eq!(o.status.code(), Some(2), "{}", said(&o));
    assert!(said(&o).contains("--terrain-limit-gib"), "{}", said(&o));
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), fresh);
    let o = owd(
        &["import", &doc, "--dem", &dem, "--terrain-limit-gib", "1"],
        Some(TINY_GIB),
    );
    assert!(o.status.success(), "{}", said(&o));
    let raised = std::fs::read_to_string(&doc).unwrap();
    std::fs::write(&doc, &fresh).unwrap();
    let o = owd(&["import", &doc, "--dem", &dem], None);
    assert!(o.status.success(), "{}", said(&o));
    assert_eq!(raised, std::fs::read_to_string(&doc).unwrap());
}
