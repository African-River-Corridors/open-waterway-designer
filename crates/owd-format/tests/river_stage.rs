//! The river stage (spec 0017, ticket 3): one parse of the river with its islands, and one bank check,
//! tested alone from the fixtures in `tests/fixtures/river/`. Each fixture holds a `RiverIn`, what
//! the stage must return (or the code it must refuse with), and why. No session is built for them.
//!
//! The rest of the file puts the same facts through the session: the bank check that the import
//! reports and the one `owd check-banks` prints are one answer, and the later stages keep a river's
//! islands.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_format::design::river::{RiverGeometry, RiverIn, river};
use owd_format::design::terrain::dem_sampler;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/river");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let v = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (name, v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `want` is contained in `got`: every key of an object, every element of an array, every number to
/// 1e-9. Returns the first place it is not.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            for (k, wv) in w {
                let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
                contained(gv, wv, &format!("{at}/{k}"))?;
            }
            Ok(())
        }
        (Value::Array(g), Value::Array(w)) if g.len() == w.len() => g
            .iter()
            .zip(w)
            .enumerate()
            .try_for_each(|(i, (a, b))| contained(a, b, &format!("{at}/{i}"))),
        (Value::Number(g), Value::Number(w)) => {
            let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
            if (g - w).abs() <= 1e-9 * w.abs().max(1.0) {
                Ok(())
            } else {
                Err(format!("{at}: {g} is not {w}"))
            }
        }
        (g, w) if g == w => Ok(()),
        (g, w) => Err(format!("{at}: got {g}, want {w}")),
    }
}

fn polygon_area(g: &RiverGeometry) -> f64 {
    g.polygon.as_ref().expect("a polygon").area_m2()
}

#[test]
fn every_fixture_gives_what_it_says() {
    let all = fixtures();
    assert!(all.len() >= 18, "the fixtures are there: {}", all.len());
    for (name, f) in all.clone() {
        assert!(
            f["label"]
                .as_str()
                .is_some_and(|l| l.starts_with("Label: ")),
            "{name}: a fixture carries a label"
        );
        assert!(
            f["why"].as_str().is_some_and(|w| w.len() > 20),
            "{name}: says why"
        );
        let input: RiverIn = serde_json::from_value(f["input"].clone())
            .unwrap_or_else(|e| panic!("{name}: the input reads: {e}"));
        let want = &f["expect"];
        let got = river(&input);
        if let Some(code) = want["error"].as_str() {
            let e = got.expect_err(&format!("{name}: refused"));
            assert_eq!(e.code, code, "{name}: {e}");
            assert_eq!(e.stage, "river", "{name}");
            continue;
        }
        let g = got.unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = serde_json::to_value(&g).unwrap();
        if let Some(sub) = want.get("output") {
            contained(&out, sub, "output").unwrap_or_else(|m| panic!("{name}: {m}"));
        }
        if let Some(a) = want["polygon_area_m2"].as_f64() {
            assert!(
                (polygon_area(&g) - a).abs() < 1e-6,
                "{name}: area {}",
                polygon_area(&g)
            );
        }
        if let Some(n) = want["holes"].as_u64() {
            assert_eq!(g.polygon.as_ref().unwrap().holes.len() as u64, n, "{name}");
        }
        if let Some(b) = want["length_between"].as_array() {
            let (lo, hi) = (b[0].as_f64().unwrap(), b[1].as_f64().unwrap());
            assert!(
                g.length_m > lo && g.length_m < hi,
                "{name}: length {}",
                g.length_m
            );
        }
        if let Some(y) = want["centreline_y"].as_f64() {
            assert!(
                g.centreline.0.iter().all(|p| (p[1] - y).abs() < 1e-6),
                "{name}"
            );
        }
        for key in [
            "centreline_first",
            "centreline_last",
            "left_first",
            "left_last",
            "right_first",
            "right_last",
        ] {
            if let Some(p) = want.get(key) {
                let (part, end) = key.split_once('_').unwrap();
                let line = match part {
                    "centreline" => &g.centreline.0,
                    "left" => &g.banks.as_ref().expect("banks").left.0,
                    _ => &g.banks.as_ref().expect("banks").right.0,
                };
                let at = if end == "first" {
                    line[0]
                } else {
                    line[line.len() - 1]
                };
                contained(&json!(at), p, key).unwrap_or_else(|m| panic!("{name}: {m}"));
            }
        }
        if let Some(other) = want["same_output_as"].as_str() {
            let o = &all
                .iter()
                .find(|(n, _)| n == other)
                .expect("the other fixture")
                .1;
            let oin: RiverIn = serde_json::from_value(o["input"].clone()).unwrap();
            // The same geometry; `reversed` says only whether this input had to be turned.
            let (mut a, mut b) = (river(&oin).unwrap(), g.clone());
            (a.reversed, b.reversed) = (false, false);
            assert_eq!(a, b, "{name}: the same design as {other}");
        }
        if want["banks_cross_is_some"] == true {
            assert!(g.banks_cross.is_some(), "{name}: the crossing is reported");
        }
    }
}

