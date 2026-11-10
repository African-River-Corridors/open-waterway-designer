//! A terrain file over the reader's memory limit is refused by size, before decoding (ticket C11).
//! The message gives the file's cells, the memory it needs and the limit, so a user can tell what
//! to do. The limit is a parameter below the public entry, so the test needs no gigabytes.
//!
//! Label: property

use owd_format::geotiff::{DEFAULT_GRID_LIMIT_BYTES, read_bytes, read_bytes_within};
use std::path::Path;

fn fixture() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-dem.tif"))
        .unwrap()
}

#[test]
fn a_file_over_the_limit_is_refused_by_size_with_the_three_numbers() {
    let bytes = fixture();
    let dem = read_bytes(&bytes).unwrap();
    let cells = dem.width * dem.height;
    let why = read_bytes_within(&bytes, 1_000)
        .expect_err("a file over the limit is refused")
        .to_string();
    for number in [cells.to_string(), "1000".to_string()] {
        assert!(
            why.contains(&number),
            "the refusal does not give {number}: {why}"
        );
    }
    assert!(
        why.contains(&format!("{}", cells * 8)),
        "the refusal does not give the memory the grid needs ({} bytes): {why}",
        cells * 8
    );
}

#[test]
fn a_file_inside_the_limit_opens_as_with_the_default() {
    let bytes = fixture();
    let dem = read_bytes(&bytes).unwrap();
    let within = read_bytes_within(&bytes, (dem.width * dem.height * 8 * 2) as u64).unwrap();
    assert_eq!(within, dem);
}

// The stated default holds a long river's terrain: 10,000 by 35,000 cells, 8 bytes a cell, on
// native targets. Checked when the tests build, so a smaller default fails the build.
const _: () =
    assert!(cfg!(target_arch = "wasm32") || DEFAULT_GRID_LIMIT_BYTES >= 10_000 * 35_000 * 8);
