//! The bank checks in the core: each bank on its own side, no crossing, a plausible width. They
//! read two bank lines and a rough centreline, in metres. They report and change nothing.
//!
//! Label: property

use owd_core::banks::check::{CheckParams, Checks, check_banks, lines_cross};

type Pt = [f64; 2];

/// A gently winding line 3 km long running east, `y` metres north of the river axis, with a
/// vertex every 50 m.
fn east(y: f64) -> Vec<Pt> {
    (0..=60)
        .map(|i| {
            let x = 50.0 * f64::from(i);
            [500_000.0 + x, 600_000.0 + y + 40.0 * libm::sin(x / 500.0)]
        })
        .collect()
}

/// The north bank with a throat between 1.3 km and 1.7 km: it comes in by up to 140 m, so the
/// river narrows from 200 m to 60 m.
fn throat() -> Vec<Pt> {
    let mut bank = east(100.0);
    for (i, p) in bank.iter_mut().enumerate() {
        let x = 50.0 * i as f64;
        p[1] -= 140.0 * (1.0 - ((x - 1500.0).abs() / 200.0).min(1.0));
    }
    bank
}

fn failed(left: &[Pt], right: &[Pt]) -> Vec<(f64, Checks)> {
    check_banks(left, right, &east(0.0), &CheckParams::at_step(20.0))
}

#[test]
fn a_sound_pair_of_banks_passes_every_check() {
    let (left, right) = (east(100.0), east(-100.0));
    assert_eq!(failed(&left, &right), vec![]);
    assert_eq!(lines_cross(&left, &right), None);
}

#[test]
fn a_throat_fails_the_width_check_and_nothing_else() {
    let f = failed(&throat(), &east(-100.0));
    assert!(
        !f.is_empty(),
        "the throat is half the width of its neighbours"
    );
    for (at, c) in &f {
        assert_eq!(c.names(), vec!["width-narrow"], "at {at} m");
        assert!((1300.0..1700.0).contains(at), "at {at} m");
    }
    assert_eq!(lines_cross(&throat(), &east(-100.0)), None);
}

#[test]
fn banks_with_their_names_the_wrong_way_round_fail_the_side_check() {
    // The centreline runs east: the north bank is the left one.
    let f = failed(&east(-100.0), &east(100.0));
    assert!(f.len() > 100, "{} stations of 20 m", f.len());
    assert!(f.iter().all(|(_, c)| c.side), "{f:?}");
}

#[test]
fn a_crossing_is_found_and_placed() {
    let mut left = east(100.0);
    left[30][1] -= 300.0; // a spike across the south bank at 1.5 km
    let x = lines_cross(&left, &east(-100.0)).expect("the banks cross");
    assert!((501_400.0..501_600.0).contains(&x[0]), "{x:?}");
    assert!((x[1] - 600_000.0).abs() < 200.0, "{x:?}");
}
