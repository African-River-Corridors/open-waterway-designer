//! QA on PR 558, item 1, on the command line: a 0.4 file with a rolling-circle option and a given
//! line opens with the rolling circle as the design line; the note says how to make the given line
//! the design line, and that command, run as the note gives it, does.
//!
//! Label: property

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../owd-format/tests/data")
        .join(name)
}

#[test]
fn the_note_s_command_makes_the_kept_given_line_the_design_line() {
    let dir = std::env::temp_dir().join(format!("owd-old-given-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(data("format-0.3-two-lines.owd")).unwrap())
            .unwrap();
    doc["format_version"] = json!("0.4");
    let rc = doc["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "rolling_circle")
        .unwrap()
        .clone();
    let mut given = rc.clone();
    given["id"] = json!("given-old-study");
    given["name"] = json!("Old study");
    given["method"] = json!("given");
    let pts = json!([[500_100.0, 600_000.0], [501_900.0, 600_050.0]]);
    given["line"] = json!({"type": "LineString", "coordinates": pts});
    doc["options"] = json!([rc, given]);
    doc["stages"]["optimise"]["results"] = Value::Null;
    doc["stages"]["optimise"]["depends_on"] = Value::Null;
    let path = dir.join("old.owd");
    std::fs::write(&path, doc.to_string()).unwrap();
    let p = path.to_str().unwrap();
    let owd = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_owd"))
            .args(args)
            .output()
            .unwrap()
    };

    // Size again (the file's draught changed), then read the advice the open prints.
    let o = owd(&["size", p, "--draught", "2.6"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let said = String::from_utf8_lossy(&o.stderr).to_string();
    let advice = said
        .lines()
        .find(|l| l.contains("given-old-study"))
        .unwrap_or_else(|| panic!("no note for the given line: {said}"));
    // The command is the text between back-ticks that starts with `owd align`.
    let at = advice.find("`owd align").expect("the note gives a command") + 1;
    let end = at + advice[at..].find('`').expect("the command ends");
    let cmd: Vec<&str> = advice[at..end].split_whitespace().collect();
    assert_eq!(cmd, ["owd", "align", "--method", "given"], "{advice}");
    // The command as the note gives it, on this file.
    let o = owd(&["align", p, cmd[2], cmd[3]]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let line = &saved["stages"]["alignment"]["results"];
    assert_eq!(line["id"], "given-old-study");
    assert_eq!(line["line"]["coordinates"], pts);
    let _ = std::fs::remove_dir_all(&dir);
}
