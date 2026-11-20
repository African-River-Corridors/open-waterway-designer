//! Run inputs (spec 0017, ticket 11): the run's objective, its limits and its target output level,
//! as one typed value in `inputs.run`.
//!
//! The fixtures in `tests/fixtures/run/` hold a run-inputs block, why it is there, and what reading
//! it must give: `ok` with the target level it reads as, or the code it must be refused with. No
//! session is built for them. The tests below the fixtures read the text forms the command line and
//! the app give, and the block in a document.
//!
//! Label: property

use owd_format::design::run_inputs::{
    Bound, DesignMeasure, INPUT, Limit, Objective, OutputLevel, RunInputs, RunInputsEdit, Sense,
    parse_level, parse_limit, parse_objective, read_run_inputs,
};
use owd_format::design::{Role, Session, Stage};
use owd_format::stage::contracts;
use serde_json::{Value, json};
use std::path::Path;

fn fixtures() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run");
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

#[test]
fn each_fixture_reads_as_it_says() {
    let all = fixtures();
    assert!(all.len() >= 14, "the fixtures are missing");
    for (name, f) in all {
        let got = read_run_inputs(&f["input"]);
        let want = &f["expect"];
        match (got, want["code"].as_str()) {
            (Ok(run), None) => {
                assert_eq!(
                    json!(run.target()),
                    want["target"],
                    "{name}: the target level"
                );
                // What was read writes back as it was given: nothing is dropped or added.
                assert_eq!(serde_json::to_value(&run).unwrap(), f["input"], "{name}");
            }
            (Err(e), Some(code)) => {
                assert_eq!(e.code, code, "{name}: {}", e.message);
                assert_eq!(e.stage, INPUT, "{name}");
            }
            (Ok(_), Some(code)) => panic!("{name}: read, but should be refused with {code}"),
            (Err(e), None) => panic!("{name}: refused with {e}, but should read"),
        }
    }
}

#[test]
fn an_unknown_measure_is_named_in_the_refusal() {
    let e = read_run_inputs(&json!({"objective": {"measure": "beauty", "sense": "maximise"}}))
        .unwrap_err();
    assert_eq!(e.code, "RUN_MEASURE_UNKNOWN");
    assert!(e.message.contains("`beauty`"), "{}", e.message);
    // The message lists the measures OWD knows.
    assert!(e.message.contains("total_cut"), "{}", e.message);
}

#[test]
fn the_objective_reads_from_text_one_way() {
    let cost = Objective {
        measure: DesignMeasure::Cost,
        sense: Sense::Minimise,
    };
    assert_eq!(
        parse_objective("minimise:cost").unwrap(),
        Some(cost.clone())
    );
    assert_eq!(
        parse_objective(" minimise cost ").unwrap(),
        Some(cost.clone())
    );
    assert_eq!(parse_objective("minimize:cost").unwrap(), Some(cost));
    assert_eq!(
        parse_objective("maximise:capacity").unwrap(),
        Some(Objective {
            measure: DesignMeasure::Capacity,
            sense: Sense::Maximise
        })
    );
    assert_eq!(parse_objective("none").unwrap(), None);
    assert_eq!(parse_objective("").unwrap(), None);
    assert_eq!(
        parse_objective("minimise:beauty").unwrap_err().code,
        "RUN_MEASURE_UNKNOWN"
    );
    assert_eq!(
        parse_objective("shrink:cost").unwrap_err().code,
        "RUN_SENSE_UNKNOWN"
    );
    assert_eq!(
        parse_objective("cost").unwrap_err().code,
        "RUN_OBJECTIVE_SYNTAX"
    );
}

#[test]
fn a_limit_reads_from_text_one_way_and_writes_back_the_same() {
    let cut = parse_limit("total_cut<=500000").unwrap();
    assert_eq!(
        cut,
        Limit {
            measure: DesignMeasure::TotalCut,
            bound: Bound::AtMost,
            value: 500_000.0,
            currency: None
        }
    );
    let cost = parse_limit("cost <= 2000000 USD").unwrap();
    assert_eq!(cost.currency.as_deref(), Some("USD"));
    assert_eq!(cost.bound, Bound::AtMost);
    let len = parse_limit("line_length>=1000").unwrap();
    assert_eq!(len.bound, Bound::AtLeast);
    for l in [&cut, &cost, &len] {
        assert_eq!(&parse_limit(&l.to_text()).unwrap(), l, "{}", l.to_text());
    }
    assert_eq!(
        parse_limit("total_cut=5").unwrap_err().code,
        "RUN_LIMIT_SYNTAX"
    );
    assert_eq!(
        parse_limit("total_cut<=lots").unwrap_err().code,
        "RUN_LIMIT_SYNTAX"
    );
    assert_eq!(
        parse_limit("depth<=3").unwrap_err().code,
        "RUN_MEASURE_UNKNOWN"
    );
}

