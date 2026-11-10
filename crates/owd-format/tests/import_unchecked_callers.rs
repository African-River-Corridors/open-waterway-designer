//! `Session::import_unchecked` keeps banks that cross. It exists only for banks a machine made, and
//! its one caller is the bank extraction (`owd_reach::banks::record_banks_as`). A user's files come
//! in through `Session::import`, which refuses crossing banks: no other crate may call it.
//!
//! Label: property

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn only_the_bank_extraction_calls_the_unchecked_import() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "app/src-tauri/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    let callers: Vec<String> = files
        .iter()
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains(".import_unchecked(")))
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|p| !p.ends_with("tests/import_unchecked_callers.rs"))
        .collect();
    assert_eq!(callers, vec!["crates/owd-reach/src/banks.rs"]);
}
