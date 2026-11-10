//! The bank dataset file: one GeoJSON FeatureCollection that holds a whole bank dataset. Its
//! features are the two bank lines and, when there is one, the river polygon, each named by
//! `properties.role`. Its top-level member `bank_dataset` holds the version of the file form, the
//! source, the accuracy and the low-confidence stretches. Export writes the file; import reads it
//! in one go. It is the hand-over format for a bank source outside this tool.
//!
//! Label: property

use owd_format::design::banks::{BankDataset, BankSource, BankSourceKind, LowConfidence};
use owd_format::design::{DesignError, Role, Session};
use serde_json::{Value, json};

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

fn at(x: f64, y: f64) -> [f64; 2] {
    [x + 500_000.0, y + 600_000.0]
}

/// A bank of a straight river 3.2 km long, `y` metres from its middle, in UTM 30N.
fn bank(y: f64) -> Vec<[f64; 2]> {
    (0..=32).map(|i| at(100.0 * f64::from(i), y)).collect()
}

fn ring() -> Vec<[f64; 2]> {
    vec![
        at(0.0, -200.0),
        at(3200.0, -200.0),
        at(3200.0, 200.0),
        at(0.0, 200.0),
        at(0.0, -200.0),
    ]
}

fn stretch() -> Value {
    json!({"from_m": 400.0, "to_m": 650.0, "reason": "trees over the bank"})
}

/// A design with both banks, the river polygon, and what the bank source said about them.
fn design() -> Session {
    let line = |y: f64| {
        format!(
            r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
            json!(bank(y))
        )
    };
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
        json!(ring())
    );
    let mut s = Session::new("Banks", "2026-11-10");
    s.import(&[
        (Role::LeftBank, &line(200.0)),
        (Role::RightBank, &line(-200.0)),
        (Role::RiverPolygon, &polygon),
    ])
    .unwrap();
    let mut doc: Value = serde_json::from_str(&s.to_text()).unwrap();
    doc["inputs"]["banks"] = json!({
        "source": {"kind": "bank-source", "name": "a bank source", "date": "2026-11-10",
                   "licence": "CC BY 4.0"},
        "accuracy_m": 2.5,
        "low_confidence": [stretch()]
    });
    Session::open(&doc.to_string()).unwrap()
}

/// A bank dataset file written by hand, in UTM 30N: `features` and the `bank_dataset` member.
fn file(features: Value, bank_dataset: Value) -> String {
    let mut f: Value = serde_json::from_str(&format!("{{{UTM30}}}")).unwrap();
    f["type"] = json!("FeatureCollection");
    f["features"] = features;
    if !bank_dataset.is_null() {
        f["bank_dataset"] = bank_dataset;
    }
    f.to_string()
}

fn feature(role: &str, kind: &str, coordinates: Value) -> Value {
    json!({
        "type": "Feature",
        "properties": {"role": role},
        "geometry": {"type": kind, "coordinates": coordinates}
    })
}

fn two_banks() -> Value {
    // The order of the features does not matter: the role names each one.
    json!([
        feature("right_bank", "LineString", json!(bank(-200.0))),
        feature("left_bank", "LineString", json!(bank(200.0))),
    ])
}