#[test]
fn the_target_level_reads_from_text_one_way_and_levels_are_ordered() {
    assert_eq!(parse_level("concept").unwrap(), Some(OutputLevel::Concept));
    assert_eq!(
        parse_level("pre-feasibility").unwrap(),
        Some(OutputLevel::PreFeasibility)
    );
    assert_eq!(
        parse_level("pre_feasibility").unwrap(),
        Some(OutputLevel::PreFeasibility)
    );
    assert_eq!(
        parse_level("Feasibility").unwrap(),
        Some(OutputLevel::Feasibility)
    );
    assert_eq!(parse_level("").unwrap(), None);
    assert_eq!(
        parse_level("detailed").unwrap_err().code,
        "RUN_LEVEL_UNKNOWN"
    );
    assert!(OutputLevel::Concept < OutputLevel::PreFeasibility);
    assert!(OutputLevel::PreFeasibility < OutputLevel::Feasibility);
    // With no target stated, a run aims at concept, the lowest level.
    assert_eq!(RunInputs::default().target(), OutputLevel::Concept);
}

#[test]
fn every_measure_has_one_name_and_one_unit() {
    let names: Vec<&str> = DesignMeasure::ALL.iter().map(|m| m.name()).collect();
    assert_eq!(
        names,
        [
            "cost",
            "total_cut",
            "dredge",
            "dry_cut",
            "land_take",
            "line_length",
            "structure_count",
            "capacity"
        ]
    );
    for m in DesignMeasure::ALL {
        assert_eq!(json!(m), json!(m.name()));
        assert!(!m.unit().is_empty());
    }
}

#[test]
fn the_run_inputs_have_a_checked_in_schema() {
    let c = contracts()
        .into_iter()
        .find(|c| c.stage == "inputs.run")
        .expect("the run inputs are a contract with a schema file");
    assert_eq!(c.file_name(), "inputs.run.1.schema.json");
    let s = c.schema();
    // The measure names and the levels are the schema's enums, so a reader can check them.
    let text = s.to_string();
    for word in ["total_cut", "pre_feasibility", "at_most", "maximise"] {
        assert!(text.contains(word), "{word} is not in the schema");
    }
}

// ---- The block in a document ----

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A sized synthetic design: a straight river 3 km long with a falling water level.
fn sized() -> Session {
    let line: Vec<[f64; 2]> = (0..=30)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0])
        .collect();
    let mut s = Session::new("Run inputs", "2026-11-20");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(line)
        ),
    )])
    .unwrap();
    s.set_water_profile(&[[0.0, 9.0], [3000.0, 12.0]]).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

fn limits(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|t| t.to_string()).collect()
}

#[test]
fn a_document_without_run_inputs_opens_and_saves_as_before() {
    let text = sized().to_text();
    assert!(!text.contains("\"run\""));
    let s = Session::open(&text).unwrap();
    assert_eq!(s.run_inputs().unwrap(), None);
    assert_eq!(s.to_text(), text, "opening and saving changed the file");
}

#[test]
fn run_inputs_set_once_read_back_typed_after_a_save() {
    let mut s = sized();
    let lims = limits(&["total_cut<=500000", "cost<=2000000 USD"]);
    let run = s
        .edit_run_inputs(&RunInputsEdit {
            objective: Some("minimise:cost"),
            limits: Some(&lims),
            target_level: Some("pre-feasibility"),
        })
        .unwrap();
    assert_eq!(run.target(), OutputLevel::PreFeasibility);
    assert_eq!(
        s.document()["inputs"]["run"],
        json!({
            "objective": {"measure": "cost", "sense": "minimise"},
            "limits": [
                {"measure": "total_cut", "bound": "at_most", "value": 500000.0},
                {"measure": "cost", "bound": "at_most", "value": 2000000.0, "currency": "USD"}
            ],
            "target_level": "pre_feasibility"
        })
    );
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.run_inputs().unwrap(), Some(run));
}

#[test]
fn setting_run_inputs_moves_no_stage() {
    let mut s = sized();
    // A stage past sizing hashes the inputs it reads: the run inputs must not be among them.
    s.run_stage(Stage::Alignment).unwrap();
    assert_eq!(
        s.stage_status(Stage::Alignment),
        owd_format::design::StageStatus::Fresh
    );
    let before: Vec<_> = Stage::ALL.iter().map(|st| s.stage_status(*st)).collect();
    let stages = s.document()["stages"].clone();
    s.edit_run_inputs(&RunInputsEdit {
        objective: Some("minimise:total_cut"),
        limits: None,
        target_level: Some("feasibility"),
    })
    .unwrap();
    let after: Vec<_> = Stage::ALL.iter().map(|st| s.stage_status(*st)).collect();
    assert_eq!(before, after);
    assert_eq!(s.document()["stages"], stages);
}

