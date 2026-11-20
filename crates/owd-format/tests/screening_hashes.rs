//! QA on PR 560 (spec 0017 ticket 7): what the reference line, the fits answer and the envelope
//! hash, and what an older file keeps when it opens.
//!
//! - A reference line's hash covers what makes the line: the banks, the river polygon, the project
//!   CRS and the parameters. It is the formula OWD used before the line became a stage, so a line
//!   that was fresh in an older file stays fresh when it moves, and the volumes that read the bed
//!   centred on it stay fresh too (they would otherwise rerun with the bed on the centreline).
//! - The fits answer and the envelope go stale when anything they read changes, one field at a time.
//! - Opening an older file says in plain words what moved and why an answer shows stale.
//!
//! Label: property

use owd_core::envelope::Settings;
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_format::design::envelope::EnvelopeResult;
use owd_format::design::hash::hash_value;
use owd_format::design::reference_line::{
    ReferenceLineIn, reference_line_parameters, reference_line_stage,
};
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use owd_format::types::{Polyline, RiverPolygon};
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn at(x: f64, y: f64) -> [f64; 2] {
    [x + 500_000.0, y + 600_000.0]
}

fn line(pts: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn polygon(ring: &[[f64; 2]]) -> String {
    format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring)
    )
}

type Line = Vec<[f64; 2]>;

/// A straight river 200 m wide and 3 km long: its banks, its polygon's ring and a centreline that
/// zigzags 30 m either side of the middle (so a bed centred on it differs from one on the middle).
fn river_lines() -> (Line, Line, Line, Line) {
    let left: Line = (0..=60).map(|i| at(50.0 * i as f64, 100.0)).collect();
    let right: Line = (0..=60).map(|i| at(50.0 * i as f64, -100.0)).collect();
    let centre: Line = (0..=50)
        .map(|i| at(60.0 * i as f64, if i % 2 == 0 { 30.0 } else { -30.0 }))
        .collect();
    let mut ring = right.clone();
    ring.extend(left.iter().rev());
    ring.push(ring[0]);
    (left, right, centre, ring)
}

/// The river with its polygon and centreline, and its banks when `banks`.
fn river(banks: bool) -> Session {
    let (left, right, centre, ring) = river_lines();
    let (p, c, l, r) = (polygon(&ring), line(&centre), line(&left), line(&right));
    let mut roles = vec![
        (Role::RiverPolygon, p.as_str()),
        (Role::Centreline, c.as_str()),
    ];
    if banks {
        roles.push((Role::LeftBank, l.as_str()));
        roles.push((Role::RightBank, r.as_str()));
    }
    let mut s = Session::new("Hashes", "2026-11-20");
    s.import(&roles).unwrap();
    s
}

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![12.0; xs.len()]
}

/// The river sized, aligned on its centreline, with water at 10 m, an assumed bed of 3 m, a
/// stand-in DEM reference, a reference line and volumes: everything fresh.
fn bedded_with_volumes() -> Session {
    let mut s = river(true);
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(&[[0.0, 3.0], [4000.0, 3.0]], Some("dry"), "2026-11-20")
        .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::Centreline, &Value::Null, None)
        .unwrap();
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    s.set_dem("dem.tif", b"a stand-in terrain file", &dem);
    s.run_reference_line().unwrap();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    s
}

/// The hash OWD gave a reference line before it was a stage (PR 558's branch, written out here
/// by hand so the test does not read the code it checks): the banks, the polygon, the project CRS
/// and the parameters, as the document holds them.
fn old_line_hash(doc: &Value) -> String {
    let inp = &doc["inputs"];
    hash_value(&json!({
        "left_bank": inp["left_bank"],
        "right_bank": inp["right_bank"],
        "river_polygon": inp["river_polygon"],
        "crs": doc["project"]["crs"],
        "parameters": reference_line_parameters(),
    }))
}

/// `doc` with its reference line put back where an older OWD kept it (`inputs.reference_line`).
fn to_old_place(doc: &Value, depends_on: &str) -> Value {
    let mut doc = doc.clone();
    let rec = doc["stages"]
        .as_object_mut()
        .unwrap()
        .remove("reference_line")
        .unwrap();
    doc["inputs"]["reference_line"] = json!({
        "line": rec["results"]["line"],
        "parameters": rec["settings"],
        "depends_on": depends_on,
        "low_confidence": rec["results"]["low_confidence"],
        "outside_water": rec["results"]["outside_water"],
        "notes": rec["results"]["notes"],
    });
    doc
}

