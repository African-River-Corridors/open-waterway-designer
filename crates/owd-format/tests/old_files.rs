//! Old files keep their work (the owner's rule of 20 November 2026): a format 0.4 file whose stages
//! were up to date when it was saved opens up to date, with its numbers, and runs from the river
//! mouth (spec 0017, chainage from the mouth).
//!
//! The four `format-0.4-*.owd` files in `tests/data` were written by the last 0.4 engine (the commit
//! before "Stage contracts 06", built from source) on the web example's river and DEM
//! (`examples/web`), with the CEMT-IV vessel under PIANC WG 141:
//!
//! - `rolling-circle`: a rolling-circle option and its volumes, both fresh. The water rises along
//!   the stored line (`--water-level 0 --water-slope=-0.0005`), so the mouth is its first end.
//! - `lattice-only`: only a pool-lattice option (`owd optimise --sites 1200`, no `owd align`), and
//!   its volumes, both fresh. The same water.
//! - `stale-volumes`: as `rolling-circle`, then a new water level, so the volumes are stale and the
//!   rolling circle (which reads no water) is fresh.
//! - `reversed`: a rolling-circle option, a pool-lattice option and the volumes, all fresh. The water
//!   falls along the stored line (`--water-level 0.4 --water-slope 0.0002`): the mouth is the far end.
//!
//! Label: property

use owd_format::design::terrain::dem_sampler;
use owd_format::design::{Session, Stage, StageStatus, volume_rows_of};
use owd_format::geotiff;
use serde_json::{Value, json};
use std::path::Path;

use StageStatus::{Fresh, NotRun, Stale};

fn text(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/data/format-0.4-{name}.owd"));
    std::fs::read_to_string(p).unwrap()
}

fn raw(name: &str) -> Value {
    serde_json::from_str(&text(name)).unwrap()
}

fn dem() -> geotiff::Dem {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-dem.tif");
    geotiff::read_bytes(&std::fs::read(p).unwrap()).unwrap()
}

fn statuses(s: &Session) -> [StageStatus; 4] {
    Stage::ALL.map(|st| s.stage_status(st))
}

fn option<'a>(doc: &'a Value, method: &str) -> &'a Value {
    doc["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap()
}

fn old_row<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["stages"]["volumes"]["results"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()
}

/// The numbers of a volumes row.
const NUMBERS: [&str; 8] = [
    "dredge_m3",
    "excav_m3",
    "total_m3",
    "in_river_m3",
    "widening_m3",
    "cut_through_m3",
    "max_face_h_m",
    "stations",
];

fn assert_same_numbers(now: &Value, then: &Value) {
    for k in NUMBERS {
        assert_eq!(now[k], then[k], "{k}");
    }
}

fn coords(v: &Value) -> Vec<Value> {
    v["coordinates"].as_array().unwrap().clone()
}

fn reversed(v: &Value) -> Vec<Value> {
    let mut c = coords(v);
    c.reverse();
    c
}

#[test]
fn a_fresh_rolling_circle_file_opens_fresh_with_its_numbers() {
    let before = raw("rolling-circle");
    let s = Session::open(&text("rolling-circle")).unwrap();
    assert_eq!(statuses(&s), [Fresh, Fresh, NotRun, Fresh]);
    let d = s.document();
    let al = &d["stages"]["alignment"]["results"];
    let rc = option(&before, "rolling_circle");
    assert_eq!(al["line"], rc["line"], "the line is the stored line");
    assert_eq!(al["checks"], rc["checks"]);
    // A fresh line is a whole 0.5 design line: its bends and its cut-offs are worked out.
    assert!(al["bends"].is_array());
    assert!(al["cut_offs"].is_array(), "{}", al["cut_offs"]);
    let rows = volume_rows_of(d);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "rolling-circle");
    assert_same_numbers(&rows[0], old_row(&before, "rolling-circle"));
    // The water rises along the stored line, so its first end is the mouth: nothing is turned.
    assert_eq!(d["inputs"]["centreline"], before["inputs"]["centreline"]);
    assert_eq!(d["inputs"]["water_level"], before["inputs"]["water_level"]);
    assert_eq!(
        d["inputs"]["mouth"]["point"],
        before["inputs"]["centreline"]["coordinates"][0]
    );
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("kept fresh"), "{notes}");
    assert!(!notes.contains("marked stale"), "{notes}");
    assert!(notes.contains("mouth"), "{notes}");
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}

