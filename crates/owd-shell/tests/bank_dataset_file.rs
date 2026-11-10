//! The bank dataset file on both surfaces: `owd import --banks FILE` and the app shell read the
//! same file into the same design, and `owd export --banks FILE` and the app shell write the same
//! file.
//!
//! Label: property

use owd_cli::commands;
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

const DATE: &str = "2026-01-02";

/// A fresh folder under the system's temporary folder, removed at the end of the test.
struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("owd-bank-file-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A bank dataset file: a straight river 3.2 km long and 400 m wide, in UTM 30N, with its
/// source, its accuracy and one low-confidence stretch.
fn bank_file() -> Value {
    let bank = |y: f64| -> Vec<[f64; 2]> {
        (0..=32)
            .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0 + y])
            .collect()
    };
    let feature = |role: &str, kind: &str, coordinates: Value| {
        json!({
            "type": "Feature",
            "properties": {"role": role},
            "geometry": {"type": kind, "coordinates": coordinates}
        })
    };
    json!({
        "type": "FeatureCollection",
        "crs": {"type": "name", "properties": {"name": "urn:ogc:def:crs:EPSG::32630"}},
        "bank_dataset": {
            "version": 1,
            "source": {"kind": "bank-source", "name": "a bank source", "date": "2026-01-01",
                       "licence": "CC BY 4.0"},
            "accuracy_m": 2.5,
            "low_confidence": [{"from_m": 400.0, "to_m": 650.0, "reason": "trees over the bank"}]
        },
        "features": [
            feature("left_bank", "LineString", json!(bank(200.0))),
            feature("right_bank", "LineString", json!(bank(-200.0))),
            feature("river_polygon", "Polygon", json!([[
                [500_000.0, 599_800.0], [503_200.0, 599_800.0], [503_200.0, 600_200.0],
                [500_000.0, 600_200.0], [500_000.0, 599_800.0]
            ]])),
        ]
    })
}

fn ok(step: &str, done: commands::Result) {
    if let Err(f) = done {
        panic!("`owd {step}` failed: {}", f.message);
    }
}

/// The document without its provenance records: the two surfaces date them on their own.
fn without_provenance(text: &str) -> Value {
    let mut d: Value = serde_json::from_str(text).unwrap();
    d.as_object_mut().unwrap().remove("provenance");
    d
}

#[test]
fn the_same_file_gives_the_same_design_and_the_same_export_on_both_surfaces() {
    let dir = Dir::new("both");
    let given = bank_file();
    let file = dir.0.join("banks.geojson");
    std::fs::write(&file, given.to_string()).unwrap();
    let doc = dir.0.join("d.owd");
    let (doc_s, file_s) = (
        doc.to_string_lossy().into_owned(),
        file.to_string_lossy().into_owned(),
    );

    // The command line: `owd import --banks FILE`.
    ok("new", commands::new(&doc_s, "Banks", Some(DATE), true));
    ok(
        "import --banks",
        commands::import_banks(&doc_s, &file_s, &None, true),
    );
    let cli_text = std::fs::read_to_string(&doc).unwrap();

    // The app shell: the same file text.
    let mut shell = AppState::new_design("Banks", DATE);
    shell.load_bank_dataset(&given.to_string(), DATE).unwrap();
    assert!(shell.is_dirty());
    let shell_text = shell.save_text_for(&doc);

    let cli = without_provenance(&cli_text);
    assert_eq!(cli, without_provenance(&shell_text));
    // One import filled the banks, the polygon and what the source said about them.
    for input in ["left_bank", "right_bank", "river_polygon"] {
        assert!(cli["inputs"][input].is_object(), "{input}");
    }
    assert_eq!(cli["inputs"]["banks"]["accuracy_m"], json!(2.5));
    assert_eq!(
        cli["inputs"]["banks"]["source"],
        given["bank_dataset"]["source"]
    );
    // Both surfaces record where the banks came from.
    for text in [&cli_text, &shell_text] {
        let d: Value = serde_json::from_str(text).unwrap();
        for input in ["left_bank", "right_bank"] {
            assert!(d["provenance"][input].is_object(), "{input}");
        }
    }

    // Export: `owd export --banks FILE` and the shell write the same text.
    let out = dir.0.join("out.geojson");
    ok(
        "export --banks",
        commands::export_banks(&doc_s, false, &out.to_string_lossy()),
    );
    let from_cli = std::fs::read_to_string(&out).unwrap();
    assert_eq!(from_cli, shell.export_bank_dataset(false).unwrap());
    // The file that comes out says what the file that went in said.
    let written: Value = serde_json::from_str(&from_cli).unwrap();
    assert_eq!(written["bank_dataset"], given["bank_dataset"]);
    for bank in [0, 1] {
        assert_eq!(written["features"][bank], given["features"][bank]);
    }

    // A file that is not a bank dataset is refused on both, with the same words, and changes nothing.
    let mut bad = given.clone();
    bad.as_object_mut().unwrap().remove("bank_dataset");
    std::fs::write(&file, bad.to_string()).unwrap();
    let refusal = commands::import_banks(&doc_s, &file_s, &None, true)
        .expect_err("a file with no `bank_dataset` member is not a bank dataset");
    assert_eq!(refusal.code, 2);
    assert!(
        refusal.message.contains("bank_dataset"),
        "{}",
        refusal.message
    );
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), cli_text);
    let said = shell.load_bank_dataset(&bad.to_string(), DATE).unwrap_err();
    assert_eq!(said, refusal.message);
}
