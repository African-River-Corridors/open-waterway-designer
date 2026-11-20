//! Given structures (spec 0017, ticket 14) set through the command line (`owd structures --given`)
//! and through the app shell give the same document, and the same refusal: both call
//! `Session::set_structures_text` (the one path rule of ADR 0015).
//!
//! Label: property

use owd_cli::structures::{StructureFlags, structures};
use owd_shell::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn both(name: &str, given: &Value) -> (Result<Value, String>, Result<Value, String>) {
    let dir = Dir(std::env::temp_dir().join(format!("owd-structs-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let text = std::fs::read_to_string(web.join("river-example.owd")).unwrap();
    let doc = dir.0.join("d.owd");
    std::fs::write(&doc, &text).unwrap();
    let file = dir.0.join("structures.json");
    std::fs::write(&file, given.to_string()).unwrap();

    let mut shell = AppState::open(&doc, &text, None).unwrap();
    let from_shell = shell
        .set_structures(&given.to_string())
        .map(|()| serde_json::from_str(&shell.save_text_for(&doc)).unwrap());

    let flags = StructureFlags {
        given: Some(file.to_string_lossy().into_owned()),
        clear: false,
    };
    let from_cli = structures(&doc.to_string_lossy(), &flags, &None, true)
        .map(|()| serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap())
        .map_err(|f| f.message);
    (from_cli, from_shell)
}

fn item(c: f64, level: f64) -> Value {
    json!({"name": format!("at {c}"), "kind": "lock", "site": {"chainage": c},
           "upstream_level_m": level, "height_m": 5.0,
           "cost": {"amount": 2000000.0, "currency": "USD"}})
}

#[test]
fn the_same_structures_give_the_same_document_through_both_surfaces() {
    let given = json!({"mode": "given", "items": [item(1000.0, 2.0), item(2000.0, 4.0)],
                       "pool_levels": {"below_m": 0.3, "above_m": 0.7}});
    let (cli, shell) = both("same", &given);
    let (cli, shell) = (cli.unwrap(), shell.unwrap());
    assert_eq!(cli, shell);
    assert_eq!(cli["inputs"]["structures"], given);
}

#[test]
fn levels_that_step_up_are_refused_the_same_way_by_both() {
    let given = json!({"mode": "given", "items": [item(1000.0, 4.0), item(2000.0, 2.0)]});
    let (cli, shell) = both("stepped", &given);
    let (cli, shell) = (cli.unwrap_err(), shell.unwrap_err());
    assert!(cli.contains("WEIRS_LEVELS_NOT_STEPPED"), "{cli}");
    assert_eq!(cli, shell);
}