#[test]
fn the_stage_does_not_depend_on_the_order_the_bank_is_given_in() {
    // The same river, the right bank given both ways round: one answer.
    let all = fixtures();
    let get = |n: &str| all.iter().find(|(k, _)| k == n).unwrap().1["input"].clone();
    let against: RiverIn = serde_json::from_value(get("banks-in-opposite-directions")).unwrap();
    let mut with = against.clone();
    with.right_bank.as_mut().unwrap().0.reverse();
    assert_eq!(river(&against).unwrap(), river(&with).unwrap());
}

#[test]
fn a_geographic_river_is_checked_in_metres() {
    // Two banks 100 m apart, 2 km long, given as longitude and latitude on the equator. Checked as
    // degrees they would be 0.0009 apart and read as one point; checked in metres they pass.
    let per_deg = 111_319.49;
    let lon = |m: f64| m / per_deg;
    let line = |y: f64| -> Vec<[f64; 2]> {
        (0..=20)
            .map(|i| [lon(100.0 * f64::from(i)), lon(y)])
            .collect()
    };
    let input: RiverIn = serde_json::from_value(json!({
        "left_bank": line(50.0), "right_bank": line(-50.0), "crs_epsg": 4326
    }))
    .unwrap();
    let g = river(&input).unwrap();
    assert!(g.bank_checks.is_empty(), "{:?}", g.bank_checks);
    // The line between the banks is smoothed, which trims its ends a little; in degrees it would be
    // 0.018 long.
    assert!(
        g.length_m > 1800.0 && g.length_m < 2000.0,
        "the length is in metres: {}",
        g.length_m
    );
}

// --- one bank check -----------------------------------------------------------------------------

const UTM30: &str = r#"{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

fn line_json(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString","crs":{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// A straight study whose right bank dives north across the centreline (but not past the left bank)
/// for a stretch: a local side fault. The import refuses banks that cross, so these do not.
fn faulty_study() -> (Session, Vec<String>) {
    let n = 40;
    let left: Vec<Pt> = (0..=n)
        .map(|i| [X0 + 50.0 * f64::from(i), Y0 + 50.0])
        .collect();
    let right: Vec<Pt> = (0..=n)
        .map(|i| {
            let x = 50.0 * f64::from(i);
            let dive = if (900.0..=1100.0).contains(&x) {
                80.0
            } else {
                0.0
            };
            [X0 + x, Y0 - 50.0 + dive]
        })
        .collect();
    let mut s = Session::new("Bank check", "2026-11-19");
    let report = s
        .import(&[
            (Role::LeftBank, &line_json(&left)),
            (Role::RightBank, &line_json(&right)),
        ])
        .unwrap();
    (s, report.warnings)
}

#[test]
fn the_import_warnings_and_the_check_come_from_one_report() {
    let (s, warnings) = faulty_study();
    let g = s.river_geometry().unwrap();
    assert!(!g.bank_checks.is_empty(), "the fault is found");
    // The import's warning lines are the stage's report, worded: the session keeps no second check.
    assert_eq!(warnings, g.warnings(false));
    // And every failed check named in the list is worded in the warnings, with its station count.
    let mut names: Vec<&str> = g
        .bank_checks
        .iter()
        .flat_map(|c| c.checks.iter().map(String::as_str))
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let n = g
            .bank_checks
            .iter()
            .filter(|c| c.checks.iter().any(|x| x == name))
            .count();
        assert!(
            warnings
                .iter()
                .any(|l| l.contains(&format!("`{name}` check at {n} station"))),
            "{name} x{n} in {warnings:?}"
        );
    }
}

