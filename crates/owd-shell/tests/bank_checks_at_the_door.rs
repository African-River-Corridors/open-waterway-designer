//! The bank checks run at the door. Every import of banks, on the command line and in the app
//! shell, checks them (each bank on its own side, a plausible width) and reports what fails, with
//! the same words on both. The checks do not repair the banks and do not refuse them.
//!
//! Label: property

use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A gently winding line 3 km long, in UTM 30N, running east, `y` metres north of the river axis,
/// with a vertex every 50 m.
fn east(y: f64) -> Vec<Pt> {
    (0..=60)
        .map(|i| {
            let x = 50.0 * f64::from(i);
            [500_000.0 + x, 600_000.0 + y + 40.0 * (x / 500.0).sin()]
        })
        .collect()
}

/// The north bank with a throat between 1.3 km and 1.7 km: it comes in by up to 140 m, so the
/// river narrows from 200 m to 60 m. The banks do not cross.
fn throat() -> Vec<Pt> {
    let mut bank = east(100.0);
    for (i, p) in bank.iter_mut().enumerate() {
        let x = 50.0 * i as f64;
        p[1] -= 140.0 * (1.0 - ((x - 1500.0).abs() / 200.0).min(1.0));
    }
    bank
}

fn text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-bank-door-{tag}-{}", std::process::id()));
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

fn strings(v: &Value, what: &str) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("{what} is not a list: {v}"))
        .iter()
        .map(|w| w.as_str().expect("a warning is a line of text").to_string())
        .collect()
}

const CHILD: &str = "OWD_BANK_DOOR_IMPORT";

/// `owd import --json` writes its result to the standard output. To read it, this test file runs
/// itself again with only this test, which then does the import. Run as a plain test it does nothing.
#[test]
fn the_command_line_import_itself() {
    let Ok(job) = std::env::var(CHILD) else {
        return;
    };
    let job: Value = serde_json::from_str(&job).unwrap();
    let path = |k: &str| job[k].as_str().map(str::to_string);
    let done = owd_cli::commands::import(
        job["doc"].as_str().unwrap(),
        [
            ("left_bank", path("left")),
            ("right_bank", path("right")),
            ("centreline", None),
            ("river_polygon", None),
        ],
        None,
        owd_cli::commands::WaterFlags::default(),
        &None,
        true,
    );
    if done.is_err() {
        panic!("`owd import` refused the banks");
    }
}

/// The two banks through `owd import --json`: the warnings of its output, and the document.
fn through_the_command_line(tag: &str, left: &[Pt], right: &[Pt]) -> (Vec<String>, Value) {
    let dir = Dir::new(tag);
    let file = |name: &str| dir.0.join(name).to_string_lossy().into_owned();
    let (doc, l, r) = (file("d.owd"), file("left.geojson"), file("right.geojson"));
    std::fs::write(&l, text(left)).unwrap();
    std::fs::write(&r, text(right)).unwrap();
    if owd_cli::commands::new(&doc, "Bank checks", Some("2026-11-10"), true).is_err() {
        panic!("`owd new` failed");
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "the_command_line_import_itself", "--nocapture"])
        .env(
            CHILD,
            json!({"doc": doc, "left": l, "right": r}).to_string(),
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "`owd import` must not refuse these banks:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The result is one line of JSON; the test runner may print before it on the same line.
    let result: Value = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(&line[line.find('{')?..]).ok())
        .find(|v| v.get("project_epsg").is_some())
        .unwrap_or_else(|| panic!("`owd import --json` printed no result:\n{stdout}"));
    assert!(
        result.get("warnings").is_some(),
        "the result of `owd import --json` has no `warnings` list: {result}"
    );
    let saved = serde_json::from_str(&std::fs::read_to_string(Path::new(&doc)).unwrap()).unwrap();
    (strings(&result["warnings"], "`warnings`"), saved)
}

/// The two banks through the app shell: the warnings of its snapshot, and the document.
fn through_the_shell(left: &[Pt], right: &[Pt]) -> (Vec<String>, Value) {
    let mut s = AppState::new_design("Bank checks", "2026-11-10");
    s.load_inputs(
        &[(Role::LeftBank, text(left)), (Role::RightBank, text(right))],
        "2026-11-10",
    )
    .expect("the shell must not refuse these banks");
    let snap = s.snapshot();
    (
        strings(&snap["warnings"], "the snapshot's `warnings`"),
        snap["document"].clone(),
    )
}

/// Both surfaces report the same warnings, one of them names `check`, and the banks are kept.
fn reported_on_both(tag: &str, left: &[Pt], right: &[Pt], check: &str) {
    let (shell, shell_doc) = through_the_shell(left, right);
    let (cli, cli_doc) = through_the_command_line(tag, left, right);
    assert!(
        shell.iter().any(|w| w.contains(check)),
        "the shell reports no `{check}` warning: {shell:?}"
    );
    assert!(
        cli.iter().any(|w| w.contains(check)),
        "`owd import` reports no `{check}` warning: {cli:?}"
    );
    assert_eq!(shell, cli, "the two surfaces must use the same words");
    // The checks report. They do not repair and do not refuse: both banks are in the document,
    // with every vertex.
    for doc in [&shell_doc, &cli_doc] {
        for (role, n) in [("left_bank", left.len()), ("right_bank", right.len())] {
            assert_eq!(
                doc["inputs"][role]["coordinates"].as_array().map(Vec::len),
                Some(n)
            );
        }
    }
}

#[test]
fn a_river_that_narrows_to_a_third_is_reported_on_both_surfaces_with_the_same_words() {
    reported_on_both("narrow", &throat(), &east(-100.0), "width-narrow");
}

#[test]
fn banks_named_the_wrong_way_round_are_reported_on_both_surfaces_with_the_same_words() {
    // No centreline says which way the river flows, so import keeps the names it was given. The
    // side check then fails all along: looking along the banks, the "left" one is on the right.
    reported_on_both("side", &east(-100.0), &east(100.0), "side");
}

#[test]
fn a_sound_pair_of_banks_gives_no_warning_on_either_surface() {
    let (left, right) = (east(100.0), east(-100.0));
    let (cli, _) = through_the_command_line("sound", &left, &right);
    let (shell, _) = through_the_shell(&left, &right);
    assert_eq!(cli, Vec::<String>::new());
    assert_eq!(shell, Vec::<String>::new());
}
