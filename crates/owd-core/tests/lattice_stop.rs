//! The lattice search takes its time limit from the caller.
//!
//! The caller passes a `stop` check. The search asks it as it goes. When the check says stop, the
//! search ends with the timed-out outcome: `timed_out` set, not converged, no line. A check that
//! never says stop changes nothing.
//!
//! Label: property

use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, SectionSpec, TerrainGrid, solve_lattice, solve_lattice_until,
};

/// Flat dry ground, 5 m above the grade: 440 m by 240 m in 4 m cells. The search below needs more
/// than 1000 expansions on it.
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

fn section() -> SectionSpec {
    let mut section = SectionSpec::new(HalfWidthSpec::Fixed(10.0));
    section.rake_half_m = 40.0;
    section
}

const START: (f64, f64, f64) = (60.0, 120.0, 0.0);
const GOAL: (f64, f64, f64) = (300.0, 168.0, 0.0);

#[test]
fn a_search_told_to_stop_now_returns_the_timed_out_outcome() {
    let grid = flat_ground();
    let cfg = LatticeConfig::new(50.0).validated().unwrap();
    let mut asked = 0;
    let mut stop_now = || {
        asked += 1;
        true
    };
    let res = solve_lattice_until(&grid, START, GOAL, 0.0, &section(), &cfg, &mut stop_now);
    assert!(res.timed_out, "the result says the search was stopped");
    assert!(!res.converged);
    assert!(res.line.is_none());
    assert!(!res.hit_cap);
    assert!(
        res.expansions <= 1000,
        "the search asks at least once in every 1000 expansions; it made {}",
        res.expansions
    );
    assert_eq!(
        asked, 1,
        "the search ends the first time the check says stop"
    );
}

#[test]
fn a_check_that_never_says_stop_changes_nothing() {
    let grid = flat_ground();
    let cfg = LatticeConfig::new(50.0).validated().unwrap();
    let plain = solve_lattice(&grid, START, GOAL, 0.0, &section(), &cfg);
    assert!(plain.converged && !plain.timed_out);
    assert!(
        plain.expansions > 1000,
        "the search is long enough to be asked"
    );
    let mut asked = 0;
    let mut never = || {
        asked += 1;
        false
    };
    let first = solve_lattice_until(&grid, START, GOAL, 0.0, &section(), &cfg, &mut never);
    let second = solve_lattice_until(&grid, START, GOAL, 0.0, &section(), &cfg, &mut || false);
    assert!(asked >= 1, "the search asked the check");
    assert_eq!(first, plain);
    assert_eq!(second, plain);
}