fn volumes_numbers(s: &Session) -> Value {
    let r = &s.document()["stages"]["volumes"]["results"];
    json!([r["dredge_m3"], r["excav_m3"], r["total_m3"]])
}

/// Blocking item 1. An older file whose reference line was fresh, with a bed model and fresh
/// volumes, opens with all of them fresh, and `owd run` changes nothing. The line's hash is the
/// old formula, so the volumes' record (which holds the line's hash while the line is fresh) is
/// the one the older OWD wrote.
#[test]
fn an_old_file_with_a_fresh_line_and_a_bed_keeps_its_volumes_fresh_and_run_keeps_them() {
    let s = bedded_with_volumes();
    let doc = s.document();
    assert_eq!(
        doc["stages"]["reference_line"]["depends_on"],
        json!(old_line_hash(doc)),
        "a reference line's hash is what makes the line: banks, polygon, CRS, parameters"
    );
    let before = volumes_numbers(&s);
    let old = to_old_place(doc, &old_line_hash(doc));
    let mut back = Session::open(&old.to_string()).unwrap();
    assert_eq!(back.reference_line_status(), StageStatus::Fresh);
    assert_eq!(back.stage_status(Stage::Volumes), StageStatus::Fresh);
    let ran = back.run_stale_with(Some(&flat)).unwrap();
    assert!(ran.is_empty(), "nothing is due: {ran:?}");
    assert_eq!(volumes_numbers(&back), before);
    // A moved line that was stale stays stale.
    let stale = to_old_place(doc, &"1".repeat(64));
    let back = Session::open(&stale.to_string()).unwrap();
    assert_eq!(back.reference_line_status(), StageStatus::Stale);
}

/// Item 10: a file saved after the move reopens unchanged.
#[test]
fn a_moved_file_saved_again_reopens_unchanged() {
    let s = bedded_with_volumes();
    let old = to_old_place(s.document(), &old_line_hash(s.document()));
    let once = Session::open(&old.to_string()).unwrap().to_text();
    let twice = Session::open(&once).unwrap().to_text();
    assert_eq!(once, twice);
}

/// Item 2, and QA item 11 on the second pass. The centreline and the banks' notes place the
/// line's low-confidence stretches but do not shape the line. A change to either makes the stage
/// record stale (its stretches are out of date, and it says so), but the bed stays centred on the
/// line, so the volumes stay fresh. A bank shapes the line: both go stale.
#[test]
fn new_bank_notes_make_the_record_stale_but_keep_the_bed_axis_and_the_volumes() {
    let mut s = bedded_with_volumes();
    s.set_low_confidence_width(json!({"min_width_m": 90.0, "stretches": [
        {"from_m": 500.0, "to_m": 700.0, "kind": "narrow"}]}))
        .unwrap();
    assert_eq!(s.reference_line_status(), StageStatus::Stale, "bank notes");
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh, "bed axis kept");

    let mut t = river(true);
    t.run_reference_line().unwrap();
    let straight: Line = (0..=30).map(|i| at(100.0 * i as f64, 5.0)).collect();
    t.import(&[(Role::Centreline, line(&straight).as_str())])
        .unwrap();
    assert_eq!(t.reference_line_status(), StageStatus::Stale, "centreline");
    t.run_reference_line().unwrap();
    assert_eq!(t.reference_line_status(), StageStatus::Fresh);

    let moved: Line = (0..=60).map(|i| at(50.0 * i as f64, 110.0)).collect();
    s.import(&[(Role::LeftBank, line(&moved).as_str())])
        .unwrap();
    assert_eq!(s.reference_line_status(), StageStatus::Stale);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale, "a bank");
}

/// Item 3. A river with a polygon and a centreline but no banks: fits and the envelope read the
/// centreline. Once both banks are imported a rerun would read a reference line, so both are stale.
#[test]
fn fits_and_the_envelope_go_stale_when_both_banks_arrive() {
    let mut s = river(false);
    s.run_fits(None).unwrap();
    s.run_envelope(Settings::default(), true).unwrap();
    assert_eq!(s.fits_status(), StageStatus::Fresh);
    assert_eq!(s.envelope_status(), StageStatus::Fresh);
    let (left, right, _, _) = river_lines();
    s.import(&[
        (Role::LeftBank, line(&left).as_str()),
        (Role::RightBank, line(&right).as_str()),
    ])
    .unwrap();
    assert_eq!(s.fits_status(), StageStatus::Stale);
    assert_eq!(s.envelope_status(), StageStatus::Stale);
}

