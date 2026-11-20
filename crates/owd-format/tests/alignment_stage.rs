//! The alignment stage (spec 0017, ticket 6), tested alone from the fixtures in
//! `tests/fixtures/alignment/`: the river inputs, the sizing inputs and the alignment settings, what
//! the stage must return (or the code it must refuse with), a label and why. No session is built.
//!
//! The stages before it are pure too: the test makes the river geometry with the river stage and the
//! channel size with the sizing stage, from the fixture's own inputs, then calls the alignment stage.
//! (A `ChannelSize` read from JSON has no Rule engine shape, which the rolling circle reads: it
//! refuses with `SIZING_NO_SHAPE`. So the fixture holds the sizing input, not the size.)
//!
//! Label: property

use owd_core::rules::engine;
use owd_format::design::design_line::{AlignmentIn, DesignLine, alignment};
use owd_format::design::river::{RiverIn, river};
use owd_format::design::sizing::{SizingIn, size};
use owd_format::design::{AlignMethod, AlignmentSettings};
use owd_format::types::{StageError, TerrainSampler};
use serde_json::Value;
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/alignment");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).unwrap();
            (name, serde_json::from_str(&text).unwrap())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `want` is contained in `got`: every key of an object, every item of a list (same length), every
/// number to 1e-9.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => w.iter().try_for_each(|(k, wv)| {
            let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
            contained(gv, wv, &format!("{at}/{k}"))
        }),
        (Value::Array(g), Value::Array(w)) => {
            if g.len() != w.len() {
                return Err(format!("{at}: {} items, want {}", g.len(), w.len()));
            }
            g.iter()
                .zip(w)
                .enumerate()
                .try_for_each(|(i, (gv, wv))| contained(gv, wv, &format!("{at}/{i}")))
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

/// Run the river, sizing and alignment stages on a fixture's input.
fn run(f: &Value) -> Result<DesignLine, StageError> {
    let river_in: RiverIn = serde_json::from_value(f["input"]["river"].clone()).unwrap();
    let mut sizing_in: SizingIn = serde_json::from_value(f["input"]["sizing"].clone()).unwrap();
    // A design always records each set's content hash, and `size` refuses an entry without one. The
    // fixtures stay free of hashes, which change when the built-in Rules change (as the sizing
    // fixtures do).
    for set in &mut sizing_in.rule_sets {
        if set.hash.is_none() {
            set.hash = engine::built_in()
                .find(&set.id, &set.version)
                .map(|s| s.hash.clone());
        }
    }
    let settings: AlignmentSettings =
        serde_json::from_value(f["input"]["settings"].clone()).unwrap();
    let input = AlignmentIn {
        river: river(&river_in)?,
        size: size(&sizing_in, engine::built_in())?,
        settings,
        seed: None,
        constraints: None,
    };
    let flat = f["terrain"]["flat_m"].as_f64();
    let terrain = move |xs: &[f64], _: &[f64]| -> Vec<f64> {
        xs.iter().map(|_| flat.unwrap_or(f64::NAN)).collect()
    };
    let t: Option<TerrainSampler> = flat.map(|_| &terrain as TerrainSampler);
    alignment(&input, t)
}

#[test]
fn every_fixture_gives_what_it_says() {
    let all = fixtures();
    assert!(all.len() >= 10, "the fixtures are there: {}", all.len());
    for (name, f) in all {
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
        let got = run(&f);
        if let Some(code) = f["expect"]["error"].as_str() {
            let e = got.expect_err(&format!("{name}: refused"));
            assert_eq!(
                (e.code.as_str(), e.stage.as_str()),
                (code, "alignment"),
                "{name}: {e}"
            );
            continue;
        }
        let out = got.unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = serde_json::to_value(&out).unwrap();
        contained(&json, &f["expect"]["output"], "output")
            .unwrap_or_else(|m| panic!("{name}: {m}"));
        // The stored form reads back as the same line.
        let back: DesignLine = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, out, "{name}: round trip");
        if f["every_bend_passes"] == true {
            assert!(out.bends.iter().all(|b| b.pass), "{name}: {:?}", out.bends);
        }
        if let Some(cuts) = f["cut_offs_by_hand"].as_array() {
            let got = out.cut_offs.as_ref().expect("the water is known");
            assert_eq!(got.len(), cuts.len(), "{name}: {got:?}");
            for (g, w) in got.iter().zip(cuts) {
                let tol = w["tolerance_m"].as_f64().unwrap();
                for (v, k) in [(g.river_from.0, "river_from"), (g.river_to.0, "river_to")] {
                    let want = w[k].as_f64().unwrap();
                    assert!((v - want).abs() <= tol, "{name}: {k} {v} not {want}");
                }
                assert!(g.from.0 < g.to.0, "{name}: {g:?}");
            }
        }
    }
}

#[test]
fn every_method_has_a_fixture() {
    let all = fixtures();
    for m in [
        AlignMethod::RollingCircle,
        AlignMethod::Reroute,
        AlignMethod::Centreline,
        AlignMethod::Given,
    ] {
        assert!(
            all.iter()
                .any(|(_, f)| f["input"]["settings"]["method"] == m.name()
                    && f["expect"]["output"].is_object()),
            "no fixture runs the method `{}`",
            m.name()
        );
    }
}

/// A bend's chainage is along the design line from its first point: the same river given the other
/// way round with the mouth stated gives the same line and the same bends (spec 0017).
#[test]
fn a_reversed_river_with_its_mouth_gives_the_same_design_line() {
    let all = fixtures();
    let f = &all
        .iter()
        .find(|(n, _)| n == "centreline-tight-bend")
        .unwrap()
        .1;
    let mut forward = f.clone();
    let line = forward["input"]["river"]["centreline"].clone();
    let first = line[0].clone();
    forward["input"]["river"]["mouth"] = serde_json::json!({ "point": first });
    let mut backward = forward.clone();
    let mut pts = line.as_array().unwrap().clone();
    pts.reverse();
    backward["input"]["river"]["centreline"] = Value::Array(pts);
    let (a, b) = (run(&forward).unwrap(), run(&backward).unwrap());
    assert_eq!(a, b);
    assert_eq!(
        a.line.0[0],
        [first[0].as_f64().unwrap(), first[1].as_f64().unwrap()]
    );
    assert!(!a.bends.is_empty());
}
