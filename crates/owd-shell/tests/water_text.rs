//! The water level as typed is read in one place: the app shell.
//!
//! The desktop app and the web build both send the level and the fall per metre as text. The shell
//! reads the text, so the two builds cannot drift apart, and neither holds a reader of its own.
//!
//! Label: property

use owd_shell::state::AppState;
use serde_json::Value;
use std::path::Path;

/// The example design: it has a centreline, which a fall per metre needs.
fn design() -> AppState {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-reroute.owd");
    AppState::open(&file, &std::fs::read_to_string(&file).unwrap(), None).unwrap()
}

fn water(state: &AppState) -> Value {
    state.snapshot()["document"]["inputs"]["water_level"].clone()
}

#[test]
fn the_shell_reads_the_level_and_the_fall_as_typed() {
    for (level, fall, numbers) in [
        ("12.5", "", (12.5, None)),
        (" 13.5 ", " ", (13.5, None)),
        ("12.5", "0.0001", (12.5, Some(0.0001))),
        ("-3", " 2e-4 ", (-3.0, Some(2e-4))),
    ] {
        let (mut typed, mut given) = (design(), design());
        typed
            .set_water_text(level, fall)
            .unwrap_or_else(|why| panic!("({level:?}, {fall:?}) was refused: {why}"));
        given.set_water(numbers.0, numbers.1).unwrap();
        assert_ne!(
            water(&typed),
            water(&design()),
            "the water level did not change"
        );
        assert_eq!(water(&typed), water(&given), "({level:?}, {fall:?})");
    }
}

#[test]
fn the_shell_refuses_text_that_is_not_a_number_with_the_words_the_app_shows_today() {
    for (level, fall, words) in [
        ("abc", "", "the water level must be a number, not `abc`"),
        ("1,5", "", "the water level must be a number, not `1,5`"),
        ("", "", "give a water level in metres"),
        ("  ", "0.0001", "give a water level in metres"),
        (
            "12.5",
            "steep",
            "the fall per metre must be a number, not `steep`",
        ),
    ] {
        let mut state = design();
        assert_eq!(
            state.set_water_text(level, fall),
            Err(words.to_string()),
            "({level:?}, {fall:?})"
        );
        assert_eq!(
            water(&state),
            water(&design()),
            "a refused level changed the design"
        );
    }
}

#[test]
fn neither_build_of_the_app_reads_the_text_itself() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in ["app/src-tauri/src/lib.rs", "crates/owd-web/src/lib.rs"] {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !source.contains("must be a number, not"),
            "{file} still reads typed numbers itself (`must be a number, not`)"
        );
    }
}
