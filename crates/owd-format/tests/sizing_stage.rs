//! The sizing stage (spec 0017, ticket 5), tested alone from the fixtures in
//! `tests/fixtures/sizing/`: a `SizingIn`, what the stage must return (or the code it must refuse
//! with), a label and why. No session is built for them. The sizing tests that used to be written
//! against the session with the answer worked out by the old path are these fixtures.
//!
//! The rest of the file puts the same stage through the session: the stored result is the typed
//! `ChannelSize` key for key, a stage reads it and not a rebuild, and a stale sizing is refused.
//!
//! Label: property

use owd_core::rules::engine;
use owd_format::design::sizing::{ChannelSize, SizingIn, size};
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sizing");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (
                name,
                serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The fixture's input, with the content hash of each selected set filled in from the built-in
/// library where the fixture gives none: a design always records one, and `size` refuses an entry
/// without. (The fixtures stay free of hashes, which change when the built-in Rules change.)
fn input_of(f: &Value) -> SizingIn {
    let mut input: SizingIn = serde_json::from_value(f["input"].clone()).expect("the input reads");
    for set in &mut input.rule_sets {
        if set.hash.is_none() {
            set.hash = engine::built_in()
                .find(&set.id, &set.version)
                .map(|s| s.hash.clone());
        }
    }
    input
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
        let input = input_of(&f);
        let got = size(&input, engine::built_in());
        if let Some(code) = f["expect"]["error"].as_str() {
            let e = got.expect_err(&format!("{name}: refused"));
            assert_eq!(
                (e.code.as_str(), e.stage.as_str()),
                (code, "sizing"),
                "{name}: {e}"
            );
            continue;
        }
        let out = got.unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = serde_json::to_value(&out).unwrap();
        contained(&json, &f["expect"]["output"], "output")
            .unwrap_or_else(|m| panic!("{name}: {m}"));
        // Whatever the vessel, the bottom width at R_min is the straight width and the widening there,
        // and the shape the later stages read gives the same widening at R_min.
        assert!(
            (out.bottom_width_at_rmin_m
                - (out.bottom_width_straight_m + out.bend_widening_at_rmin_m))
                .abs()
                < 1e-9,
            "{name}"
        );
        assert!(
            (out.bend_widening_m(out.min_bend_radius_m).unwrap() - out.bend_widening_at_rmin_m)
                .abs()
                < 1e-9,
            "{name}"
        );
        assert!(
            !out.design_basis.is_empty() && out.rules_notice.contains("unverified"),
            "{name}"
        );
    }
}

#[test]
fn a_custom_vessel_has_a_label_and_no_class_id() {
    let all = fixtures();
    let f = &all
        .iter()
        .find(|(n, _)| n == "custom-motor-vessel")
        .unwrap()
        .1;
    let input = input_of(f);
    let out = size(&input, engine::built_in()).unwrap();
    assert_eq!(out.vessel_standard_id, None);
    assert!(
        out.vessel_label
            .as_deref()
            .is_some_and(|l| l.starts_with("Custom: Test barge"))
    );
    // A class has no label.
    let f = &all.iter().find(|(n, _)| n == "cemt-iv-pianc-c").unwrap().1;
    let input = input_of(f);
    assert_eq!(size(&input, engine::built_in()).unwrap().vessel_label, None);
}

#[test]
fn the_same_input_gives_the_same_size() {
    let all = fixtures();
    let f = &all
        .iter()
        .find(|(n, _)| n == "gb50139-typed-class")
        .unwrap()
        .1;
    let input = input_of(f);
    assert_eq!(
        size(&input, engine::built_in()).unwrap(),
        size(&input, engine::built_in()).unwrap()
    );
}

// --- through the session ---------------------------------------------------------------------------

fn sized_session() -> Session {
    let centre: Vec<[f64; 2]> = (0..=10)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Sizing", "2026-11-19");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":{}}}"#,
            json!(centre)
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

#[test]
fn the_stored_result_is_the_typed_channel_size() {
    let s = sized_session();
    let stored = &s.document()["stages"]["sizing"]["results"];
    let typed: ChannelSize =
        serde_json::from_value(stored.clone()).expect("the stored result reads");
    // Written back, it is the stored JSON key for key (and in the same order).
    assert_eq!(
        serde_json::to_string(&typed).unwrap(),
        serde_json::to_string(stored).unwrap()
    );
    // And it is what the stage makes from the document's settings.
    let made = size(&s.sizing_in(), engine::built_in()).unwrap();
    assert_eq!(made, typed);
}