#[test]
fn a_part_not_named_is_kept_and_an_empty_text_clears_it() {
    let mut s = sized();
    let lims = limits(&["total_cut<=500000"]);
    s.edit_run_inputs(&RunInputsEdit {
        objective: Some("minimise:cost"),
        limits: Some(&lims),
        target_level: None,
    })
    .unwrap();
    // Only the level: the objective and the limits stay.
    let run = s
        .edit_run_inputs(&RunInputsEdit {
            objective: None,
            limits: None,
            target_level: Some("feasibility"),
        })
        .unwrap();
    assert_eq!(run.objective.unwrap().measure, DesignMeasure::Cost);
    assert_eq!(run.limits.len(), 1);
    // Clear each part: the block goes, and the file is as it was before any was set.
    let text_before = sized().to_text();
    let none: Vec<String> = Vec::new();
    s.edit_run_inputs(&RunInputsEdit {
        objective: Some(""),
        limits: Some(&none),
        target_level: Some(""),
    })
    .unwrap();
    assert_eq!(s.run_inputs().unwrap(), None);
    assert!(s.document()["inputs"].get("run").is_none());
    assert_eq!(s.to_text(), text_before);
}

#[test]
fn a_refused_edit_changes_nothing() {
    let mut s = sized();
    s.edit_run_inputs(&RunInputsEdit {
        objective: Some("minimise:cost"),
        limits: None,
        target_level: None,
    })
    .unwrap();
    let text = s.to_text();
    let lims = limits(&["total_cut<=500000", "total_cut>=600000"]);
    let e = s
        .edit_run_inputs(&RunInputsEdit {
            objective: Some("maximise:dredge"),
            limits: Some(&lims),
            target_level: None,
        })
        .unwrap_err();
    assert!(e.to_string().contains("RUN_LIMITS_CONTRADICT"), "{e}");
    let e = s
        .edit_run_inputs(&RunInputsEdit {
            objective: Some("minimise:beauty"),
            limits: None,
            target_level: None,
        })
        .unwrap_err();
    assert!(e.to_string().contains("RUN_MEASURE_UNKNOWN"), "{e}");
    assert_eq!(s.to_text(), text);
}

#[test]
fn a_document_whose_run_inputs_break_a_rule_does_not_open() {
    let mut doc: Value = serde_json::from_str(&sized().to_text()).unwrap();
    doc["inputs"]["run"] = json!({"limits": [
        {"measure": "total_cut", "bound": "at_most", "value": 1.0},
        {"measure": "total_cut", "bound": "at_most", "value": 2.0}
    ]});
    let e = Session::open(&doc.to_string()).err().unwrap();
    assert!(e.to_string().contains("RUN_LIMIT_DUPLICATE"), "{e}");
    // An unknown measure breaks the document schema itself.
    doc["inputs"]["run"] = json!({"objective": {"measure": "beauty", "sense": "minimise"}});
    let e = Session::open(&doc.to_string()).err().unwrap();
    assert!(e.to_string().contains("/inputs/run"), "{e}");
}

fn example(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_saved_examples_open_and_save_byte_for_byte() {
    // QA on PR 570, finding 2: files saved before run inputs existed.
    for name in [
        "synthetic-river.owd",
        "web/river-example.owd",
        "web/river-reroute.owd",
        "web/river-rolling-circle.owd",
    ] {
        let text = example(name);
        let s = Session::open(&text).unwrap();
        assert_eq!(s.run_inputs().unwrap(), None, "{name}");
        assert_eq!(s.to_text(), text, "{name}: opening and saving changed it");
    }
}

#[test]
fn every_stage_of_the_chain_stays_fresh_when_run_inputs_are_set() {
    // QA on PR 570, finding 1: sizing, alignment, optimise and volumes all run, on the web
    // example's DEM, then the run inputs are set.
    use owd_format::design::StageStatus;
    use owd_format::design::terrain::dem_sampler;
    use owd_format::types::TerrainSampler;
    let dem = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-dem.tif"),
    )
    .unwrap();
    let grid = owd_format::geotiff::read_bytes(&dem).unwrap();
    let t = dem_sampler(&grid, Some(32630)).unwrap();
    let mut s = Session::open(&example("web/river-example.owd")).unwrap();
    for st in Stage::ALL {
        s.run_stage_with(st, Some(&t as TerrainSampler)).unwrap();
    }
    let fresh = vec![StageStatus::Fresh; Stage::ALL.len()];
    let status =
        |s: &Session| -> Vec<_> { Stage::ALL.iter().map(|st| s.stage_status(*st)).collect() };
    assert_eq!(status(&s), fresh);
    let stages = s.document()["stages"].clone();
    let lims = limits(&["total_cut<=500000", "cost<=2000000 USD"]);
    s.edit_run_inputs(&RunInputsEdit {
        objective: Some("minimise:cost"),
        limits: Some(&lims),
        target_level: Some("feasibility"),
    })
    .unwrap();
    assert_eq!(status(&s), fresh);
    assert_eq!(s.document()["stages"], stages);
    // And after a save and an open.
    assert_eq!(status(&Session::open(&s.to_text()).unwrap()), fresh);
}