fn roles(file: &Value) -> Vec<(String, String)> {
    file["features"]
        .as_array()
        .expect("the file has features")
        .iter()
        .map(|f| {
            (
                f["properties"]["role"].as_str().unwrap_or("").to_string(),
                f["geometry"]["type"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn a_dataset_exported_and_imported_again_is_equal() {
    let s = design();
    let want: BankDataset = s.bank_dataset().unwrap();

    // The file, in the project CRS.
    let text = s.export_bank_dataset(false).unwrap();
    let f: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(f["type"], "FeatureCollection");
    let crs = f["crs"]["properties"]["name"].as_str().unwrap_or("");
    assert!(crs.ends_with("32630"), "the file names its CRS: `{crs}`");
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(
        roles(&f),
        vec![
            pair("left_bank", "LineString"),
            pair("right_bank", "LineString"),
            pair("river_polygon", "Polygon")
        ]
    );
    assert_eq!(
        f["bank_dataset"],
        json!({
            "version": 1,
            "source": {"kind": "bank-source", "name": "a bank source", "date": "2026-11-10",
                       "licence": "CC BY 4.0"},
            "accuracy_m": 2.5,
            "low_confidence": [stretch()]
        })
    );

    // Import it into a new design: one call fills the banks, the polygon and `inputs.banks`.
    let mut t = Session::new("Copy", "2026-11-11");
    let report = t.import_bank_dataset(&text).unwrap();
    assert_eq!(report.project_epsg, 32630);
    let names: Vec<&str> = report.inputs.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(names, ["left_bank", "right_bank", "river_polygon"]);
    assert_eq!(report.inputs[0].2, 33, "positions of the left bank");
    assert!(t.validate().is_empty(), "{:?}", t.validate());
    assert_eq!(
        t.document()["inputs"]["banks"],
        s.document()["inputs"]["banks"]
    );
    // The dataset is equal in every part, its hash too.
    let got = t.bank_dataset().unwrap();
    assert_eq!(got, want);
    assert_eq!(
        got.source,
        Some(BankSource {
            kind: BankSourceKind::BankSource,
            name: "a bank source".into(),
            date: "2026-11-10".into(),
            licence: Some("CC BY 4.0".into()),
        })
    );
    assert_eq!(
        got.low_confidence,
        vec![LowConfidence {
            from_m: 400.0,
            to_m: 650.0,
            reason: "trees over the bank".into(),
        }]
    );
    // Export again: the same file, byte for byte.
    assert_eq!(t.export_bank_dataset(false).unwrap(), text);
}

#[test]
fn the_file_in_longitude_and_latitude_gives_the_same_dataset() {
    let s = design();
    let want = s.bank_dataset().unwrap();
    let text = s.export_bank_dataset(true).unwrap();
    let f: Value = serde_json::from_str(&text).unwrap();
    // GeoJSON with no `crs` member is WGS84. UTM zone 30N is centred on 3 degrees west.
    assert!(f.get("crs").is_none_or(Value::is_null));
    let first = &f["features"][0]["geometry"]["coordinates"][0];
    let (lon, lat) = (first[0].as_f64().unwrap(), first[1].as_f64().unwrap());
    assert!(
        (lon + 3.0).abs() < 0.01 && (lat - 5.43).abs() < 0.01,
        "{lon} {lat}"
    );

    let mut t = Session::new("Copy", "2026-11-11");
    assert_eq!(t.import_bank_dataset(&text).unwrap().project_epsg, 32630);
    let got = t.bank_dataset().unwrap();
    // The positions come back to the millimetre; the rest is equal.
    let close = |a: &[[f64; 2]], b: &[[f64; 2]]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(p, q)| (p[0] - q[0]).abs() < 1e-3 && (p[1] - q[1]).abs() < 1e-3)
    };
    assert!(close(&got.left, &want.left) && close(&got.right, &want.right));
    assert!(close(
        &got.river.as_ref().unwrap().exterior,
        &want.river.as_ref().unwrap().exterior
    ));
    assert_eq!(got.source, want.source);
    assert_eq!(got.accuracy_m, want.accuracy_m);
    assert_eq!(got.low_confidence, want.low_confidence);
}

#[test]
fn a_file_with_an_accuracy_keeps_it_through_import() {
    // The least a bank source must write: the two lines, the version and its accuracy.
    let text = file(two_banks(), json!({"version": 1, "accuracy_m": 3.0}));
    let mut s = Session::new("Handed over", "2026-11-10");
    s.import_bank_dataset(&text).unwrap();
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert_eq!(s.document()["inputs"]["banks"]["accuracy_m"], json!(3.0));
    let d = s.bank_dataset().unwrap();
    assert_eq!(d.accuracy_m, Some(3.0));
    assert_eq!(d.source, None);
    assert!(d.low_confidence.is_empty());
    assert_eq!(d.left, bank(200.0));
    assert_eq!(d.right, bank(-200.0));
    // The accuracy is still there after an export and an import.
    let mut t = Session::new("Copy", "2026-11-11");
    t.import_bank_dataset(&s.export_bank_dataset(false).unwrap())
        .unwrap();
    assert_eq!(t.bank_dataset().unwrap().accuracy_m, Some(3.0));

    // A file with no accuracy replaces the dataset: the old accuracy does not stay behind.
    t.import_bank_dataset(&file(two_banks(), json!({"version": 1})))
        .unwrap();
    assert_eq!(t.bank_dataset().unwrap().accuracy_m, None);
}

#[test]
fn a_file_that_is_not_a_bank_dataset_is_refused_and_changes_nothing() {
    let left = || feature("left_bank", "LineString", json!(bank(200.0)));
    let right = || feature("right_bank", "LineString", json!(bank(-200.0)));
    let v1 = || json!({"version": 1});
    let cases = [
        // (the file, a word the refusal must hold)
        (file(two_banks(), Value::Null), "bank_dataset"),
        (file(two_banks(), json!({"version": 2})), "version"),
        (file(json!([left()]), v1()), "right_bank"),
        (
            file(
                json!([
                    left(),
                    right(),
                    feature("island", "Polygon", json!([ring()]))
                ]),
                v1(),
            ),
            "island",
        ),
        (file(json!([left(), left(), right()]), v1()), "left_bank"),
        (
            file(two_banks(), json!({"version": 1, "accuracy_m": -1.0})),
            "accuracy_m",
        ),
        (
            file(
                two_banks(),
                json!({"version": 1, "low_confidence": [{"from_m": 0.0, "to_m": 50.0}]}),
            ),
            "reason",
        ),
    ];
    let mut s = design();
    let before = s.to_text();
    for (text, word) in cases {
        match s.import_bank_dataset(&text) {
            Err(DesignError::Input(m)) => assert!(m.contains(word), "`{word}` is not in: {m}"),
            other => panic!("`{word}`: expected a refusal, got {other:?}"),
        }
        assert_eq!(s.to_text(), before, "`{word}`: a refusal changes nothing");
    }
    // A design with no banks has no dataset to export.
    let e = Session::new("Empty", "2026-11-10")
        .export_bank_dataset(false)
        .unwrap_err();
    assert!(matches!(e, DesignError::Input(_)), "{e:?}");
}
