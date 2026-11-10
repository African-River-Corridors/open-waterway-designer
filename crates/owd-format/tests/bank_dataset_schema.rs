//! The document may hold what a bank source says about its banks, at `inputs.banks`: the source,
//! the accuracy and the low-confidence stretches. The member is optional. The bank lines stay in
//! `inputs.left_bank` and `inputs.right_bank`.
//!
//! Label: property

use owd_format::design::{Role, Session};
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A straight river 2 km long and 200 m wide, in UTM 30N, as two banks.
fn document() -> Value {
    let line = |y: f64| {
        let pts: Vec<[f64; 2]> = (0..=20)
            .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0 + y])
            .collect();
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(pts)
        )
    };
    let mut s = Session::new("Banks", "2026-11-10");
    s.import(&[
        (Role::LeftBank, &line(100.0)),
        (Role::RightBank, &line(-100.0)),
    ])
    .unwrap();
    serde_json::from_str(&s.to_text()).unwrap()
}

fn banks() -> Value {
    json!({
        "source": {"kind": "file", "name": "survey.geojson", "date": "2026-11-10", "licence": null},
        "accuracy_m": 2.5,
        "low_confidence": [{"from_m": 400.0, "to_m": 650.0, "reason": "trees over the bank"}]
    })
}

/// The errors of the document with `banks` at `inputs.banks`, as `Session::open` gives them.
fn errors_with(banks: Value) -> Vec<String> {
    let mut d = document();
    d["inputs"]["banks"] = banks;
    match Session::open(&d.to_string()) {
        Ok(s) => s.validate(),
        Err(e) => vec![e.to_string()],
    }
}

#[test]
fn a_document_may_say_where_its_banks_came_from_and_how_accurate_they_are() {
    // A document with no `inputs.banks` is valid, as before.
    assert!(Session::open(&document().to_string()).is_ok());

    assert_eq!(errors_with(banks()), Vec::<String>::new());
    // Each kind of source. The licence may be a string. The accuracy may be unknown.
    for kind in ["file", "edit", "bank-source"] {
        let mut b = banks();
        b["source"]["kind"] = json!(kind);
        b["source"]["licence"] = json!("CC BY 4.0");
        b["accuracy_m"] = Value::Null;
        assert_eq!(errors_with(b), Vec::<String>::new(), "{kind}");
    }
    // Only the accuracy, as `Session::set_bank_accuracy` leaves it on a document with no source.
    assert_eq!(
        errors_with(json!({"accuracy_m": 1.0})),
        Vec::<String>::new()
    );

    // What the schema refuses, each named by its place in the document.
    let mut wrong_kind = banks();
    wrong_kind["source"]["kind"] = json!("satellite");
    let mut negative = banks();
    negative["accuracy_m"] = json!(-1.0);
    let mut no_reason = banks();
    no_reason["low_confidence"][0]
        .as_object_mut()
        .unwrap()
        .remove("reason");
    let mut unknown = banks();
    unknown["mask_threshold"] = json!(0.5);
    for (bad, place) in [
        (wrong_kind, "/inputs/banks/source/kind"),
        (negative, "/inputs/banks/accuracy_m"),
        (no_reason, "/inputs/banks/low_confidence/0"),
        (unknown, "/inputs/banks"),
    ] {
        let e = errors_with(bad);
        assert!(
            !e.is_empty() && e.iter().all(|m| m.contains(place)),
            "{place}: {e:?}"
        );
    }
}