/// `doc` with the value at `pointer` replaced, opened again.
fn edited(s: &Session, pointer: &str, v: Value) -> Session {
    let mut doc = s.document().clone();
    *doc.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} is in the document")) = v;
    Session::open(&doc.to_string()).unwrap()
}

/// Item 7. The project CRS is part of what the line, fits and the envelope read.
#[test]
fn a_new_project_crs_makes_the_line_fits_and_the_envelope_stale() {
    let mut s = river(true);
    s.run_reference_line().unwrap();
    s.run_fits(None).unwrap();
    s.run_envelope(Settings::default(), true).unwrap();
    let name = s.document()["project"]["crs"]["name"].as_str().unwrap();
    let t = edited(
        &s,
        "/project/crs/name",
        json!(format!("{name} (as relabelled)")),
    );
    assert_eq!(t.reference_line_status(), StageStatus::Stale);
    assert_eq!(t.fits_status(), StageStatus::Stale);
    assert_eq!(t.envelope_status(), StageStatus::Stale);
}

fn barge() -> CustomVessel {
    CustomVessel {
        name: "A barge".into(),
        loa_m: 85.0,
        beam_m: 9.5,
        draught_m: 2.5,
        formation: Formation::Motor,
        n_abreast: None,
        n_in_line: None,
        gb_class: None,
        air_draught_m: None,
        deadweight_t: None,
    }
}

/// Item 10. The envelope goes stale when its settings, the custom vessel or the bed's shallowest
/// depth change, one at a time.
#[test]
fn the_envelope_goes_stale_on_each_thing_it_reads() {
    let mut s = river(true);
    s.run_envelope(Settings::default(), true).unwrap();
    assert_eq!(s.envelope_status(), StageStatus::Fresh);

    let t = edited(&s, "/stages/envelope/settings/draught_m", json!(1.5));
    assert_eq!(t.envelope_status(), StageStatus::Stale, "settings");

    let mut t = Session::open(&s.to_text()).unwrap();
    t.set_custom_vessel(&barge()).unwrap();
    assert_eq!(t.envelope_status(), StageStatus::Stale, "custom vessel");

    let mut t = Session::open(&s.to_text()).unwrap();
    t.set_water_profile(&[[0.0, 10.0]]).unwrap();
    assert_eq!(
        t.envelope_status(),
        StageStatus::Fresh,
        "the water is not read"
    );
    t.set_bed_model(&[[0.0, 3.0], [4000.0, 3.0]], Some("dry"), "2026-11-20")
        .unwrap();
    assert_eq!(
        t.envelope_status(),
        StageStatus::Stale,
        "bed shallowest depth"
    );
}

/// Item 10. The fits answer goes stale when its settings, the custom vessel, the polygon or the
/// centreline change, one at a time.
#[test]
fn a_plan_only_answer_goes_stale_on_each_thing_it_reads() {
    let mut s = river(false);
    s.run_fits(None).unwrap();
    assert_eq!(s.fits_status(), StageStatus::Fresh);

    let t = edited(&s, "/fits/settings/plan_line", json!("centreline"));
    assert_eq!(t.fits_status(), StageStatus::Stale, "settings");

    let mut t = Session::open(&s.to_text()).unwrap();
    t.set_custom_vessel(&barge()).unwrap();
    assert_eq!(t.fits_status(), StageStatus::Stale, "custom vessel");

    let mut t = Session::open(&s.to_text()).unwrap();
    let (_, _, _, mut ring) = river_lines();
    ring[1][1] -= 20.0;
    t.import(&[(Role::RiverPolygon, polygon(&ring).as_str())])
        .unwrap();
    assert_eq!(t.fits_status(), StageStatus::Stale, "polygon");

    let mut t = Session::open(&s.to_text()).unwrap();
    let straight: Line = (0..=30).map(|i| at(100.0 * i as f64, 5.0)).collect();
    t.import(&[(Role::Centreline, line(&straight).as_str())])
        .unwrap();
    assert_eq!(t.fits_status(), StageStatus::Stale, "centreline");
}

