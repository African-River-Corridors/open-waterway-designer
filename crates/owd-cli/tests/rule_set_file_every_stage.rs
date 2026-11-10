//! A design sized with a Rule Set file runs its later stages (ticket #491). `--rule-set-file` is a
//! global option: every command that opens a design loads it. Without the file, or with a changed
//! one, each later command says so plainly, never "sizing is stale".
//!
//! The river is the repository's synthetic river (2.4 km, as the web fixtures), on the committed
//! synthetic terrain `examples/web/river-dem.tif`. The Rule Set file is the built-in PIANC set with
//! a wider bottom (k 3.6 for 3.2) at a version that is not built in.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SHIFT: Pt = [500_000.0, 600_000.0];
const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SET: &str = "pianc-wg141@0.9.0";

fn owd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .args(args)
        .output()
        .expect("owd runs")
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read(doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(doc).unwrap()).unwrap()
}

/// The Rule Set file's text: the built-in PIANC set, its bottom factor `k` in place of 3.2, at
/// version 0.9.0.
fn rule_set(k: &str) -> String {
    let text = owd_core::rules::engine::BUILT_IN[0]
        .1
        .replace("\"k\": 3.2,", &format!("\"k\": {k},"))
        .replace("\"version\": \"0.3.0\"", "\"version\": \"0.9.0\"");
    assert!(
        text.contains(&format!("\"k\": {k},")),
        "the bottom factor was replaced"
    );
    text
}

struct Case {
    dir: PathBuf,
    doc: String,
    file: String,
}

/// `new` and `import` (the synthetic river and its terrain) in a fresh folder, and the Rule Set file.
fn case(name: &str) -> Case {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-rsf-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let river = synthetic_river(RiverParams {
        length_m: 2400.0,
        ..RiverParams::default()
    })
    .unwrap();
    let shift = |p: &Pt| [p[0] + SHIFT[0], p[1] + SHIFT[1]];
    let centre: Vec<Pt> = river.centreline.iter().map(shift).collect();
    let ring: Vec<Pt> = river.polygon.exterior.iter().map(shift).collect();
    let (c, p) = (dir.join("centreline.geojson"), dir.join("polygon.geojson"));
    std::fs::write(
        &c,
        format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(centre)
        ),
    )
    .unwrap();
    std::fs::write(
        &p,
        format!(
            r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
            json!([ring])
        ),
    )
    .unwrap();
    let dem = dir.join("river-dem.tif");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-dem.tif"),
        &dem,
    )
    .unwrap();
    let doc = dir.join("river.owd").to_string_lossy().into_owned();
    let file = dir.join("mine.json");
    std::fs::write(&file, rule_set("3.6")).unwrap();
    for args in [
        vec!["new", &doc, "--title", "River", "--date", "2026-11-10"],
        vec![
            "import",
            &doc,
            "--centreline",
            c.to_str().unwrap(),
            "--polygon",
            p.to_str().unwrap(),
            "--dem",
            dem.to_str().unwrap(),
        ],
    ] {
        let o = owd(&args);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
    }
    Case {
        dir,
        doc,
        file: file.to_string_lossy().into_owned(),
    }
}

/// `owd size` with the Rule Set file.
fn size_with_file(c: &Case) {
    let o = owd(&[
        "size",
        &c.doc,
        "--rule-set-file",
        &c.file,
        "--rule-set",
        SET,
        "--set-input",
        "se_quality=B",
        "--vessel",
        "CEMT_IV",
        "--draught",
        "2.5",
    ]);
    assert_eq!(code(&o), 0, "size: {}", err(&o));
}

/// The later commands: align, plan-cut, volumes, run.
fn later(doc: &str) -> Vec<Vec<String>> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        v(&["align", doc, "--method", "rolling-circle"]),
        v(&["plan-cut", doc]),
        v(&[
            "volumes",
            doc,
            "--water-level",
            "0.0",
            "--water-slope",
            "0.0",
        ]),
        v(&["run", doc]),
    ]
}

fn with_file(args: &[String], file: &str) -> Vec<String> {
    let mut a = args.to_vec();
    a.extend(["--rule-set-file".to_string(), file.to_string()]);
    a
}

fn owd_s(args: &[String]) -> Output {
    owd(&args.iter().map(String::as_str).collect::<Vec<_>>())
}

