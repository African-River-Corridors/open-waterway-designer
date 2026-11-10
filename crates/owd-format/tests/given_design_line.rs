//! A design line can be an input: a line from a file becomes a design line option with the method
//! `given`. It gets the radius check every option gets. A line under the minimum radius is
//! reported, not refused. The volumes stage runs on it like on any option.
//!
//! The session tests use a straight synthetic river in UTM 30N. The volumes test uses the web
//! example and its terrain file.
//!
//! Label: property

use owd_core::alignment::radius_check::min_radius_on_chord;
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus, Terrain};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

type Pt = [f64; 2];

/// A straight river 3000 m long and 600 m wide, sized for a CEMT IV vessel. No design line yet.
fn design() -> Session {
    let centre: Vec<Pt> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let mut s = Session::new("A given line", "2026-11-10");
    s.import(&[
        (Role::Centreline, &utm_line(&centre)),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

/// A line as GeoJSON in UTM 30N, the project CRS of the design.
fn utm_line(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn straight() -> Vec<Pt> {
    vec![[X0 + 200.0, Y0 + 5.0], [X0 + 2800.0, Y0 + 5.0]]
}

/// A line with a square corner: no radius at all.
fn cornered() -> Vec<Pt> {
    vec![
        [X0 + 200.0, Y0],
        [X0 + 1500.0, Y0],
        [X0 + 1500.0, Y0 + 1300.0],
    ]
}

/// Two straights of 600 m joined by an arc of `radius_m` that turns 60 degrees, a point every 5 m.
fn bend(radius_m: f64) -> Vec<Pt> {
    let mut pts: Vec<Pt> = (0..120)
        .map(|i| [X0 + 200.0 + 5.0 * f64::from(i), Y0])
        .collect();
    let (cx, cy) = (X0 + 800.0, Y0 + radius_m);
    let sweep = std::f64::consts::FRAC_PI_3;
    let n = (radius_m * sweep / 5.0).ceil() as u32;
    for i in 0..=n {
        let a = sweep * f64::from(i) / f64::from(n);
        pts.push([cx + radius_m * a.sin(), cy - radius_m * a.cos()]);
    }
    let end = *pts.last().unwrap();
    for i in 1..=120 {
        let d = 5.0 * f64::from(i);
        pts.push([end[0] + d * sweep.cos(), end[1] + d * sweep.sin()]);
    }
    pts
}

fn r_min(s: &Session) -> f64 {
    s.document()["stages"]["sizing"]["results"]["min_bend_radius_m"]
        .as_f64()
        .unwrap()
}

fn option(s: &Session, id: &str) -> Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap_or_else(|| panic!("the design has no option `{id}`"))
        .clone()
}

fn line_of(option: &Value) -> Vec<Pt> {
    option["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn ids(s: &Session) -> Vec<String> {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_straight_given_line_becomes_an_option_that_holds_the_radius() {
    let mut s = design();
    let id = s
        .import_design_line(&utm_line(&straight()), "Study line")
        .unwrap();
    assert_eq!(id, "given-study-line");
    assert_eq!(ids(&s), ["given-study-line"], "no other option is made");
    let o = option(&s, &id);
    assert_eq!(o["method"], "given");
    assert_eq!(o["name"], "Study line");
    assert_eq!(o["line"]["type"], "LineString");
    assert_eq!(line_of(&o), straight(), "the line is in the project CRS");
    assert_eq!(o["checks"]["r_min_m"], json!(r_min(&s)));
    assert_eq!(o["checks"]["radius_ok"], json!(true));
    assert_eq!(o["checks"]["radius_check_chord_m"], json!(r_min(&s) / 3.0));
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
}

#[test]
fn a_line_under_the_minimum_radius_is_reported_not_refused() {
    let mut s = design();
    let id = s
        .import_design_line(&utm_line(&cornered()), "Cornered")
        .expect("a line under the minimum radius is still imported");
    let o = option(&s, &id);
    assert_eq!(line_of(&o), cornered());
    assert_eq!(o["checks"]["radius_ok"], json!(false));
    let measured = o["checks"]["measured_min_radius_m"]
        .as_f64()
        .expect("the measured radius is a number");
    let want = min_radius_on_chord(&cornered(), r_min(&s) / 3.0).unwrap();
    assert!(
        (measured - want).abs() < 1e-6,
        "the radius is read on a chord of a third of the minimum radius: got {measured}, want {want}"
    );
    assert!(measured < 0.5 * r_min(&s));
    assert_eq!(
        owd_format::design::export::failed_checks(&o),
        ["R_min"],
        "the failed check rides with the option"
    );
}

#[test]
fn a_given_line_in_longitude_and_latitude_is_stored_in_the_project_crs() {
    let mut s = design();
    let (_, utm) = s.water_reference().unwrap();
    let lon_lat: Vec<Pt> = straight()
        .iter()
        .map(|p| utm.inverse(p[0], p[1]).unwrap())
        .collect();
    let text = format!(
        r#"{{"type":"LineString","coordinates":{}}}"#,
        json!(lon_lat)
    );
    let id = s.import_design_line(&text, "From WGS84").unwrap();
    for (got, want) in line_of(&option(&s, &id)).iter().zip(straight()) {
        assert!(
            (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
            "got {got:?}, want {want:?}"
        );
    }
}

#[test]
fn the_same_name_replaces_its_line_and_another_name_adds_one() {
    let mut s = design();
    let a = s
        .import_design_line(&utm_line(&straight()), "Study line")
        .unwrap();
    let again = s
        .import_design_line(&utm_line(&cornered()), "Study line")
        .unwrap();
    assert_eq!(a, again);
    assert_eq!(ids(&s), [a.clone()]);
    assert_eq!(line_of(&option(&s, &a)), cornered());
    let b = s
        .import_design_line(&utm_line(&straight()), "Second line")
        .unwrap();
    assert_eq!(ids(&s), [a, b]);
}

#[test]
fn a_given_line_that_cannot_be_checked_is_refused_and_nothing_changes() {
    let input = |r: Result<String, DesignError>, word: &str| match r {
        Err(DesignError::Input(m)) => assert!(m.contains(word), "the message lacks `{word}`: {m}"),
        other => panic!("expected a refusal with `{word}`, got {other:?}"),
    };
    let mut s = design();
    let before = s.to_text();
    input(s.import_design_line(&utm_line(&straight()), "  "), "name");
    let polygon =
        format!(r#"{{"type":"Polygon"{UTM30},"coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}"#);
    input(s.import_design_line(&polygon, "A polygon"), "LineString");
    input(s.import_design_line("not GeoJSON", "Text"), "JSON");
    assert_eq!(s.to_text(), before);

    // The check reads the minimum radius of the sizing: with no sizing there is none.
    let mut unsized_design = Session::new("No sizing", "2026-11-10");
    unsized_design
        .import(&[(Role::Centreline, &utm_line(&straight()))])
        .unwrap();
    let before = unsized_design.to_text();
    input(
        unsized_design.import_design_line(&utm_line(&straight()), "Study line"),
        "sizing",
    );
    assert_eq!(unsized_design.to_text(), before);
}

#[test]
fn a_new_sizing_checks_the_given_line_again_and_keeps_it() {
    // An arc of 290 m holds the 255 m of a CEMT IV vessel, and not the 330 m of a CEMT Va.
    let mut s = design();
    let id = s
        .import_design_line(&utm_line(&bend(290.0)), "Bend")
        .unwrap();
    let before = option(&s, &id);
    assert_eq!(before["checks"]["radius_ok"], json!(true));
    let old_r = r_min(&s);

    s.set_vessel("CEMT_Va", Some(2.5)).unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    let ran = s.run_stale_with(None).unwrap();
    assert_eq!(ran, [Stage::Sizing, Stage::Alignment]);
    assert!(r_min(&s) > old_r);

    assert_eq!(ids(&s), [id.clone()], "the run makes no other option");
    let after = option(&s, &id);
    assert_eq!(after["line"], before["line"], "the given line is kept");
    assert_eq!(after["name"], before["name"]);
    assert_eq!(after["checks"]["r_min_m"], json!(r_min(&s)));
    assert_eq!(
        after["checks"]["radius_check_chord_m"],
        json!(r_min(&s) / 3.0)
    );
    assert_eq!(after["checks"]["radius_ok"], json!(false));
    let measured = after["checks"]["measured_min_radius_m"].as_f64().unwrap();
    assert!((280.0..300.0).contains(&measured), "measured {measured}");
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
}

#[test]
fn the_alignment_methods_and_a_given_line_leave_each_other_alone() {
    let mut s = design();
    let rolling = json!({"use_water": false});
    s.align(AlignMethod::RollingCircle, &rolling, None).unwrap();
    let id = s
        .import_design_line(&utm_line(&straight()), "Study line")
        .unwrap();
    assert_eq!(ids(&s), ["rolling-circle", "given-study-line"]);
    let given = option(&s, &id);

    // The method run again replaces its own option only.
    s.align(AlignMethod::RollingCircle, &rolling, None).unwrap();
    assert_eq!(ids(&s), ["rolling-circle", "given-study-line"]);
    assert_eq!(option(&s, &id), given);

    // The alignment stage run again keeps both.
    s.set_vessel("CEMT_Va", Some(2.5)).unwrap();
    s.run_stale_with(None).unwrap();
    assert_eq!(ids(&s), ["rolling-circle", "given-study-line"]);
    assert_eq!(option(&s, &id)["line"], given["line"]);
    assert_eq!(option(&s, &id)["checks"]["r_min_m"], json!(r_min(&s)));
    assert_eq!(
        option(&s, "rolling-circle")["checks"]["r_min_m"],
        json!(r_min(&s))
    );
}

fn web(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web")
        .join(name)
}

#[test]
fn the_volumes_stage_runs_on_a_given_line_with_terrain() {
    let text = std::fs::read_to_string(web("river-example.owd")).unwrap();
    let dem =
        owd_format::geotiff::read_bytes(&std::fs::read(web("river-dem.tif")).unwrap()).unwrap();
    let terrain = dem_sampler(&dem, Some(32630)).unwrap();
    let mut s = Session::open(&text).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    let rows = |s: &Session| -> Vec<Value> {
        s.document()["stages"]["volumes"]["results"]["options"]
            .as_array()
            .unwrap()
            .clone()
    };
    // The example's own option, on this engine and this terrain.
    s.run_stage_with(Stage::Volumes, Some(&terrain as Terrain))
        .unwrap();
    let before = rows(&s);
    assert_eq!(before.len(), 1);

    // A straight line between two points of the example's river centreline.
    let centre = line_of(&json!({"line": s.document()["inputs"]["centreline"]}));
    let given = vec![centre[centre.len() / 4], centre[centre.len() / 2]];
    let id = s
        .import_design_line(&utm_line(&given), "Straight across")
        .unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert_eq!(
        s.stage_status(Stage::Volumes),
        StageStatus::Stale,
        "a new option makes the volumes stale"
    );

    s.run_stage_with(Stage::Volumes, Some(&terrain as Terrain))
        .unwrap();
    let after = rows(&s);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], before[0], "the other option keeps its numbers");
    let row = &after[1];
    assert_eq!(row["id"], json!(id));
    assert_eq!(row["name"], "Straight across");
    assert_eq!(row["radius_ok"], json!(true));
    assert!(row["stations"].as_u64().unwrap() > 10);
    for key in ["dredge_m3", "excav_m3", "total_m3", "max_face_h_m"] {
        let v = row[key]
            .as_f64()
            .unwrap_or_else(|| panic!("`{key}` is not a number"));
        assert!(v.is_finite() && v >= 0.0, "`{key}` is {v}");
    }
    assert!(row["total_m3"].as_f64().unwrap() > 0.0);
    assert!(row.get("no_feasible_line").is_none());
    // The sections of the given line can be drawn like any option's.
    let walk = s.option_section_walk(&id, &terrain).unwrap();
    assert_eq!(
        walk.stations.len() as u64,
        row["stations"].as_u64().unwrap()
    );
}
