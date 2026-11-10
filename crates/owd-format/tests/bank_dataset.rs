//! The bank dataset: one typed view of the banks of a design, read from the document. It holds
//! the two bank lines, the river polygon with its holes, the source, the accuracy, the
//! low-confidence stretches and a hash of all of it.
//!
//! Label: property

use owd_format::design::banks::{BankDataset, BankSource, BankSourceKind, LowConfidence};
use owd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};
use std::path::Path;

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn points(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn at(x: f64, y: f64) -> [f64; 2] {
    [x + 500_000.0, y + 600_000.0]
}

/// A straight river 3.2 km long and 400 m wide, in UTM 30N: both banks, the centreline and the
/// polygon. The channel is sized and the rolling-circle alignment has run.
fn design() -> Session {
    let line = |y: f64| {
        let pts: Vec<[f64; 2]> = (0..=32).map(|i| at(100.0 * f64::from(i), y)).collect();
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(pts)
        )
    };
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!([
            at(0.0, -200.0),
            at(3200.0, -200.0),
            at(3200.0, 200.0),
            at(0.0, 200.0),
            at(0.0, -200.0)
        ])
    );
    let mut s = Session::new("Banks", "2026-11-10");
    s.import(&[
        (Role::LeftBank, &line(200.0)),
        (Role::RightBank, &line(-200.0)),
        (Role::Centreline, &line(0.0)),
        (Role::RiverPolygon, &polygon),
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
    s
}

fn statuses(s: &Session) -> [StageStatus; 2] {
    [
        s.stage_status(Stage::Sizing),
        s.stage_status(Stage::Alignment),
    ]
}

#[test]
fn an_old_document_gives_a_dataset_equal_to_its_bank_members() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd"),
    )
    .unwrap();
    let s = Session::open(&text).unwrap();
    let inputs = &s.document()["inputs"];
    // Reading the dataset adds nothing to the document.
    assert!(inputs.get("banks").is_none());

    let d: BankDataset = s.bank_dataset().expect("the document has both banks");
    assert_eq!(d.left, points(&inputs["left_bank"]["coordinates"]));
    assert_eq!(d.right, points(&inputs["right_bank"]["coordinates"]));
    let river = d.river.as_ref().expect("the document has a river polygon");
    assert_eq!(
        river.exterior,
        points(&inputs["river_polygon"]["coordinates"][0])
    );
    assert!(river.holes.is_empty());
    assert_eq!(d.source, None);
    assert_eq!(d.accuracy_m, None);
    assert!(d.low_confidence.is_empty());
    // The hash is of the content: the same document gives the same hash.
    assert_eq!(d.hash.len(), 64);
    assert_eq!(
        d.hash,
        Session::open(&text).unwrap().bank_dataset().unwrap().hash
    );
    assert_eq!(
        Session::new("t", "2026-11-10").bank_dataset(),
        None,
        "no banks"
    );
}

#[test]
fn the_dataset_reads_the_source_the_accuracy_the_stretches_and_the_holes() {
    let mut doc: Value = serde_json::from_str(&design().to_text()).unwrap();
    let island = json!([
        at(1000.0, -20.0),
        at(1200.0, -20.0),
        at(1200.0, 20.0),
        at(1000.0, 20.0),
        at(1000.0, -20.0)
    ]);
    doc["inputs"]["river_polygon"]["coordinates"]
        .as_array_mut()
        .unwrap()
        .push(island.clone());
    let plain = Session::open(&doc.to_string())
        .unwrap()
        .bank_dataset()
        .unwrap();
    assert_eq!(plain.river.as_ref().unwrap().holes, vec![points(&island)]);

    doc["inputs"]["banks"] = json!({
        "source": {"kind": "bank-source", "name": "a bank source", "date": "2026-11-10",
                   "licence": "CC BY 4.0"},
        "accuracy_m": 2.5,
        "low_confidence": [{"from_m": 400.0, "to_m": 650.0, "reason": "trees over the bank"}]
    });
    let d = Session::open(&doc.to_string())
        .unwrap()
        .bank_dataset()
        .unwrap();
    assert_eq!(
        d.source,
        Some(BankSource {
            kind: BankSourceKind::BankSource,
            name: "a bank source".into(),
            date: "2026-11-10".into(),
            licence: Some("CC BY 4.0".into()),
        })
    );
    assert_eq!(d.accuracy_m, Some(2.5));
    assert_eq!(
        d.low_confidence,
        vec![LowConfidence {
            from_m: 400.0,
            to_m: 650.0,
            reason: "trees over the bank".into(),
        }]
    );
    assert_eq!((&d.left, &d.right), (&plain.left, &plain.right));
    // The hash covers every part: the accuracy, and the lines.
    assert_ne!(d.hash, plain.hash);
    doc["inputs"]["banks"]["accuracy_m"] = json!(1.0);
    let other = Session::open(&doc.to_string())
        .unwrap()
        .bank_dataset()
        .unwrap();
    assert_ne!(other.hash, d.hash);
    doc["inputs"]["banks"]["accuracy_m"] = json!(2.5);
    doc["inputs"]["left_bank"]["coordinates"][3][1] = json!(600_210.0);
    let moved = Session::open(&doc.to_string())
        .unwrap()
        .bank_dataset()
        .unwrap();
    assert_ne!(moved.hash, d.hash);
}

#[test]
fn a_new_accuracy_makes_the_later_stages_stale_and_the_same_accuracy_does_not() {
    use StageStatus::{Fresh, Stale};
    let mut s = design();
    assert_eq!(statuses(&s), [Fresh, Fresh]);

    // No accuracy is recorded: saying so again changes nothing, and adds no member.
    let before = s.to_text();
    s.set_bank_accuracy(None).unwrap();
    assert_eq!(s.to_text(), before);

    // A value that is no accuracy is refused, and changes nothing.
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let e = s.set_bank_accuracy(Some(bad)).unwrap_err();
        assert!(matches!(e, DesignError::Input(_)), "{e:?}");
        assert_eq!(s.to_text(), before);
    }

    // A new accuracy: the alignment read the banks, so it is stale. The sizing reads no input.
    s.set_bank_accuracy(Some(2.5)).unwrap();
    assert_eq!(s.document()["inputs"]["banks"]["accuracy_m"], json!(2.5));
    assert_eq!(s.bank_dataset().unwrap().accuracy_m, Some(2.5));
    assert_eq!(statuses(&s), [Fresh, Stale]);
    assert!(s.validate().is_empty(), "{:?}", s.validate());

    // Run the alignment again: fresh. The same accuracy again leaves it fresh.
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    assert_eq!(statuses(&s), [Fresh, Fresh]);
    let with_accuracy = s.to_text();
    s.set_bank_accuracy(Some(2.5)).unwrap();
    assert_eq!(s.to_text(), with_accuracy);
    assert_eq!(statuses(&s), [Fresh, Fresh]);

    // Another value, or none, is a change.
    s.set_bank_accuracy(Some(1.0)).unwrap();
    assert_eq!(statuses(&s), [Fresh, Stale]);
    s.set_bank_accuracy(None).unwrap();
    assert_eq!(s.bank_dataset().unwrap().accuracy_m, None);
}