/// The content hash the design recorded for the file's set.
fn recorded_hash(doc: &str) -> String {
    let d = read(doc);
    d["stages"]["sizing"]["settings"]["rule_sets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| {
            format!(
                "{}@{}",
                s["id"].as_str().unwrap(),
                s["version"].as_str().unwrap()
            ) == SET
        })
        .expect("the design records the file's set")["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn every_later_stage_runs_with_the_file() {
    let c = case("runs");
    size_with_file(&c);
    let d = read(&c.doc);
    let s = &d["stages"]["sizing"]["results"];
    assert_eq!(
        s["bottom_width_straight_m"].as_f64().unwrap(),
        3.6 * s["beam_m"].as_f64().unwrap(),
        "the sized numbers are the file's"
    );
    for args in later(&c.doc) {
        let o = owd_s(&with_file(&args, &c.file));
        assert!(code(&o) == 0 || code(&o) == 1, "{args:?}: {}", err(&o));
        assert!(!err(&o).contains("is stale"), "{args:?}: {}", err(&o));
    }
    let d = read(&c.doc);
    for stage in ["alignment", "plan_cut", "volumes"] {
        assert!(
            !d["stages"][stage]["results"].is_null(),
            "{stage} did not run"
        );
    }
    let o = owd(&["run", &c.doc, "--rule-set-file", &c.file, "--json"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let ran: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        ran["ran"],
        json!([]),
        "nothing is stale with the file loaded"
    );
    let _ = &c.dir;
}

#[test]
fn without_the_file_each_later_command_names_the_set_and_the_option() {
    let c = case("missing");
    size_with_file(&c);
    let hash = recorded_hash(&c.doc);
    let before = std::fs::read_to_string(&c.doc).unwrap();
    for args in later(&c.doc) {
        let o = owd_s(&args);
        let e = err(&o);
        assert_ne!(code(&o), 0, "{args:?} ran without the file");
        assert!(e.contains(SET), "{args:?}: {e}");
        assert!(e.contains(&hash), "{args:?}: {e}");
        assert!(e.contains("--rule-set-file"), "{args:?}: {e}");
        assert!(!e.contains("is stale"), "{args:?}: {e}");
        assert_eq!(std::fs::read_to_string(&c.doc).unwrap(), before);
    }
}

#[test]
fn a_changed_file_is_named_with_both_hashes() {
    let c = case("changed");
    size_with_file(&c);
    let old = recorded_hash(&c.doc);
    let changed = c.dir.join("changed.json");
    std::fs::write(&changed, rule_set("3.7")).unwrap();
    let f = changed.to_string_lossy().into_owned();
    for args in later(&c.doc) {
        let o = owd_s(&with_file(&args, &f));
        let e = err(&o);
        assert_ne!(code(&o), 0, "{args:?} ran with a changed file");
        assert!(e.contains(SET), "{args:?}: {e}");
        assert!(
            e.contains("has changed since the design was sized"),
            "{args:?}: {e}"
        );
        assert!(e.contains(&old), "{args:?}: {e}");
        // The loaded file's hash: another long hex word in the message.
        let other = e
            .split(|ch: char| !ch.is_ascii_hexdigit())
            .any(|w| w.len() >= 16 && w != old);
        assert!(other, "{args:?}: the new hash is not named: {e}");
        assert!(!e.contains("is stale"), "{args:?}: {e}");
    }
}

#[test]
fn a_design_with_built_in_sets_is_unchanged_with_and_without_the_option() {
    let (a, b) = (case("builtin-a"), case("builtin-b"));
    for c in [&a, &b] {
        let o = owd(&[
            "size",
            &c.doc,
            "--rule-set",
            "pianc-wg141",
            "--set-input",
            "se_quality=B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ]);
        assert_eq!(code(&o), 0, "{}", err(&o));
    }
    let (ra, rb) = (
        owd(&["align", &a.doc, "--method", "rolling-circle"]),
        owd(&[
            "align",
            &b.doc,
            "--method",
            "rolling-circle",
            "--rule-set-file",
            &b.file,
        ]),
    );
    assert_eq!(code(&ra), code(&rb), "{} | {}", err(&ra), err(&rb));
    let (mut da, mut db) = (read(&a.doc), read(&b.doc));
    // Only the folder differs: the inputs' file names are relative.
    for d in [&mut da, &mut db] {
        d["provenance"] = Value::Null;
    }
    assert_eq!(da["stages"], db["stages"]);
    assert_eq!(da["options"], db["options"]);
}

#[test]
fn the_option_is_taken_by_commands_that_open_a_design() {
    let c = case("global");
    size_with_file(&c);
    for args in [
        vec!["status", c.doc.as_str()],
        vec!["validate", c.doc.as_str()],
    ] {
        let mut a = args.clone();
        a.extend(["--rule-set-file", c.file.as_str()]);
        let o = owd(&a);
        assert!(code(&o) == 0 || code(&o) == 1, "{a:?}: {}", err(&o));
        assert!(
            !err(&o).contains("unexpected argument"),
            "{a:?}: {}",
            err(&o)
        );
    }
}
