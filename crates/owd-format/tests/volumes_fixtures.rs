//! The volumes stage (spec 0017, ticket 9), tested alone from the fixtures in
//! `tests/fixtures/volumes/`: the sizing input, the rest of the stage's input, the terrain, what the
//! stage must return (or the code it must refuse with), a label and why. No session is built.
//!
//! A `ChannelSize` read from JSON has no Rule engine shape, which the widened half-width reads, so a
//! fixture holds the sizing input and the test makes the size with the sizing stage (as the alignment
//! fixtures do). A fixture with `size_from_json` reads the size back from JSON first, to show a fixed
//! half-width does not need the shape.
//!
//! The terrain is flat (`flat_m`), ends at an x (`ends_at_x`), or is nowhere (`none`).
//!
//! Label: golden (the prism), property (the rest)

use owd_core::rules::engine;
use owd_format::design::sizing::{ChannelSize, SizingIn, size};
use owd_format::design::volumes_stage::{VolumesIn, volumes_stage};
use owd_format::types::Chainage;
use serde_json::Value;
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/volumes");
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

/// `want` is contained in `got`: every key of an object, every number to 1e-9.
fn contained(got: &Value, want: &Value, at: &str) -> Result<(), String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => w.iter().try_for_each(|(k, wv)| {
            let gv = g.get(k).ok_or_else(|| format!("{at}/{k}: missing"))?;
            contained(gv, wv, &format!("{at}/{k}"))
        }),
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

fn sized(f: &Value) -> ChannelSize {
    let mut sizing_in: SizingIn = serde_json::from_value(f["input"]["sizing"].clone()).unwrap();
    // A design records each set's content hash; the fixtures stay free of hashes, which change
    // when the built-in Rules change (as the sizing fixtures do).
    for set in &mut sizing_in.rule_sets {
        if set.hash.is_none() {
            set.hash = engine::built_in()
                .find(&set.id, &set.version)
                .map(|s| s.hash.clone());
        }
    }
    let made = size(&sizing_in, engine::built_in()).unwrap();
    if f["size_from_json"] == true {
        serde_json::from_value(serde_json::to_value(made).unwrap()).unwrap()
    } else {
        made
    }
}

/// The cut of one section whose design invert is `c` metres under a flat terrain, at the fixture
/// settings (samples 1 m apart over 200 m each side, a wet slope of 3): `c` on the bottom and
/// `c - r / 3` at `r` metres up a slope.
fn section_m2(c: f64, hb: f64) -> f64 {
    (-200..=200)
        .map(|o| {
            let r = (f64::from(o).abs() - hb).max(0.0);
            (c - r / 3.0).max(0.0)
        })
        .sum()
}

#[test]
fn every_fixture_gives_what_it_says() {
    let all = fixtures();
    assert!(all.len() >= 6, "the fixtures are there: {}", all.len());
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
        let size = sized(&f);
        let mut rest = f["input"].clone();
        rest.as_object_mut().unwrap().remove("sizing");
        rest["size"] = serde_json::to_value(&size).unwrap();
        let mut input: VolumesIn =
            serde_json::from_value(rest).unwrap_or_else(|e| panic!("{name}: the input reads: {e}"));
        input.size = size.clone();

        let t = &f["terrain"];
        let flat = t["flat_m"].as_f64().unwrap_or(f64::NAN);
        let ends = t["ends_at_x"].as_f64().unwrap_or(f64::INFINITY);
        let none = t["none"] == true;
        let terrain = move |xs: &[f64], _: &[f64]| -> Vec<f64> {
            xs.iter()
                .map(|&x| if none || x > ends { f64::NAN } else { flat })
                .collect()
        };
        let got = volumes_stage(&input, &terrain);

        if let Some(code) = f["expect"]["error"].as_str() {
            let e = got.expect_err(&format!("{name}: refused"));
            assert_eq!(
                (e.code.as_str(), e.stage.as_str()),
                (code, "volumes"),
                "{name}: {e}"
            );
            if let Some(at) = f["expect"]["at"].as_f64() {
                assert_eq!(e.at, Some(Chainage(at)), "{name}: {e}");
            }
            continue;
        }
        let out = got.unwrap_or_else(|e| panic!("{name}: {e}"));
        contained(
            &serde_json::to_value(&out).unwrap(),
            &f["expect"]["output"],
            "output",
        )
        .unwrap_or_else(|m| panic!("{name}: {m}"));
        if f["expect"]["prism_by_hand"] == true {
            let water = f["input"]["water"]["given"][0][1].as_f64().unwrap();
            let c = flat - (water - size.design_depth_m);
            let hb = input
                .settings
                .half_width_m
                .unwrap_or(size.bottom_width_straight_m / 2.0);
            let area = section_m2(c, hb);
            assert!(c > 0.5 && area > 10.0, "{name}: a real cut");
            for st in &out.stations {
                assert!(
                    (st.dredge_area_m2 - area).abs() <= 1e-9 * area,
                    "{name}: {} m: {} against {area}",
                    st.chainage_m,
                    st.dredge_area_m2
                );
            }
        }
    }
}
