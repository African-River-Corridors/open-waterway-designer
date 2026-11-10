//! `owd check-banks` runs the bank checks of the core. It must not need the bank extraction crate
//! (`owd-reach`), which is to leave the repository. `owd-reach` itself calls the checks of the core
//! and keeps no copy of them.
//!
//! Label: property

use std::path::Path;

fn source(path: &str) -> Option<String> {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).ok()
}

#[test]
fn the_check_banks_command_takes_its_checks_from_the_core() {
    let command = source("src/check_banks.rs").expect("crates/owd-cli/src/check_banks.rs");
    assert!(
        !command.contains("owd_reach"),
        "crates/owd-cli/src/check_banks.rs still uses `owd_reach`"
    );
    assert!(
        command.contains("owd_core::banks::check"),
        "crates/owd-cli/src/check_banks.rs does not use `owd_core::banks::check`"
    );
}

#[test]
fn the_bank_extraction_crate_keeps_no_copy_of_the_checks() {
    // Once the extraction has left the repository there is no file to read: nothing to check.
    let Some(reach) = source("../owd-reach/src/bankcheck.rs") else {
        return;
    };
    assert!(
        reach.contains("owd_core::banks::check"),
        "crates/owd-reach/src/bankcheck.rs does not use `owd_core::banks::check`"
    );
    for copy in ["fn assess(", "fn lines_cross(", "pub struct CheckParams"] {
        assert!(
            !reach.contains(copy),
            "crates/owd-reach/src/bankcheck.rs still holds `{copy}`"
        );
    }
}