#[test]
fn a_stage_that_reads_the_size_refuses_a_stale_sizing_and_says_to_run_it() {
    let mut s = sized_session();
    let ring: Vec<[f64; 2]> = vec![
        [499_900.0, 599_900.0],
        [501_100.0, 599_900.0],
        [501_100.0, 600_100.0],
        [499_900.0, 600_100.0],
        [499_900.0, 599_900.0],
    ];
    s.import(&[(
        Role::RiverPolygon,
        &format!(
            r#"{{"type":"Polygon","crs":{{"type":"name","properties":{{"name":"urn:ogc:def:crs:EPSG::32630"}}}},"coordinates":[{}]}}"#,
            json!(ring)
        ),
    )])
    .unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    // The plan cut reads the stored size. The vessel changes and sizing is not rerun: the size it
    // would read is the old vessel's, so it refuses and names the stage. (`channel_size` refuses
    // too, as a backstop for a reader that has no guard of its own.)
    s.set_vessel("CEMT_Va", None).unwrap();
    let e = s.run_plan_cut(&json!({})).unwrap_err();
    assert!(
        e.to_string().contains("sizing") && e.to_string().contains("stale"),
        "{e}"
    );
    // Run again, and it reads the new size.
    s.run_stage(Stage::Sizing).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    s.run_plan_cut(&json!({})).unwrap();
}

#[test]
fn a_size_read_from_json_gives_a_typed_error_where_it_has_no_shape_not_a_panic() {
    // The type is public and reads from JSON, with no engine shape beside the data. Asking it for the
    // shape or for the widening at a radius must not panic on any public path.
    let s = sized_session();
    let typed: ChannelSize =
        serde_json::from_value(s.document()["stages"]["sizing"]["results"].clone()).unwrap();
    for e in [
        typed.shape().map(|_| ()).unwrap_err(),
        typed.bend_widening_m(300.0).unwrap_err(),
        typed.bottom_width_at_m(300.0).unwrap_err(),
    ] {
        assert_eq!(
            (e.code.as_str(), e.stage.as_str()),
            ("SIZING_NO_SHAPE", "sizing"),
            "{e}"
        );
    }
    // A size made by the stage, and one read through the session, have it.
    let made = size(&s.sizing_in(), engine::built_in()).unwrap();
    assert!(made.shape().is_ok() && made.bend_widening_m(300.0).unwrap() >= 0.0);
    assert!(s.channel_size().unwrap().shape().is_ok());
}

#[test]
fn channel_size_refuses_a_sizing_that_is_not_run_or_is_stale_and_says_to_run_it() {
    // The reader every later stage uses, tested on its own: not through a stage with a guard of its
    // own in front of it.
    let empty = Session::new("x", "2026-11-20");
    let e = empty.channel_size().unwrap_err().to_string();
    assert!(e.contains("sizing") && e.contains("not been run"), "{e}");

    let mut s = sized_session();
    assert!(s.channel_size().is_ok());
    s.set_vessel("CEMT_Va", None).unwrap();
    let e = s.channel_size().unwrap_err().to_string();
    assert!(
        e.contains("sizing") && e.contains("stale") && e.contains("run it"),
        "{e}"
    );
    s.run_stage(Stage::Sizing).unwrap();
    assert!(s.channel_size().is_ok());
}

#[test]
fn a_rule_set_entry_with_no_hash_is_refused() {
    // The old path refused a selected set that records no content hash: the design cannot say which
    // rules it was sized with. The pure function keeps that refusal.
    let all = fixtures();
    let f = &all.iter().find(|(n, _)| n == "cemt-iv-pianc-c").unwrap().1;
    let mut input: SizingIn = serde_json::from_value(f["input"].clone()).unwrap();
    for set in &mut input.rule_sets {
        set.hash = None;
    }
    let e = size(&input, engine::built_in()).expect_err("a set with no hash is refused");
    assert_eq!(e.code, "SIZING_RULE_SET_NO_HASH", "{e}");
}
