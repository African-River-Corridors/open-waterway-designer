//! `owd align --use-water <true|false>` (ticket #488): the flag sets the rolling circle's
//! `use_water` for the run and stores it in the alignment settings; with no flag the stored
//! setting stands; `owd run` honours it; a bad value is refused by name.
//!
//! The river is two closed-form sine banks (see `owd-format/tests/use_water_default.rs`): the
//! import builds the polygon, so the default is water-aware and `false` gives another line.
//!
//! Label: property

use owd_format::design::{Role, Session, Stage};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn sine(off: f64) -> Vec<Pt> {
    (0..=600)
        .map(|i| {
            let x = 10.0 * f64::from(i);
            let y = 600.0 * (2.0 * std::f64::consts::PI * x / 1200.0).sin() + off;
            [500_000.0 + x, 600_000.0 + y]
        })
        .collect()
}

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// A sized design of the two banks, written to `name.owd` in a fresh folder.
fn design(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-use-water-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (left, right) = (line_text(&sine(75.0)), line_text(&sine(-75.0)));
    let mut s = Session::new("Use water", "2026-11-10");
    s.import(&[(Role::LeftBank, &left), (Role::RightBank, &right)])
        .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let path = dir.join("d.owd");
    std::fs::write(&path, s.to_text()).unwrap();
    path
}

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

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn stored(d: &Value) -> &Value {
    &d["stages"]["alignment"]["settings"]["rolling_circle"]["use_water"]
}

fn used(d: &Value) -> &Value {
    &d["options"][0]["settings"]["use_water"]
}

fn align(path: &Path, extra: &[&str]) -> Output {
    let p = path.to_string_lossy();
    let mut args = vec!["align", &p, "--method", "rolling-circle"];
    args.extend(extra);
    owd(&args)
}

#[test]
fn the_flag_sets_and_stores_the_setting_and_run_keeps_it() {
    let path = design("stored");
    let o = align(&path, &["--use-water", "false"]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let off = read(&path);
    assert_eq!(stored(&off), &json!(false));
    assert_eq!(used(&off), &json!(false));

    // No flag: the stored setting stands.
    let o = align(&path, &[]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let again = read(&path);
    assert_eq!(stored(&again), &json!(false));
    assert_eq!(again["options"][0]["line"], off["options"][0]["line"]);

    // `owd run` after a change of sizing reruns the alignment with the stored setting.
    let mut d = read(&path);
    d["stages"]["sizing"]["settings"]["draught_m"] = json!(2.4);
    std::fs::write(&path, d.to_string()).unwrap();
    let p = path.to_string_lossy();
    let o = owd(&["run", &p]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let ran = read(&path);
    assert_ne!(
        ran["stages"]["alignment"]["depends_on"], again["stages"]["alignment"]["depends_on"],
        "`owd run` did not rerun the alignment"
    );
    assert_eq!(stored(&ran), &json!(false));
    assert_eq!(used(&ran), &json!(false));
}

#[test]
fn the_flag_true_and_the_default_give_the_same_line_here() {
    let (a, b) = (design("true"), design("default"));
    let o = align(&a, &["--use-water", "true"]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let o = align(&b, &[]);
    assert!(code(&o) == 0 || code(&o) == 1, "{}", err(&o));
    let (on, default) = (read(&a), read(&b));
    assert_eq!(stored(&on), &json!(true));
    assert_eq!(stored(&default), &Value::Null);
    assert_eq!(used(&default), &json!(true));
    assert_eq!(on["options"][0]["line"], default["options"][0]["line"]);
}

#[test]
fn a_bad_value_is_refused_by_name() {
    let path = design("bad");
    let before = std::fs::read_to_string(&path).unwrap();
    let o = align(&path, &["--use-water", "maybe"]);
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(err(&o).contains("--use-water"), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}
