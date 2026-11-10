//! Ticket C5: the longest vessel for each beam gives the same answer from the app's shell and
//! from `owd envelope --beams … --length-search …`, and the document holds it.
//!
//! Label: property
//!
//! Closed form: a straight river 20 m wide. Under the default rule set (PIANC WG 141 quality C)
//! the straight fairway is three beams wide (`owd_core`'s test `envelope_longest_for_beam` reads
//! that from the rule set). A 4 m beam needs 12 m: every length fits. A 10 m beam needs 30 m: no
//! length fits.

use owd_core::envelope::{LengthSearch, Settings, Size};
use owd_format::design::Role;
use owd_shell::state::AppState;
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A straight river 3.2 km long and 20 m wide, in UTM 30N, with both banks and a centreline.
fn study() -> AppState {
    let at = |x: f64, y: f64| [x + 500_000.0, y + 600_000.0];
    let mut s = AppState::new_design("t", "2026-11-10");
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!([
            at(0.0, -10.0),
            at(3200.0, -10.0),
            at(3200.0, 10.0),
            at(0.0, 10.0),
            at(0.0, -10.0)
        ])
    );
    let line = |y: f64| {
        let pts: Vec<[f64; 2]> = (0..=32).map(|i| at(100.0 * f64::from(i), y)).collect();
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(pts)
        )
    };
    s.load_inputs(
        &[
            (Role::RiverPolygon, polygon),
            (Role::LeftBank, line(10.0)),
            (Role::RightBank, line(-10.0)),
            (Role::Centreline, line(0.0)),
        ],
        "2026-11-10",
    )
    .unwrap();
    s
}

fn settings() -> Settings {
    Settings {
        sizes: vec![Size {
            loa_m: 10.0,
            beam_m: 2.0,
        }],
        beams_m: vec![4.0, 10.0],
        length_search: LengthSearch {
            min_m: 20.0,
            max_m: 200.0,
            step_m: 1.0,
        },
        ..Settings::default()
    }
}

#[test]
fn the_shell_and_the_cli_give_the_same_answer_for_each_beam() {
    let mut s = study();
    let dir = std::env::temp_dir().join(format!("owd-c5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (doc, out) = (dir.join("river.owd"), dir.join("out.owd"));
    std::fs::write(&doc, s.save_text_for(&doc)).unwrap();

    // The CLI, on the saved file.
    owd_cli::envelope::envelope(
        &doc.to_string_lossy(),
        Some("10x2"),
        None,
        None,
        Some("4,10"),
        Some("20:200:1"),
        false,
        false,
        None,
        &Some(out.to_string_lossy().into_owned()),
        true,
    )
    .unwrap_or_else(|f| panic!("the CLI: {}", f.message));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let from_cli = &written["stages"]["envelope"];

    // The shell, with the same settings.
    s.run_envelope_with(settings()).unwrap();
    let snap = s.snapshot();
    let from_shell = &snap["document"]["stages"]["envelope"];
    assert_eq!(from_shell, from_cli);

    assert_eq!(from_shell["settings"]["beams_m"], json!([4.0, 10.0]));
    assert_eq!(
        from_shell["settings"]["length_search"],
        json!({"min_m": 20.0, "max_m": 200.0, "step_m": 1.0})
    );
    let by_beam = from_shell["results"]["by_beam"].as_array().unwrap();
    assert_eq!(by_beam.len(), 2);
    // 4 m: every length fits, so the upper bound of the search stops it.
    assert_eq!(by_beam[0]["beam_m"], json!(4.0));
    assert_eq!(by_beam[0]["longest_m"], json!(200.0));
    assert_eq!(by_beam[0]["at_max"], true);
    assert_eq!(by_beam[0]["binding"], json!([]));
    // 10 m: no length fits, and the width is what binds.
    assert_eq!(by_beam[1]["beam_m"], json!(10.0));
    assert_eq!(by_beam[1]["longest_m"], Value::Null);
    assert_eq!(by_beam[1]["at_max"], false);
    let binding = by_beam[1]["binding"].as_array().unwrap();
    assert!(!binding.is_empty());
    for b in binding {
        assert_eq!(b["kind"], "width", "{b}");
        assert!(b["start_chainage_m"].is_number() && b["end_chainage_m"].is_number());
        assert!(b["required_m"].as_f64().unwrap() > b["found_m"].as_f64().unwrap());
    }
}

#[test]
fn with_no_beam_the_document_holds_the_series_alone() {
    let mut s = study();
    s.run_envelope().unwrap();
    let snap = s.snapshot();
    let e = &snap["document"]["stages"]["envelope"];
    assert!(e["results"]["sizes"].is_array());
    assert!(e["results"].get("by_beam").is_none(), "{}", e["results"]);
    assert!(e["settings"].get("beams_m").is_none(), "{}", e["settings"]);
    assert!(e["settings"].get("length_search").is_none());
    assert!(Settings::default().beams_m.is_empty());
}
