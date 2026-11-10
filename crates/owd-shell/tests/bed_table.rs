//! The depth table is read in one place, below both surfaces.
//!
//! The app sends the depth and the depth table as the user typed them. The shell reads the text
//! with the reader `owd bed --table` uses, so the same table gives the same bed model on both
//! surfaces, and the same bad table is refused with the same words. The app's own code no longer
//! reads the table.
//!
//! Label: property

use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const DATE: &str = "2026-01-02";

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("owd-bed-table-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The bed model's points after the app shell takes a depth and a table as typed. `Err` holds the
/// refusal.
fn shell_bed(depth: &str, table: &str) -> Result<Value, String> {
    let mut state = AppState::new_design("Bed table", DATE);
    state.set_bed_text(depth, table, Some("dry"), DATE)?;
    Ok(state.snapshot()["document"]["inputs"]["bed_model"]["points"].clone())
}

/// The bed model's points after `owd bed --depth D` or `owd bed --table FILE`. `Err` holds the
/// refusal, without the table file's name in front.
fn cli_bed(dir: &Path, depth: Option<f64>, table: Option<&str>) -> Result<Value, String> {
    let doc = dir.join("d.owd");
    let _ = std::fs::remove_file(&doc);
    if owd_cli::commands::new(&doc.to_string_lossy(), "Bed table", Some(DATE), true).is_err() {
        panic!("`owd new` failed");
    }
    let file = dir.join("depths.csv");
    if let Some(text) = table {
        std::fs::write(&file, text).unwrap();
    }
    let file = file.to_string_lossy().into_owned();
    owd_cli::bed::bed(
        &doc.to_string_lossy(),
        depth,
        table.map(|_| file.as_str()),
        Some("dry"),
        false,
        &None,
        true,
    )
    .map_err(|f| {
        let named = format!("{file}, ");
        f.message
            .strip_prefix(&named)
            .unwrap_or(&f.message)
            .to_string()
    })?;
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
    Ok(d["inputs"]["bed_model"]["points"].clone())
}

#[test]
fn the_same_table_gives_the_same_bed_model_on_both_surfaces() {
    let dir = work_dir("same");
    // A header, a comment, a blank line and spaces: all that `owd bed --table` takes today.
    let table = "chainage_m,depth_m\n# surveyed at low water\n0, 2.5\n\n 1200 ,3\n4000,1.75\n";
    let points = json!([[0.0, 2.5], [1200.0, 3.0], [4000.0, 1.75]]);
    assert_eq!(cli_bed(&dir, None, Some(table)), Ok(points.clone()));
    assert_eq!(shell_bed("", table), Ok(points));

    // One depth is the depth from the start of the river.
    assert_eq!(cli_bed(&dir, Some(2.5), None), Ok(json!([[0.0, 2.5]])));
    assert_eq!(shell_bed(" 2.5 ", ""), Ok(json!([[0.0, 2.5]])));
    // In the app a depth may stand with a table: the depth holds from the start.
    assert_eq!(
        shell_bed("2", "500,3\n900,4"),
        Ok(json!([[0.0, 2.0], [500.0, 3.0], [900.0, 4.0]]))
    );
}

#[test]
fn the_same_bad_table_is_refused_with_the_same_words_on_both_surfaces() {
    let dir = work_dir("bad");
    for (table, says) in [
        ("0,2\nabc,3", "line 2"),
        ("0,2\n100", "line 2"),
        ("0,2\n\n100,3,4", "line 3"),
        ("0,2\n100;3", "line 2"),
        ("0,2\n100,", "line 2"),
        // The session's own checks: chainages that go back, a depth below zero, no depth at all.
        ("100,2\n0,3", "must not decrease"),
        ("0,-2", "negative"),
        ("# nothing yet\n", "at least one depth"),
    ] {
        let cli = cli_bed(&dir, None, Some(table)).expect_err("the command line refuses the table");
        let shell = shell_bed("", table);
        assert_eq!(
            shell,
            Err(cli.clone()),
            "the shell and the command line differ on the table {table:?}"
        );
        assert!(cli.contains(says), "the refusal of {table:?} is: {cli}");
    }
    // A depth that is not a number is refused by name. It never reaches the session as NaN.
    let why = shell_bed("abc", "").expect_err("a depth that is not a number is refused");
    assert!(why.contains("`abc`"), "the refusal is: {why}");
}

#[test]
fn the_app_does_not_read_the_depth_table_itself() {
    let page = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src/lib/StartFromRiver.svelte"),
    )
    .unwrap();
    for own_reading in ["depthRows.split(", "Number(depth)"] {
        assert!(
            !page.contains(own_reading),
            "app/src/lib/StartFromRiver.svelte still reads the depth itself: `{own_reading}`"
        );
    }
}