/// A straight river 400 m wide with a shallow bar, water at 10 m and a DEM reference: a full
/// answer.
fn full_answer() -> Session {
    let ring = [
        at(0.0, -200.0),
        at(3000.0, -200.0),
        at(3000.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0),
    ];
    let centre = [at(0.0, 0.0), at(3000.0, 0.0)];
    let mut s = Session::new("Full", "2026-11-20");
    s.import(&[
        (Role::RiverPolygon, polygon(&ring).as_str()),
        (Role::Centreline, line(&centre).as_str()),
    ])
    .unwrap();
    s.set_water_profile(&[[0.0, 10.0]]).unwrap();
    s.set_bed_model(
        &[[0.0, 4.0], [1200.0, 1.5], [1500.0, 1.5], [3000.0, 4.0]],
        Some("dry"),
        "2026-11-20",
    )
    .unwrap();
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    s.set_dem("dem.tif", b"a stand-in terrain file", &dem);
    let choices = ["PIANC-B".to_string()];
    s.run_fits_with(Some(&choices), Some(&flat)).unwrap();
    assert_eq!(s.fits_result().unwrap()["mode"], "full");
    assert_eq!(s.fits_status(), StageStatus::Fresh);
    s
}

/// Items 4 and 10. A full answer goes stale when what its run reads beyond the plan changes: the
/// bed, the water, the DEM, the bathymetry mark, the sizing's settings and the volumes' settings
/// (it walks each class's sections with them).
#[test]
fn a_full_answer_goes_stale_on_each_thing_its_run_reads() {
    let s = full_answer();
    let cases: [(&str, Value); 5] = [
        ("/inputs/bed_model/points/0/1", json!(3.5)),
        ("/inputs/water_level/points/0/1", json!(10.5)),
        ("/inputs/dem/sha256", json!("0".repeat(64))),
        ("/stages/sizing/settings/draught_m", json!(3.0)),
        ("/stages/volumes/settings", json!({"station_m": 10.0})),
    ];
    for (pointer, v) in cases {
        let t = edited(&s, pointer, v);
        assert_eq!(t.fits_status(), StageStatus::Stale, "{pointer}");
    }
    let mut t = Session::open(&s.to_text()).unwrap();
    t.set_dem_holds_bathymetry(true).unwrap();
    assert_eq!(t.fits_status(), StageStatus::Stale, "bathymetry mark");
    // QA item 13: the optimise stage's pool levels, which the walk reads before the input profile.
    let engine = s.document()["engine_version"].clone();
    let t = edited(
        &s,
        "/stages/optimise",
        json!({"settings": {}, "results": {"profile": [[0.0, 9.0], [3000.0, 9.0]]},
               "depends_on": "0".repeat(64), "engine_version": engine}),
    );
    assert_eq!(t.fits_status(), StageStatus::Stale, "optimise pool levels");
}

/// QA item 13: a full answer read on the reference line goes stale when the bed axis changes (the
/// line's shape no longer matches what made it).
#[test]
fn a_full_answer_goes_stale_when_the_bed_axis_changes() {
    let mut s = bedded_with_volumes();
    let choices = ["PIANC-B".to_string()];
    s.run_fits_with(Some(&choices), Some(&flat)).unwrap();
    assert_eq!(s.fits_result().unwrap()["mode"], "full");
    assert_eq!(s.fits_status(), StageStatus::Fresh);
    let mut doc = s.document().clone();
    doc["stages"]["reference_line"]["depends_on"] = json!("1".repeat(64));
    if let Some(r) = doc["stages"]["reference_line"]["results"].as_object_mut() {
        r.insert("line_basis".into(), json!("1".repeat(64)));
    }
    let t = Session::open(&doc.to_string()).unwrap();
    assert_eq!(t.fits_status(), StageStatus::Stale, "bed axis");
}

/// Item 9. A stored sweep with a field the contract does not know is refused, not read with the
/// field dropped.
#[test]
fn an_envelope_answer_with_an_unknown_field_is_refused() {
    let mut s = river(true);
    s.run_envelope(Settings::default(), true).unwrap();
    let mut r = s.envelope_result().unwrap().clone();
    assert!(serde_json::from_value::<EnvelopeResult>(r.clone()).is_ok());
    r["a_field_from_elsewhere"] = json!(1);
    let e = serde_json::from_value::<EnvelopeResult>(r).unwrap_err();
    assert!(e.to_string().contains("a_field_from_elsewhere"), "{e}");
}