#[test]
fn a_file_with_only_a_fresh_lattice_line_opens_with_it_as_the_fresh_design_line() {
    let before = raw("lattice-only");
    let s = Session::open(&text("lattice-only")).unwrap();
    assert_eq!(statuses(&s), [Fresh, Fresh, Fresh, Fresh]);
    let lines = s.design_lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["id"], "lattice");
    assert_eq!(lines[0]["line"], option(&before, "lattice")["line"]);
    assert!(s.stale_refined_line_note().is_none());
    let rows = volume_rows_of(s.document());
    assert_eq!(rows[0]["id"], "lattice");
    assert_same_numbers(&rows[0], old_row(&before, "lattice"));
    // The pools and the weirs are kept as they were.
    let op = &s.document()["stages"]["optimise"]["results"];
    assert_eq!(
        op["pools"],
        before["stages"]["optimise"]["results"]["pools"]
    );
    assert_eq!(
        op["cascade"],
        before["stages"]["optimise"]["results"]["cascade"]
    );
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("kept fresh"), "{notes}");
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}

#[test]
fn a_stale_stage_stays_stale_and_a_fresh_one_stays_fresh() {
    let before = raw("stale-volumes");
    let s = Session::open(&text("stale-volumes")).unwrap();
    assert_eq!(statuses(&s), [Fresh, Fresh, NotRun, Stale]);
    // The stale numbers are kept, as they were.
    assert_same_numbers(
        &volume_rows_of(s.document())[0],
        old_row(&before, "rolling-circle"),
    );
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("kept fresh"), "{notes}");
    assert!(
        notes.contains("volumes") && notes.contains("stale"),
        "{notes}"
    );
}

#[test]
fn a_file_stale_when_it_was_saved_stays_stale() {
    // Format 0.3, every stage run and then the draught changed and the sizing run again (see
    // `one_design_line.rs`): the sizing is fresh, the stages after it stale.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/format-0.3-two-lines.owd");
    let s = Session::open(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert_eq!(statuses(&s), [Fresh, Stale, Stale, Stale]);
}

#[test]
fn a_moved_file_saves_and_reopens_unchanged() {
    for name in [
        "rolling-circle",
        "lattice-only",
        "stale-volumes",
        "reversed",
    ] {
        let s = Session::open(&text(name)).unwrap();
        let again = Session::open(&s.to_text()).unwrap();
        assert_eq!(again.to_text(), s.to_text(), "{name}");
        assert!(again.open_notes().is_empty(), "{name}");
        assert_eq!(statuses(&again), statuses(&s), "{name}");
    }
}

/// The volumes run again on the moved file: the line is not touched, the totals are the stored ones
/// (the same DEM), and the stations are stored for the drawings.
fn volumes_run_again(name: &str, id: &str) {
    let before = raw(name);
    let mut s = Session::open(&text(name)).unwrap();
    let line = s.design_lines()[0]["line"].clone();
    let dem = dem();
    let t = dem_sampler(&dem, Some(32630)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t)).unwrap();
    assert_eq!(s.design_lines()[0]["line"], line, "the line is not touched");
    assert_eq!(s.stage_status(Stage::Volumes), Fresh);
    let row = &volume_rows_of(s.document())[0];
    assert_eq!(row["id"], id);
    let then = old_row(&before, id);
    for k in NUMBERS {
        let (a, b) = (row[k].as_f64().unwrap(), then[k].as_f64().unwrap());
        assert!(
            (a - b).abs() <= 1e-9 * b.abs().max(1.0),
            "{name} {k}: {a} now, {b} stored"
        );
    }
    assert!(row["sections"].is_object(), "the stations are stored");
    assert!(s.design_water().is_some(), "the water along the line too");
}

#[test]
fn the_volumes_run_again_on_a_moved_rolling_circle_file_and_give_its_numbers() {
    volumes_run_again("rolling-circle", "rolling-circle");
}

#[test]
fn the_volumes_run_again_on_a_moved_lattice_file_and_give_its_numbers() {
    volumes_run_again("lattice-only", "lattice");
}

// --- the mouth ---------------------------------------------------------------------------------

