//! The water-aware rolling circle between bend windows (issue #514, follow-up to #503): over the
//! whole line, the water-aware line holds R_min wherever the plain line holds it.
//!
//! The river is closed-form: two sine banks, 1,200 m wavelength, 600 m amplitude, each offset 75,
//! 125 or 150 m (vertically) from the centre sine, 6 km long; the line rolled is the centreline
//! built from the banks, as the app builds it. Every bend is
//! too tight for the channel to fit the water, so every bend takes the last rung. The roll runs as
//! the command line runs it: R_min 255 m, step 10 m, two passes, both sides, the default gap and
//! check chord, with and without water.
//!
//! Label: property (R_min held on the default chord along the whole line; no self-crossing).

use owd_core::alignment::centreline::build_centreline;
use owd_core::alignment::loops::is_self_intersecting;
use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::alignment::rolling_circle::{WaterInput, rolling_circle_with_water};
use owd_core::geom::{Geometry, Polygon, Pt};
use owd_core::rules::engine::selection_for_old_ruleset;
use owd_core::rules::rulesets::{ChannelShape, channel_shape_for};

const R_MIN: f64 = 255.0;

fn shape() -> ChannelShape {
    channel_shape_for(
        &selection_for_old_ruleset("PIANC-B").unwrap(),
        "CEMT_IV",
        None,
    )
    .unwrap()
}

fn sine(amp: f64, wl: f64, off: f64, len: f64) -> Vec<Pt> {
    let n = (len / 10.0) as usize;
    (0..=n)
        .map(|i| {
            let x = i as f64 * 10.0;
            [
                x,
                amp * libm::sin(2.0 * core::f64::consts::PI * x / wl) + off,
            ]
        })
        .collect()
}

/// (plain radius, water radius, water line crosses itself) on the default chord.
fn roll(half_width: f64, wl: f64, amp: f64) -> (f64, f64, bool) {
    let left = sine(amp, wl, half_width, 6000.0);
    let right = sine(amp, wl, -half_width, 6000.0);
    // The line the app rolls: the centreline built from the two banks.
    let cl = build_centreline(&left, &right, 20.0, 5, 2).unwrap();
    let mut ring = left;
    let mut right = right;
    right.reverse();
    ring.extend(right);
    ring.push(ring[0]);
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let s = shape();
    let wi = WaterInput {
        water: core::slice::from_ref(&poly),
        shape: &s,
        clearance_m: 0.0,
        smooth_m: 0.0,
        r_pref_m: None,
    };
    let sides = vec!["left".to_string(), "right".to_string()];
    let run = |water: bool| {
        rolling_circle_with_water(
            &Geometry::LineString(cl.clone()),
            R_MIN,
            10.0,
            2,
            &sides,
            None,
            None,
            water.then_some(&wi),
        )
        .unwrap()
    };
    let (plain, water) = (run(false), run(true));
    let chord = water.result.radius_check_chord_m;
    (
        min_radius_on_chord(&plain.result.centreline, chord).unwrap(),
        min_radius_on_chord(&water.result.centreline, chord).unwrap(),
        is_self_intersecting(&water.result.centreline),
    )
}

#[test]
fn the_water_line_holds_r_min_wherever_the_plain_line_holds_it() {
    for hw in [75.0, 125.0, 150.0] {
        let (plain, water, crossed) = roll(hw, 1200.0, 600.0);
        assert!(!crossed, "half-width {hw}: the water line crosses itself");
        if radius_ok(plain, R_MIN) {
            assert!(
                radius_ok(water, R_MIN),
                "half-width {hw}: plain {plain:.1} m holds R_min, water {water:.1} m does not"
            );
        }
    }
}
