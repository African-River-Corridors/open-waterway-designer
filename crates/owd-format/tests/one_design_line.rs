//! Format 0.5 (spec 0017, ticket 6): one design line per run, and an older file opens into it.
//!
//! `tests/data/format-0.3-two-lines.owd` is the web fixture as format 0.3 wrote it (OWD 0.3.1): a
//! rolling-circle option and a pool-lattice option, every stage run, then the draught changed.
//!
//! Label: property

use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, Session, Stage, StageStatus, volume_rows_of};
use owd_format::geotiff;
use serde_json::{Value, json};
use std::path::Path;

fn old_text() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/format-0.3-two-lines.owd");
    std::fs::read_to_string(p).unwrap()
}

fn old() -> Value {
    serde_json::from_str(&old_text()).unwrap()
}

fn option<'a>(doc: &'a Value, method: &str) -> &'a Value {
    doc["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap()
}

#[test]
fn an_older_file_opens_with_one_design_line_and_its_later_stages_stale() {
    let before = old();
    let s = Session::open(&old_text()).unwrap();
    let d = s.document();
    assert_eq!(d["format_version"], "0.5");
    assert!(d.get("options").is_none(), "no list of options");
    // The rolling-circle option is the alignment's line; the lattice's is the optimise stage's.
    let al = &d["stages"]["alignment"];
    assert_eq!(al["settings"]["method"], "rolling_circle");
    assert_eq!(
        al["results"]["line"],
        option(&before, "rolling_circle")["line"]
    );
    assert_eq!(
        al["results"]["checks"],
        option(&before, "rolling_circle")["checks"]
    );
    let refined = &d["stages"]["optimise"]["results"]["design_line"];
    assert_eq!(refined["line"], option(&before, "lattice")["line"]);
    // The optimise stage is stale after the move, so the later stages read the alignment's line
    // (QA item 2), and the volumes keep that line's row.
    assert_eq!(s.design_lines().len(), 1);
    assert_eq!(s.design_lines()[0]["id"], "rolling-circle");
    let rows = volume_rows_of(d);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "rolling-circle");
    let old_row = before["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "rolling-circle")
        .unwrap();
    assert_eq!(
        &rows[0], old_row,
        "the stored numbers are kept as they were"
    );
    // Kept, but stale: no stage reads them as fresh until it runs again.
    for st in [Stage::Alignment, Stage::Optimise, Stage::Volumes] {
        assert_eq!(s.stage_status(st), StageStatus::Stale, "{}", st.name());
    }
    // Bends and cut-offs were never worked out for an old line: empty and unknown, not made up.
    assert_eq!(al["results"]["bends"], json!([]));
    assert!(al["results"]["cut_offs"].is_null());
    // No mouth was recorded, so no stored chainage changes meaning.
    assert_eq!(d["inputs"]["water_level"], before["inputs"]["water_level"]);
    assert_eq!(d["inputs"]["left_bank"], before["inputs"]["left_bank"]);
    assert!(d["inputs"].get("mouth").is_none());
    // The notes say what changed, in words.
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("one design line"), "{notes}");
    assert!(notes.contains("stale"), "{notes}");
    assert!(notes.contains("--mouth"), "{notes}");
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}

#[test]
fn the_moved_file_saves_as_0_5_and_reopens_the_same_with_no_notes() {
    let s = Session::open(&old_text()).unwrap();
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(again.to_text(), s.to_text());
    assert!(again.open_notes().is_empty());
}

#[test]
fn an_option_that_is_not_kept_is_named_in_the_notes() {
    // A 0.4 file whose options were a re-route and a given line: the first is the design line.
    let mut doc = old();
    doc["format_version"] = json!("0.4");
    let rc = option(&doc, "rolling_circle").clone();
    let mut reroute = rc.clone();
    reroute["id"] = json!("reroute");
    reroute["name"] = json!("Least-cost re-route");
    reroute["method"] = json!("reroute");
    let mut given = rc;
    given["id"] = json!("given-old-study");
    given["name"] = json!("Old study");
    given["method"] = json!("given");
    doc["options"] = json!([reroute, given]);
    let s = Session::open(&doc.to_string()).unwrap();
    let d = s.document();
    assert_eq!(d["stages"]["alignment"]["settings"]["method"], "reroute");
    assert_eq!(d["stages"]["alignment"]["results"]["id"], "reroute");
    let notes = s.open_notes().join("\n");
    // The given line is not the design line, but its points are kept (QA item 1).
    assert!(
        notes.contains("`given-old-study` (Old study) is kept in the alignment settings"),
        "{notes}"
    );
    assert_eq!(
        d["stages"]["alignment"]["settings"]["given"]["name"],
        "Old study"
    );
    assert!(notes.contains("owd align --method given"), "{notes}");
}

#[test]
fn after_the_move_a_stage_run_makes_the_alignment_fresh_again() {
    let mut s = Session::open(&old_text()).unwrap();
    let dem = geotiff::read_bytes(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-dem.tif"),
        )
        .unwrap(),
    )
    .unwrap();
    let t = dem_sampler(&dem, Some(32630)).unwrap();
    s.run_stage_with(Stage::Sizing, Some(&t)).unwrap();
    s.run_stage_with(Stage::Alignment, Some(&t)).unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    // Now worked out: the line's bends, and its cut-offs against the river polygon.
    let line = &s.document()["stages"]["alignment"]["results"];
    assert!(line["cut_offs"].is_array(), "{}", line["cut_offs"]);
}

/// QA on PR 558, item 1: a 0.4 file with a rolling-circle option and a given line keeps the rolling
/// circle as the design line, and keeps the given line's points in the alignment settings, so the
/// advice in the note (make it the design line with `owd align --method given`) works.
#[test]
fn a_given_line_beside_the_rolling_circle_is_kept_and_the_advised_run_makes_it_the_design_line() {
    let mut doc = old();
    doc["format_version"] = json!("0.4");
    let rc = option(&doc, "rolling_circle").clone();
    let mut given = rc.clone();
    given["id"] = json!("given-old-study");
    given["name"] = json!("Old study");
    given["method"] = json!("given");
    let pts = json!([[500_100.0, 600_000.0], [501_900.0, 600_050.0]]);
    given["line"] = json!({"type": "LineString", "coordinates": pts});
    doc["options"] = json!([rc, given]);
    doc["stages"]["optimise"]["results"] = Value::Null;
    doc["stages"]["optimise"]["depends_on"] = Value::Null;
    let mut s = Session::open(&doc.to_string()).unwrap();
    let al = &s.document()["stages"]["alignment"];
    assert_eq!(
        al["settings"]["method"], "rolling_circle",
        "the rolling circle wins"
    );
    assert_eq!(al["results"]["id"], "rolling-circle");
    assert_eq!(al["settings"]["given"]["name"], "Old study");
    assert_eq!(al["settings"]["given"]["line"]["coordinates"], pts);
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("`given-old-study` (Old study)"), "{notes}");
    assert!(notes.contains("owd align --method given"), "{notes}");
    // The advised run, at the session: sizing first (the file's draught changed), then the method.
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::Given, &Value::Null, None).unwrap();
    let line = &s.document()["stages"]["alignment"]["results"];
    assert_eq!(line["id"], "given-old-study");
    assert_eq!(line["line"]["coordinates"], pts);
}
