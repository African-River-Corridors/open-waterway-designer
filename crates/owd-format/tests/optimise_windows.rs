//! The optimise stage's window method (OPT-5) on the `optimise_pools` example: a setting off by
//! default that changes nothing when off, refused in plain words when wrong, and when on a design
//! line made window by window that holds R_min, does not cross itself, is never dearer than the line
//! it started from, records every window, and carries the pools of an earlier pool-lattice run.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_format::design::{AlignMethod, OptimiseSettings, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn params() -> RiverParams {
    RiverParams {
        length_m: 3000.0,
        wavelength_m: 2400.0,
        amplitude_m: 120.0,
        hill_at: [1500.0, -60.0],
        hill_height_m: 10.0,
        hill_radius_m: 80.0,
        ..RiverParams::default()
    }
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = synthetic_river(params()).unwrap();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

fn design() -> Session {
    let r = synthetic_river(params()).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let centre: Vec<Pt> = r.centreline.iter().map(shift).collect();
    let mut s = Session::new("Windows", "2026-11-26");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
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
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.set_dem(
        "dem.tif",
        b"dem",
        &Dem {
            width: 2,
            height: 2,
            data: vec![0.0; 4],
            geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
            nodata: None,
            epsg: Some(32630),
        },
    );
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s
}

/// A coarse window optimiser, so the test runs in seconds in a debug build.
fn coarse() -> Value {
    json!({
        "method": "windows",
        "length_weight": 2.0,
        "windows": {
            "n_heading": 32, "pitch_m": 16.0, "cell_m": 4.0, "n_radii": 6,
            "search_rake_step_m": 4.0, "search_ds_m": 12.0,
            "polish_spacings_m": [150.0, 300.0], "polish_station_m": 30.0,
            "window_time_s": 600.0, "max_expand": 400000, "threads": 2,
            "window_target_m": 1200.0, "window_min_m": 700.0,
        },
    })
}

#[test]
fn the_method_is_lattice_by_default_and_the_window_settings_are_checked() {
    let s = design();
    let mut t = s.clone();
    t.set_optimise_settings(&json!({})).unwrap();
    let st = &t.document()["stages"]["optimise"]["settings"];
    assert!(st.get("method").is_none());
    assert!(st.get("windows").is_none());
    // Named at its default, still not written.
    t.set_optimise_settings(&json!({"method": "lattice"}))
        .unwrap();
    assert!(
        t.document()["stages"]["optimise"]["settings"]
            .get("method")
            .is_none()
    );
    // On, the method and every window setting are written.
    t.set_optimise_settings(&json!({"method": "windows", "windows": {"window_target_m": 1200.0}}))
        .unwrap();
    let st = &t.document()["stages"]["optimise"]["settings"];
    assert_eq!(st["method"], json!("windows"));
    assert_eq!(st["windows"]["window_target_m"], json!(1200.0));
    assert_eq!(st["windows"]["n_heading"], json!(64));
    assert_eq!(OptimiseSettings::default().windows.window_max_m, 2500.0);
    // Refused by name, in plain words, changing nothing.
    for (bad, says) in [
        (json!({"method": "chunks"}), "`method` must be `lattice`"),
        (
            json!({"windows": {"window_size": 3}}),
            "no setting `window_size`",
        ),
        (json!({"windows": {"n_heading": "many"}}), "wrong kind"),
        (
            json!({"windows": {"window_target_m": 3000.0}}),
            "window_min_m",
        ),
        (
            json!({"windows": {"stay_in_river": "maybe"}}),
            "`stay_in_river`",
        ),
        (
            json!({"windows": {"adopt_threshold": 1.5}}),
            "`adopt_threshold`",
        ),
    ] {
        let mut u = s.clone();
        let e = u.set_optimise_settings(&bad).unwrap_err().to_string();
        assert!(e.contains(says), "{bad}: {e}");
        assert_eq!(u.to_text(), s.to_text());
    }
}

