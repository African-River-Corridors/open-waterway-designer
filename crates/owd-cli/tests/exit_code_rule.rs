//! One exit-code rule (ticket A13): a command that saved the design exits 0, whether or not an
//! option fails a check; the failed checks are on stderr ("the option fails: …"), in the document
//! and in `--json`. The global `--strict` makes a failed check exit code 1, after the save. A
//! command that cannot run keeps its own non-zero code.
//!
//! The failing design is the repository's synthetic river (`examples/web`) sized for a vessel too
//! large for its bends; the passing one is the flat terrain twin (`tests/alignment_cases`).
//!
//! Label: property

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn owd_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_owd"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("owd runs")
}

fn said(o: &Output) -> String {
    format!(
        "code {:?}\nstdout: {}\nstderr: {}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn work_dir(tag: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-exit-rule-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The synthetic river, re-run, then sized for CEMT Va: every option fails a check.
fn failing_design(tag: &str) -> PathBuf {
    let dir = work_dir(tag);
    let web = root().join("examples/web");
    for f in ["river-rolling-circle.owd", "river-dem.tif"] {
        std::fs::copy(web.join(f), dir.join(f)).unwrap();
    }
    std::fs::rename(dir.join("river-rolling-circle.owd"), dir.join("base.owd")).unwrap();
    // The stages are made fresh for this build; the code does not matter here.
    owd_in(&dir, &["run", "base.owd"]);
    let o = owd_in(&dir, &["size", "base.owd", "--vessel", "CEMT_Va"]);
    assert!(o.status.success(), "{}", said(&o));
    dir
}

/// The flat terrain twin, sized and aligned: its option passes every check.
fn passing_design(tag: &str) -> PathBuf {
    let dir = work_dir(tag);
    let inputs = root().join("tests/alignment_cases/terrain-flat/inputs");
    for f in ["left.geojson", "right.geojson", "river.geojson", "flat.tif"] {
        std::fs::copy(inputs.join(f), dir.join(f)).unwrap();
    }
    for step in [
        &["new", "base.owd", "--title", "Flat", "--date", "2026-01-02"][..],
        &[
            "import",
            "base.owd",
            "--left-bank",
            "left.geojson",
            "--right-bank",
            "right.geojson",
            "--polygon",
            "river.geojson",
            "--dem",
            "flat.tif",
        ],
        &["import", "base.owd", "--water-level", "10"],
        &[
            "size",
            "base.owd",
            "--rule-set",
            "pianc-wg141",
            "--set-input",
            "se_quality=B",
            "--vessel",
            "CEMT_IV",
            "--draught",
            "2.5",
        ],
    ] {
        let o = owd_in(&dir, step);
        assert!(o.status.success(), "{step:?}: {}", said(&o));
    }
    dir
}

/// Copy `base.owd` to `name`, run the setup steps on it (their codes do not matter), and return
/// the path.
fn prepared(dir: &Path, name: &str, setup: &[&[&str]]) -> String {
    std::fs::copy(dir.join("base.owd"), dir.join(name)).unwrap();
    for step in setup {
        let mut args: Vec<&str> = step.to_vec();
        args.insert(1, name);
        owd_in(dir, &args);
    }
    name.to_string()
}

/// `owd <cmd> <doc> <rest…> --json`, with `--strict` when `strict`.
fn command(dir: &Path, cmd: &str, doc: &str, rest: &[&str], strict: bool) -> Output {
    let mut args = vec![cmd, doc];
    args.extend(rest);
    args.push("--json");
    if strict {
        args.push("--strict");
    }
    owd_in(dir, &args)
}

/// The stderr line that names the failed checks, if any.
fn fails_line(o: &Output) -> Option<String> {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .map(|l| l.strip_prefix("owd: ").unwrap_or(l))
        .find(|l| l.starts_with("the option fails:"))
        .map(str::to_string)
}

/// Whether the `--json` output lists a failed check: `failed_checks` at the top (one option) or on
/// an option of `options`.
fn json_lists_failed_checks(o: &Output) -> bool {
    let v: Value = match serde_json::from_slice(&o.stdout) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let non_empty = |f: &Value| f.as_array().is_some_and(|a| !a.is_empty());
    non_empty(&v["failed_checks"])
        || v["options"]
            .as_array()
            .is_some_and(|os| os.iter().any(|o| non_empty(&o["failed_checks"])))
}

fn read(dir: &Path, doc: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(doc)).unwrap()).unwrap()
}

