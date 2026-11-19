//! The design sanity check on the command line: a design line with a bend under R_min prints a
//! warning on stderr and exits 0 (a warning is not a failed check); a repaired line prints none.
//!
//! The design is the repository's synthetic river (`examples/web`) sized for CEMT Va, whose bends
//! are tighter than the vessel's R_min; the unrepaired centreline option keeps them.
//!
//! Label: property

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

fn design(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-sanity-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let web = root().join("examples/web");
    for f in ["river-rolling-circle.owd", "river-dem.tif"] {
        std::fs::copy(web.join(f), dir.join(f)).unwrap();
    }
    std::fs::rename(dir.join("river-rolling-circle.owd"), dir.join("base.owd")).unwrap();
    owd_in(&dir, &["run", "base.owd"]);
    let o = owd_in(&dir, &["size", "base.owd", "--vessel", "CEMT_Va"]);
    assert!(o.status.success());
    dir
}

fn warnings(o: &Output) -> Vec<String> {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with("owd: warning:"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_bend_under_r_min_prints_a_warning_and_the_command_still_exits_0() {
    let dir = design("bends");
    let o = owd_in(&dir, &["align", "base.owd", "--method", "centreline"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let w = warnings(&o);
    assert!(!w.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(w.iter().all(|l| l.contains("tighter than R_min")), "{w:?}");
}

#[test]
fn a_repaired_line_prints_no_warning() {
    let dir = design("repaired");
    let o = owd_in(
        &dir,
        &[
            "align",
            "base.owd",
            "--method",
            "rolling-circle",
            "--use-water",
            "false",
        ],
    );
    assert!(
        warnings(&o).is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