#[test]
fn a_reversed_file_opens_from_the_mouth_with_the_same_design_and_stays_fresh() {
    let before = raw("reversed");
    let s = Session::open(&text("reversed")).unwrap();
    let d = s.document();
    assert_eq!(statuses(&s), [Fresh, Fresh, Fresh, Fresh]);
    // Chainage 0 is the low-water end: the far end of the stored centreline.
    let old_centre = &before["inputs"]["centreline"];
    assert_eq!(
        d["inputs"]["mouth"]["point"],
        *coords(old_centre).last().unwrap()
    );
    assert_eq!(coords(&d["inputs"]["centreline"]), reversed(old_centre));
    let wl = d["inputs"]["water_level"]["points"].as_array().unwrap();
    let (first, last) = (
        wl[0][1].as_f64().unwrap(),
        wl.last().unwrap()[1].as_f64().unwrap(),
    );
    assert!(first < last, "the water rises from the mouth: {wl:?}");
    // The same design: each line is the stored line, turned.
    let al = &d["stages"]["alignment"]["results"];
    assert_eq!(
        coords(&al["line"]),
        reversed(&option(&before, "rolling_circle")["line"])
    );
    let refined = &d["stages"]["optimise"]["results"]["design_line"];
    assert_eq!(
        coords(&refined["line"]),
        reversed(&option(&before, "lattice")["line"])
    );
    assert_eq!(s.design_lines()[0]["id"], "lattice");
    // The pools and the weir count from the mouth: the same water at the same places.
    let op = &d["stages"]["optimise"]["results"];
    let old_op = &before["stages"]["optimise"]["results"];
    let base_len = old_op["pools"].as_array().unwrap().last().unwrap()[1]
        .as_f64()
        .unwrap();
    let old_weir = old_op["cascade"]["weirs"][0]["chainage_m"]
        .as_f64()
        .unwrap();
    let weir = op["cascade"]["weirs"][0]["chainage_m"].as_f64().unwrap();
    assert!((weir - (base_len - old_weir)).abs() < 1e-9, "{weir}");
    let pools = op["pools"].as_array().unwrap();
    let old_pools = old_op["pools"].as_array().unwrap();
    assert_eq!(pools.len(), old_pools.len());
    for (p, q) in pools.iter().zip(old_pools.iter().rev()) {
        assert_eq!(p[2], q[2], "each pool keeps its level");
        let (a, b) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
        let (qa, qb) = (q[0].as_f64().unwrap(), q[1].as_f64().unwrap());
        assert!((a - (base_len - qb)).abs() < 1e-9 && (b - (base_len - qa)).abs() < 1e-9);
    }
    assert_eq!(
        d["stages"]["optimise"]["settings"]["candidate_sites_m"],
        json!([base_len - 1200.0])
    );
    // The volumes keep their numbers: the same line, the same cut.
    assert_same_numbers(&volume_rows_of(d)[0], old_row(&before, "lattice"));
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("turned"), "{notes}");
    assert!(notes.contains("kept fresh"), "{notes}");
    assert!(s.validate().is_empty(), "{:?}", s.validate());
}

#[test]
fn a_file_with_no_water_level_and_no_dem_is_not_turned_and_says_why() {
    let mut doc = raw("reversed");
    doc["inputs"]["water_level"] = Value::Null;
    doc["inputs"]["dem"] = Value::Null;
    let s = Session::open(&doc.to_string()).unwrap();
    let d = s.document();
    assert!(d["inputs"].get("mouth").is_none());
    assert_eq!(d["inputs"]["centreline"], doc["inputs"]["centreline"]);
    assert!(!s.old_mouth_pending());
    let notes = s.open_notes().join("\n");
    assert!(notes.contains("state the mouth"), "{notes}");
    assert!(notes.contains("--mouth"), "{notes}");
}

#[test]
fn opening_twice_does_not_turn_twice() {
    let mut s = Session::open(&text("reversed")).unwrap();
    let once = s.to_text();
    assert!(
        s.settle_old_mouth(None).is_empty(),
        "nothing is left to settle"
    );
    assert_eq!(s.to_text(), once);
    let again = Session::open(&once).unwrap();
    assert_eq!(again.to_text(), once);
    assert_eq!(
        again.document()["inputs"]["centreline"],
        s.document()["inputs"]["centreline"]
    );
}

#[test]
fn with_no_water_level_the_dem_says_which_end_is_the_mouth() {
    // The rolling circle reads no water, so it stays fresh when the water level goes.
    let mut doc = raw("rolling-circle");
    doc["inputs"]["water_level"] = Value::Null;
    let mut s = Session::open(&doc.to_string()).unwrap();
    assert!(s.old_mouth_pending(), "the DEM is named but not loaded");
    assert!(s.document()["inputs"].get("mouth").is_none());
    assert!(
        s.open_notes().join("\n").contains("elevation model"),
        "{:?}",
        s.open_notes()
    );
    let dem = dem();
    let t = dem_sampler(&dem, Some(32630)).unwrap();
    let centre = coords(&doc["inputs"]["centreline"]);
    let end = |p: &Value| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()];
    let (a, b) = (end(&centre[0]), end(centre.last().unwrap()));
    let z = t(&[a[0], b[0]], &[a[1], b[1]]);
    assert!((z[0] - z[1]).abs() > 0.01, "the DEM can tell: {z:?}");
    let notes = s.settle_old_mouth(Some(&t));
    assert!(!notes.is_empty());
    assert!(!s.old_mouth_pending());
    let lower = if z[0] < z[1] { a } else { b };
    assert_eq!(s.document()["inputs"]["mouth"]["point"], json!(lower));
    assert_eq!(s.stage_status(Stage::Alignment), Fresh);
    let line = coords(&s.document()["stages"]["alignment"]["results"]["line"]);
    let old = option(&doc, "rolling_circle");
    if z[0] < z[1] {
        assert_eq!(line, coords(&old["line"]));
    } else {
        assert_eq!(line, reversed(&old["line"]));
    }
    // Settled once: a second call changes nothing.
    let once = s.to_text();
    assert!(s.settle_old_mouth(Some(&t)).is_empty());
    assert_eq!(s.to_text(), once);
}