#[test]
fn a_clean_study_has_no_failed_station() {
    let left: Vec<Pt> = (0..=40)
        .map(|i| [X0 + 50.0 * f64::from(i), Y0 + 50.0])
        .collect();
    let right: Vec<Pt> = (0..=40)
        .map(|i| [X0 + 50.0 * f64::from(i), Y0 - 50.0])
        .collect();
    let mut s = Session::new("Clean", "2026-11-19");
    s.import(&[
        (Role::LeftBank, &line_json(&left)),
        (Role::RightBank, &line_json(&right)),
    ])
    .unwrap();
    let g = s.river_geometry().unwrap();
    assert!(g.bank_checks.is_empty() && g.banks_cross.is_none());
    assert!(g.warnings(true).is_empty());
}

// --- one parse of the polygon, with holes ----------------------------------------------------------

/// A straight 2 km river 400 m wide with an island across the middle of the channel. The terrain is
/// flat and above the water, so the cut is dry wherever the island stands.
fn volumes_with(islands: &[Vec<Pt>]) -> (f64, f64) {
    let centre: Vec<Pt> = (0..=40).map(|i| [X0 + 50.0 * f64::from(i), Y0]).collect();
    let mut s = Session::new("Island", "2026-11-19");
    s.import(&[(Role::Centreline, &line_json(&centre))])
        .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let rect = |x0: f64, x1: f64, y0: f64, y1: f64| -> Vec<Pt> {
        vec![
            [X0 + x0, Y0 + y0],
            [X0 + x1, Y0 + y0],
            [X0 + x1, Y0 + y1],
            [X0 + x0, Y0 + y1],
            [X0 + x0, Y0 + y0],
        ]
    };
    let mut rings = vec![rect(-100.0, 2100.0, -200.0, 200.0)];
    rings.extend(islands.iter().cloned());
    s.import(&[(
        Role::RiverPolygon,
        &format!(
            r#"{{"type":"Polygon","crs":{UTM30},"coordinates":{}}}"#,
            json!(rings)
        ),
    )])
    .unwrap();
    s.set_water_profile(&[[0.0, 8.0]]).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    let dem = Dem {
        width: 30,
        height: 10,
        data: vec![10.0; 300],
        geotransform: [100.0, 0.0, X0 - 100.0, 0.0, -100.0, Y0 + 500.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    s.set_dem("dem.tif", b"flat", &dem);
    let sampler = dem_sampler(&dem, Some(32630)).unwrap();
    s.run_volumes(&json!({}), Some(&sampler)).unwrap();
    let r = &s.document()["stages"]["volumes"]["results"]["options"][0];
    let num = |k: &str| r[k].as_f64().unwrap_or_else(|| panic!("{k} in {r}"));
    (num("dredge_m3"), num("excav_m3"))
}

#[test]
fn an_island_across_the_channel_is_dry_not_water_in_the_volumes() {
    let island = vec![
        [X0 + 500.0, Y0 - 100.0],
        [X0 + 1500.0, Y0 - 100.0],
        [X0 + 1500.0, Y0 + 100.0],
        [X0 + 500.0, Y0 + 100.0],
        [X0 + 500.0, Y0 - 100.0],
    ];
    let (dredge_open, dry_open) = volumes_with(&[]);
    let (dredge_island, dry_island) = volumes_with(&[island]);
    assert!(
        dredge_island < dredge_open,
        "the stations on the island are not dredged: {dredge_island} < {dredge_open}"
    );
    assert!(
        dry_island > dry_open,
        "they are dry cut instead: {dry_island} > {dry_open}"
    );
}

// --- chainage from the mouth --------------------------------------------------------------------------

/// A winding river (a sine curve) with banks and a falling water profile, given from either end.
fn winding(reverse_all: bool, mouth: Value) -> RiverIn {
    let n = 60;
    let at = |i: i32, off: f64| -> [f64; 2] {
        let x = 50.0 * f64::from(i);
        let y = 120.0 * libm::sin(x / 400.0);
        // The bank is 40 m to the side of the line, along the normal of the sine curve.
        let slope = 120.0 / 400.0 * libm::cos(x / 400.0);
        let norm = libm::hypot(1.0, slope);
        [x - off * slope / norm, y + off / norm]
    };
    let line = |off: f64| -> Vec<[f64; 2]> { (0..=n).map(|i| at(i, off)).collect() };
    let (mut c, mut l, mut r) = (line(0.0), line(40.0), line(-40.0));
    let length = owd_core::geom::linear::length(&c);
    // The level falls 5 m over the line, from the start to the end.
    let mut profile = vec![[0.0, 15.0], [length, 10.0]];
    if reverse_all {
        c.reverse();
        l.reverse();
        r.reverse();
        profile = profile
            .iter()
            .rev()
            .map(|[ch, lv]| [length - ch, *lv])
            .collect();
    }
    serde_json::from_value(json!({
        "centreline": c, "left_bank": l, "right_bank": r, "water_profile": profile,
        "mouth": mouth, "crs_epsg": 32630
    }))
    .unwrap()
}

#[test]
fn a_winding_river_gives_one_design_whichever_end_it_was_digitised_from() {
    // The river as digitised downstream, and the same river digitised upstream, with its banks and its
    // profile. The mouth is the lower end in both, found two ways: by point and by the profile.
    let end = json!({"point": [3000.0, 120.0 * libm::sin(3000.0 / 400.0)]});
    for mouth in [end, json!("downstream")] {
        let (mut fwd, mut back) = (
            river(&winding(false, mouth.clone())).unwrap(),
            river(&winding(true, mouth.clone())).unwrap(),
        );
        assert_ne!(fwd.reversed, back.reversed, "one of them had to be turned");
        (fwd.reversed, back.reversed) = (false, false);
        // The length is summed from the other end, so its last bit may differ: compare it by
        // tolerance (ADR 0005), then the rest exactly.
        assert!(
            (fwd.length_m - back.length_m).abs() <= 1e-9 * fwd.length_m,
            "{mouth}"
        );
        back.length_m = fwd.length_m;
        assert!(fwd == back, "{mouth}: the two designs differ");
        // Chainage 0 is the mouth, which is the low end: the level rises with chainage.
        let w = fwd.water_profile.as_ref().unwrap();
        assert!(
            w.level_at(owd_format::types::Chainage(0.0))
                < w.level_at(owd_format::types::Chainage(fwd.length_m))
        );
        assert!(fwd.bank_checks.is_empty(), "{:?}", fwd.bank_checks);
    }
}

#[test]
fn a_bank_check_that_cannot_run_is_a_warning_with_its_reason_not_silence() {
    // Two banks that are one point repeated give the river stage no centreline to build. The import
    // keeps them (it refuses only banks that cross), and the check cannot run: that shows as a warning.
    let mut s = Session::new("x", "2026-11-20");
    let pt = |x: f64, y: f64| -> String {
        format!(r#"{{"type":"LineString","crs":{UTM30},"coordinates":[[{x},{y}],[{x},{y}]]}}"#)
    };
    let report = s
        .import(&[
            (Role::LeftBank, &pt(X0, Y0 + 10.0)),
            (Role::RightBank, &pt(X0, Y0 - 10.0)),
        ])
        .unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("bank checks could not run")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn the_bank_checks_and_the_import_warnings_read_one_input_without_the_polygon() {
    // A study with a river polygon: the bank checks leave it out, in the import's warnings and in
    // `owd check-banks` alike (both read `Session::bank_check_input`), so a bad polygon cannot hide
    // a bank warning and the two cannot differ.
    let mut s = Session::new("x", "2026-11-20");
    let (left, right): (Vec<Pt>, Vec<Pt>) = (
        (0..=20)
            .map(|i| [X0 + 50.0 * f64::from(i), Y0 + 50.0])
            .collect(),
        (0..=20)
            .map(|i| [X0 + 50.0 * f64::from(i), Y0 - 50.0])
            .collect(),
    );
    let ring = vec![
        [X0 - 10.0, Y0 - 100.0],
        [X0 + 1100.0, Y0 - 100.0],
        [X0 + 1100.0, Y0 + 100.0],
        [X0 - 10.0, Y0 + 100.0],
        [X0 - 10.0, Y0 - 100.0],
    ];
    s.import(&[
        (Role::LeftBank, &line_json(&left)),
        (Role::RightBank, &line_json(&right)),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon","crs":{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    assert!(s.river_input().river_polygon.is_some());
    assert!(s.bank_check_input().river_polygon.is_none());
    assert!(s.bank_check_geometry().unwrap().polygon.is_none());
}
