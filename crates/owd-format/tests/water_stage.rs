//! The water stage (spec 0017, ticket 8): the water level along the design line, tested alone from
//! the fixtures in `tests/fixtures/water/`. Each fixture holds a `WaterIn`, what the stage must
//! return (or the code it must refuse with), levels read off the answer, and why. No session is
//! built for them.
//!
//! The rest of the file puts the stage through the session: the volumes stage stores it, its
//! levels are the levels the volumes walk reads, and its status is worked out on load.
//!
//! Label: property

use owd_format::design::water_stage::{DEFAULT_STEP_M, DesignWater, WaterIn, WaterSource, water};
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use owd_format::types::Chainage;
use serde_json::{Value, json};
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/water");
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

/// `want` is contained in `got`: every key of an object, every element of an array (same length),
/// every number to 1e-9 relative. Returns the first place it is not.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            for (k, wv) in w {
                let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
                contained(gv, wv, &format!("{at}/{k}"))?;
            }
            Ok(())
        }
        (Value::Array(g), Value::Array(w)) => {
            if g.len() != w.len() {
                return Err(format!("{at}: {} items, want {}", g.len(), w.len()));
            }
            g.iter()
                .zip(w)
                .enumerate()
                .try_for_each(|(i, (a, b))| contained(a, b, &format!("{at}/{i}")))
        }
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

#[test]
fn every_fixture_gives_its_answer_or_its_code() {
    let all = fixtures();
    assert!(all.len() >= 10, "the fixtures are there: {}", all.len());
    for (name, f) in all {
        let input: WaterIn = serde_json::from_value(f["input"].clone())
            .unwrap_or_else(|e| panic!("{name}: the input reads as a WaterIn: {e}"));
        let got = water(&input);
        let expect = &f["expect"];
        if let Some(code) = expect["error"].as_str() {
            let e = got.err().unwrap_or_else(|| panic!("{name}: expected {code}"));
            assert_eq!((e.stage.as_str(), e.code.as_str()), ("water", code), "{name}");
            continue;
        }
        let out = got.unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        let v = serde_json::to_value(&out).unwrap();
        contained(&v, &expect["output"], "").unwrap_or_else(|e| panic!("{name}: {e}\n{v:#}"));
        for pair in expect["levels_at"].as_array().into_iter().flatten() {
            let (s, want) = (pair[0].as_f64().unwrap(), pair[1].as_f64().unwrap());
            let level = out.level_at(Chainage(s));
            assert!(
                (level - want).abs() < 1e-9,
                "{name}: the level at {s} m is {level}, want {want}"
            );
        }
        // The answer reads back as itself.
        let back: DesignWater = serde_json::from_value(v).unwrap();
        assert_eq!(back, out, "{name}");
    }
}

#[test]
fn the_stage_has_a_schema_file() {
    assert!(
        owd_format::stage::contracts()
            .iter()
            .any(|c| c.stage == "water"),
        "the water stage has a contract"
    );
}

// --- through the session --------------------------------------------------------------------

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const POOLS: [[f64; 2]; 3] = [[0.0, 12.0], [1000.0, 10.0], [2000.0, 8.0]];

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![7.5; xs.len()]
}

/// A straight river 3 km long and 600 m wide, sized and aligned on its centreline, with a stand-in
/// DEM reference and the pool table as its water.
fn design() -> Session {
    let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let mut s = Session::new("Water", "2026-11-20");
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
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
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
    s.set_water_pools(&POOLS).unwrap();
    s
}

fn design_line(s: &Session) -> Vec<[f64; 2]> {
    s.final_line().unwrap().line.0
}

/// The stored answer's levels are the levels the volumes walk reads at the same chainages.
fn levels_agree_with_the_walk(s: &Session, w: &DesignWater) {
    let line = design_line(s);
    let at: Vec<f64> = w.points.iter().map(|p| p.chainage.0).collect();
    let walk = s.water_levels_along(&line, &at).unwrap();
    for (p, l) in w.points.iter().zip(walk) {
        // At a step the walk reads the upstream level, as the answer's second point does.
        let upstream = w
            .points
            .iter()
            .rev()
            .find(|q| q.chainage == p.chainage)
            .unwrap();
        assert_eq!(upstream.level_m, l, "at {} m", p.chainage.0);
    }
}

#[test]
fn the_volumes_stage_stores_the_water_and_it_is_the_water_it_reads() {
    let mut s = design();
    assert_eq!(s.water_status(), StageStatus::NotRun);
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    assert_eq!(s.water_status(), StageStatus::Fresh);
    let w = s.design_water().expect("the stored water");
    assert!(w.points.iter().all(|p| p.source == WaterSource::Given));
    // The pool table's steps at 1000 m and 2000 m are in the answer (the line runs on the
    // centreline, so its chainage is the river's).
    let steps: Vec<f64> = w
        .points
        .windows(2)
        .filter(|p| p[0].chainage == p[1].chainage)
        .map(|p| p[0].chainage.0)
        .collect();
    assert_eq!(steps.len(), 2, "{steps:?}");
    levels_agree_with_the_walk(&s, &w);
    // The record: the step as its settings, the hash of its input.
    let rec = &s.document()["stages"]["water"];
    assert_eq!(rec["settings"]["step_m"], json!(DEFAULT_STEP_M));
    assert!(rec["depends_on"].is_string());
    // It reopens fresh, and the document is valid.
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.water_status(), StageStatus::Fresh);
    assert!(back.validate().is_empty(), "{:?}", back.validate());
}

#[test]
fn a_new_water_level_makes_the_stored_water_stale() {
    let mut s = design();
    s.run_water().unwrap();
    assert_eq!(s.water_status(), StageStatus::Fresh);
    s.set_water_profile(&[[0.0, 9.0]]).unwrap();
    assert_eq!(s.water_status(), StageStatus::Stale);
    s.run_water().unwrap();
    assert_eq!(s.water_status(), StageStatus::Fresh);
    assert!(s.design_water().unwrap().points.iter().all(|p| p.level_m == 9.0));
}

#[test]
fn the_optimise_stage_s_pools_are_read_as_pools() {
    let mut s = design();
    // The optimise stage's pool levels as it stores them: each pool's two ends on river chainage.
    let mut doc = s.document().clone();
    doc["stages"]["optimise"]["settings"] = json!({});
    doc["stages"]["optimise"]["results"] = json!({
        "profile": [[0.0, 9.0], [1500.0, 9.0], [1500.0, 11.0], [3000.0, 11.0]],
    });
    doc["stages"]["optimise"]["engine_version"] = doc["engine_version"].clone();
    s = Session::open(&doc.to_string()).unwrap();
    let input = s.water_input().expect("the water input");
    assert_eq!(input.pools.len(), 2);
    assert_eq!(input.pools[1].from.0, 1500.0);
    s.run_water().unwrap();
    let w = s.design_water().unwrap();
    assert!(w.points.iter().all(|p| p.source == WaterSource::Pool));
    assert_eq!(w.level_at(Chainage(1499.0)), 9.0);
    assert_eq!(w.level_at(Chainage(1501.0)), 11.0);
    levels_agree_with_the_walk(&s, &w);
}

#[test]
fn without_a_design_line_the_water_stage_says_so() {
    let mut s = Session::new("Bare", "2026-11-20");
    let e = s.run_water().unwrap_err();
    assert!(e.to_string().contains("design line"), "{e}");
}