#[test]
fn the_window_method_makes_a_line_that_holds_r_min_and_is_never_dearer() {
    let mut s = design();
    let start: Vec<Pt> = owd_format::design::design_line::alignment_line_of(s.document())
        .unwrap()
        .line
        .0;
    s.run_optimise(&coarse(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    let doc = s.document().clone();
    let res = &doc["stages"]["optimise"]["results"];
    assert_eq!(res["method"], json!("windows"));
    let line = &res["design_line"];
    assert_eq!(line["method"], json!("windows"));
    assert_eq!(s.final_line().unwrap().method, "windows");
    let d = &line["details"];
    // Every window recorded with a reason, its seams and its numbers.
    let windows = d["windows"].as_array().unwrap();
    assert!(windows.len() >= 2, "{d}");
    for w in windows {
        assert!(w["reason"].is_string() && w["adopted"].is_string(), "{w}");
        assert!(w["full_start"]["cost"].is_number(), "{w}");
        assert!(w["start_seam"]["kind"].is_string());
    }
    assert!(windows.iter().any(|w| w["pass"] == json!(2)));
    // R_min, no loop, never dearer end to end.
    assert_eq!(
        line["checks"]["radius_ok"],
        json!(true),
        "{}",
        line["checks"]
    );
    assert_eq!(line["checks"]["no_self_crossing"]["ok"], json!(true));
    let t = &d["totals"];
    let (a, b) = (
        t["start"]["cost"].as_f64().unwrap(),
        t["result"]["cost"].as_f64().unwrap(),
    );
    assert!(b <= a * 1.005, "{a} -> {b}");
    // The ends of the line are the starting line's.
    let got: Vec<Pt> = line["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect();
    assert!(dist(got[0], start[0]) < 1e-6);
    assert!(dist(got[got.len() - 1], start[start.len() - 1]) < 1e-6);
    // Saved and opened again: the schema takes every value the stage stored.
    let opened = Session::open(&s.to_text()).unwrap();
    assert_eq!(opened.validate(), Vec::<String>::new());
    // The volumes stage reads it.
    s.run_volumes(&Value::Null, Some(&terrain)).unwrap();
    assert_eq!(
        s.document()["stages"]["volumes"]["results"]["id"],
        json!("windows")
    );
    // Run again: the same document.
    let mut again = design();
    again.run_optimise(&coarse(), Some(&terrain)).unwrap();
    let strip = |d: &Value| {
        let mut d = d["stages"]["optimise"]["results"]["design_line"]["details"].clone();
        d["seconds"] = Value::Null;
        for w in d["windows"].as_array_mut().unwrap() {
            w["seconds"] = Value::Null;
        }
        d
    };
    assert_eq!(strip(again.document()), strip(&doc));
}

#[test]
fn the_window_method_keeps_the_pools_of_a_fresh_lattice_run_and_its_weirs_are_seams() {
    let mut s = design();
    s.run_optimise(
        &json!({
            "candidate_sites_m": [1500.0], "lifts_m": [0.5], "terminal_min_level_m": 0.5,
            "n_heading": 16, "pitch_m": 20.0, "cell_m": 5.0, "corridor_half_m": 180.0,
            "rake_half_m": 80.0, "rake_step_m": 5.0, "ds_step_m": 10.0,
        }),
        Some(&terrain),
    )
    .unwrap();
    let before = s.document()["stages"]["optimise"]["results"].clone();
    let weirs: Vec<f64> = before["cascade"]["weirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["chainage_m"].as_f64().unwrap())
        .collect();
    let mut settings = coarse();
    settings["candidate_sites_m"] = json!([1500.0]);
    settings["lifts_m"] = json!([0.5]);
    s.run_optimise(&settings, Some(&terrain)).unwrap();
    let after = &s.document()["stages"]["optimise"]["results"];
    assert_eq!(after["pools"], before["pools"]);
    assert_eq!(after["profile"], before["profile"]);
    let seams = after["design_line"]["details"]["seams"].as_array().unwrap();
    for w in &weirs {
        assert!(
            seams
                .iter()
                .any(|s| s["kind"] == json!("weir")
                    && (s["at_m"].as_f64().unwrap() - w).abs() < 1e-6),
            "weir at {w} not a seam: {seams:?}"
        );
    }
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])).sqrt()
}