/// One command on a failing design, without and with `--strict`.
fn check_failing(tag: &str, setup: &[&[&str]], cmd: &str, rest: &[&str]) {
    let dir = failing_design(tag);
    let (a, b) = (
        prepared(&dir, "plain.owd", setup),
        prepared(&dir, "strict.owd", setup),
    );
    let plain = command(&dir, cmd, &a, rest, false);
    let strict = command(&dir, cmd, &b, rest, true);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "{cmd}: a saved design exits 0: {}",
        said(&plain)
    );
    assert_eq!(
        strict.status.code(),
        Some(1),
        "{cmd} --strict: a failed check exits 1: {}",
        said(&strict)
    );
    for o in [&plain, &strict] {
        assert!(
            fails_line(o).is_some(),
            "{cmd}: stderr names the failed checks: {}",
            said(o)
        );
        assert!(
            json_lists_failed_checks(o),
            "{cmd}: --json lists the failed checks: {}",
            said(o)
        );
    }
    assert_eq!(
        fails_line(&plain),
        fails_line(&strict),
        "{cmd}: the same words"
    );
    assert_eq!(
        read(&dir, &a),
        read(&dir, &b),
        "{cmd}: the design is saved, the same in both runs"
    );
}

#[test]
fn align_rolling_circle_follows_the_rule() {
    check_failing(
        "rolling-circle",
        &[],
        "align",
        &["--method", "rolling-circle"],
    );
}

#[test]
fn align_reroute_follows_the_rule() {
    check_failing("reroute", &[], "align", &["--method", "reroute"]);
}

#[test]
fn align_centreline_follows_the_rule() {
    check_failing("centreline", &[], "align", &["--method", "centreline"]);
}

#[test]
fn optimise_follows_the_rule() {
    check_failing(
        "optimise",
        &[&["align", "--method", "rolling-circle"]],
        "optimise",
        &["--sites", "1200"],
    );
}

#[test]
fn volumes_follows_the_rule() {
    check_failing(
        "volumes",
        &[
            &["align", "--method", "rolling-circle"],
            &["optimise", "--sites", "1200"],
        ],
        "volumes",
        &[],
    );
}

#[test]
fn run_follows_the_rule() {
    // After `size`, the options are stale: `run` makes them again, and they fail.
    check_failing("run", &[], "run", &[]);
}

#[test]
fn a_design_whose_options_pass_exits_0_with_and_without_strict() {
    let dir = passing_design("pass");
    for strict in [false, true] {
        let doc = prepared(&dir, &format!("pass-{strict}.owd"), &[]);
        for (cmd, rest) in [
            ("align", &["--method", "rolling-circle"][..]),
            ("volumes", &[]),
            ("run", &[]),
        ] {
            let o = command(&dir, cmd, &doc, rest, strict);
            assert_eq!(
                o.status.code(),
                Some(0),
                "{cmd} (strict {strict}): {}",
                said(&o)
            );
            assert!(fails_line(&o).is_none(), "{cmd}: {}", said(&o));
        }
    }
}

#[test]
fn a_command_that_cannot_run_keeps_its_code() {
    let dir = failing_design("cannot-run");
    for args in [
        &["align", "missing.owd", "--method", "rolling-circle"][..],
        &["align", "base.owd", "--method", "nonsense"],
        &["volumes", "base.owd", "--no-such-flag"],
        &["optimise", "missing.owd"],
    ] {
        let plain = owd_in(&dir, args);
        let mut strict_args = args.to_vec();
        strict_args.push("--strict");
        let strict = owd_in(&dir, &strict_args);
        assert!(
            !matches!(plain.status.code(), Some(0) | Some(1)),
            "{args:?}: {}",
            said(&plain)
        );
        assert_eq!(
            plain.status.code(),
            strict.status.code(),
            "{args:?}: --strict leaves an error's code: {}",
            said(&strict)
        );
    }
}