/// Item 6. Opening an older file says that the line moved, and whether it is fresh.
#[test]
fn opening_an_old_file_says_the_line_moved() {
    let s = bedded_with_volumes();
    let old = to_old_place(s.document(), &old_line_hash(s.document()));
    let back = Session::open(&old.to_string()).unwrap();
    let notes = back.open_notes().join("\n");
    assert!(
        notes.contains("reference line") && notes.contains("stages.reference_line"),
        "{notes}"
    );
}

/// What an older OWD hashed for a plan-only fits answer (PR 558's branch, by hand).
fn old_fits_hash(doc: &Value) -> String {
    let inp = &doc["inputs"];
    hash_value(&json!({
        "full": null,
        "left_bank": inp["left_bank"],
        "right_bank": inp["right_bank"],
        "centreline": inp["centreline"],
        "river_polygon": inp["river_polygon"],
        "crs": doc["project"]["crs"],
        "settings": doc["fits"]["settings"],
    }))
}

/// What an older OWD hashed for the envelope (PR 558's branch, by hand).
fn old_envelope_hash(doc: &Value) -> String {
    let inp = &doc["inputs"];
    hash_value(&json!({
        "left_bank": inp["left_bank"],
        "right_bank": inp["right_bank"],
        "river_polygon": inp["river_polygon"],
        "centreline": inp["centreline"],
        "crs": doc["project"]["crs"],
        "reference_line": inp["reference_line"]["depends_on"],
        "custom_vessel": doc["stages"]["sizing"]["settings"]["custom_vessel"],
        "settings": doc["stages"]["envelope"]["settings"],
    }))
}

/// Item 6. A fits answer or a sweep stored by an older OWD (its hash made the old way, and fresh
/// by it) opens stale, and a note says why and what to run.
#[test]
fn an_answer_stored_by_an_older_owd_opens_stale_with_a_note() {
    let mut s = river(false);
    s.run_fits(None).unwrap();
    s.run_envelope(Settings::default(), true).unwrap();
    let mut doc = s.document().clone();
    doc["fits"]["depends_on"] = json!(old_fits_hash(&doc));
    doc["stages"]["envelope"]["depends_on"] = json!(old_envelope_hash(&doc));
    let back = Session::open(&doc.to_string()).unwrap();
    assert_eq!(back.fits_status(), StageStatus::Stale);
    assert_eq!(back.envelope_status(), StageStatus::Stale);
    let notes = back.open_notes().join("\n");
    assert!(
        notes.contains("owd fits") && notes.contains("owd envelope"),
        "{notes}"
    );
}

/// Item 8. A file with the line in both places keeps the stage record and says so.
#[test]
fn a_line_in_both_places_keeps_the_stage_record() {
    let mut s = river(true);
    s.run_reference_line().unwrap();
    let mut doc = s.document().clone();
    let kept = doc["stages"]["reference_line"].clone();
    doc["inputs"]["reference_line"] = json!({
        "line": {"type": "LineString", "coordinates": [at(0.0, 0.0), at(10.0, 0.0)]},
        "parameters": {},
        "depends_on": "an old hash",
        "low_confidence": [],
        "outside_water": [],
        "notes": [],
    });
    let back = Session::open(&doc.to_string()).unwrap();
    assert_eq!(back.document()["stages"]["reference_line"], kept);
    assert!(back.document()["inputs"].get("reference_line").is_none());
    assert!(
        back.open_notes().join("\n").contains("two places"),
        "{:?}",
        back.open_notes()
    );
}

fn stage_input(left: Line, right: Line, ring: Line) -> ReferenceLineIn {
    ReferenceLineIn {
        left_bank: Polyline(left),
        right_bank: Polyline(right),
        river_polygon: RiverPolygon {
            shell: Polyline(ring),
            holes: vec![],
        },
        ..ReferenceLineIn::default()
    }
}

/// Item 10. The stage's other two refusals, by code.
#[test]
fn the_stage_refuses_with_no_water_and_when_the_line_cannot_be_made() {
    let (left, right, _, ring) = river_lines();
    let e = reference_line_stage(&stage_input(left, right, vec![])).unwrap_err();
    assert_eq!(e.code, "REFERENCE_LINE_NO_WATER");
    // Banks 8 m long: the line is trimmed 5 m at each end and nothing is left of it.
    let short_l = vec![at(0.0, 100.0), at(8.0, 100.0)];
    let short_r = vec![at(0.0, -100.0), at(8.0, -100.0)];
    let e = reference_line_stage(&stage_input(short_l, short_r, ring)).unwrap_err();
    assert_eq!(e.code, "REFERENCE_LINE_FAILED", "{e}");
}
