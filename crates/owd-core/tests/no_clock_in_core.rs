//! The core reads no clock.
//!
//! A function of the core gives the same answer for the same inputs, on every machine and in the
//! browser. So no source file of the core reads the time, and a lattice search run twice gives
//! two equal results. A caller that wants a time limit passes it in.
//!
//! Label: property

use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, SectionSpec, TerrainGrid, solve_lattice,
};
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The rule. In every `.rs` file under `crates/owd-core/src`, read the lines above the first line
/// that is `#[cfg(test)]` (the unit tests come after it). Leave out the lines that start with
/// `//`. None of the lines left holds `Instant::now` or `SystemTime::now`.
#[test]
fn no_source_file_of_the_core_reads_the_clock() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no source files under {}", src.display());
    let mut found = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line == "#[cfg(test)]" {
                break;
            }
            if line.starts_with("//") {
                continue;
            }
            if line.contains("Instant::now") || line.contains("SystemTime::now") {
                found.push(format!(
                    "{}:{}: {line}",
                    file.strip_prefix(&src).unwrap().display(),
                    i + 1
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "the core reads the clock:\n{}",
        found.join("\n")
    );
}

/// Flat dry ground, 5 m above the grade: 440 m by 240 m in 4 m cells.
fn flat_ground() -> TerrainGrid {
    let (rows, cols) = (60, 110);
    TerrainGrid {
        ground: vec![vec![5.0; cols]; rows],
        wet: vec![vec![false; cols]; rows],
        valid: vec![vec![true; cols]; rows],
        x0: 0.0,
        y0: 240.0,
        cell_m: 4.0,
    }
}

#[test]
fn the_same_search_run_twice_gives_two_equal_results() {
    let grid = flat_ground();
    let mut section = SectionSpec::new(HalfWidthSpec::Fixed(10.0));
    section.rake_half_m = 40.0;
    let cfg = LatticeConfig::new(50.0).validated().unwrap();
    let (start, goal) = ((60.0, 120.0, 0.0), (300.0, 168.0, 0.0));
    let first = solve_lattice(&grid, start, goal, 0.0, &section, &cfg);
    let second = solve_lattice(&grid, start, goal, 0.0, &section, &cfg);
    assert!(first.converged, "the search finds a line");
    assert_eq!(
        first, second,
        "two runs of one search differ: a result holds something read from the clock"
    );
}
