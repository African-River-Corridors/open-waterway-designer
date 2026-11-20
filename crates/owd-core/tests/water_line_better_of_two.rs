//! The water-aware rolling circle where the channel does not fit the river (issue #503): at a
//! bend that ends infeasible, the line is the better of the fairway fit's and the plain rolling circle's.
//! The tests prove this on the synthetic rivers below, over the whole line. They do not prove it
//! on every river: between bend windows the line can still be tighter than the plain one (#514).
//!
//! The rivers are synthetic, built in the test: a sine centreline with a vertex every 10 m, and a
//! water polygon that is wide between the bends and narrow at each apex (narrower than the bottom
//! width the bend needs). The roll runs as the command line runs it: R_min 240 m, step 10 m, two
//! passes, both sides, the default gap and check chord, with and without water.
//!
//! Before the fix, on the river of wavelength 1200 m and amplitude 152 m (water 40 m at the apex,
//! 120 m between; a channel needing 45.5 m) every bend took the fairway fit's line, which holds
//! R_min but runs out of the water: the width missing from the water summed to 137,215 m2 on 5 m
//! stations, against 385 m2 for the plain line.
//!
//! Label: property (no self-crossing; R_min held on the default chord; no more channel outside the
//! water than the plain line; the bends still reported infeasible).

use owd_core::alignment::loops::is_self_intersecting;
use owd_core::alignment::navigable::{local_radius_on, water_width_on};
use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::alignment::rolling_circle::{
    Rung, WaterAwareResult, WaterInput, rolling_circle_with_water,
};
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::rules::engine::selection_for_old_ruleset;
use owd_core::rules::rulesets::{ChannelShape, channel_shape_for};

const R_MIN: f64 = 240.0;
const STATION_M: f64 = 5.0;

fn shape(class: &str) -> ChannelShape {
    channel_shape_for(&selection_for_old_ruleset("PIANC-B").unwrap(), class, None).unwrap()
}

/// River: sine centreline (wavelength, amplitude), water narrow at each apex.
fn river(wl: f64, amp: f64, narrow: f64, wide: f64, len: f64) -> (Vec<Pt>, Polygon) {
    let n = (len / 10.0) as usize;
    let mut cl = vec![];
    let (mut l, mut r) = (vec![], vec![]);
    let k = 2.0 * core::f64::consts::PI / wl;
    for i in 0..=n {
        let x = i as f64 * 10.0;
        let p = [x, amp * libm::sin(k * x)];
        cl.push(p);
        let d = [1.0, amp * k * libm::cos(k * x)];
        let m = libm::hypot(d[0], d[1]);
        let nrm = [-d[1] / m, d[0] / m];
        let w = narrow + (wide - narrow) * libm::fabs(libm::cos(k * x));
        l.push([p[0] + nrm[0] * w / 2.0, p[1] + nrm[1] * w / 2.0]);
        r.push([p[0] - nrm[0] * w / 2.0, p[1] - nrm[1] * w / 2.0]);
    }
    let mut ext = l;
    r.reverse();
    ext.extend(r);
    ext.push(ext[0]);
    (
        cl,
        Polygon {
            exterior: ext,
            holes: vec![],
        },
    )
}

struct Case {
    name: &'static str,
    class: &'static str,
    wavelength: f64,
    amplitude: f64,
    narrow: f64,
    wide: f64,
}

const CASES: [Case; 3] = [
    Case {
        name: "apex 40 m, between 120 m",
        class: "CEMT_IV",
        wavelength: 1200.0,
        amplitude: 152.0,
        narrow: 40.0,
        wide: 120.0,
    },
    Case {
        name: "apex 40 m, between 80 m",
        class: "CEMT_IV",
        wavelength: 1000.0,
        amplitude: 120.0,
        narrow: 40.0,
        wide: 80.0,
    },
    Case {
        name: "wider channel, apex 40 m, between 120 m",
        class: "CEMT_Va",
        wavelength: 1200.0,
        amplitude: 152.0,
        narrow: 40.0,
        wide: 120.0,
    },
];

fn roll(c: &Case, water: bool) -> (WaterAwareResult, Polygon, ChannelShape) {
    let (cl, poly) = river(c.wavelength, c.amplitude, c.narrow, c.wide, 3600.0);
    let s = shape(c.class);
    let wi = WaterInput {
        water: core::slice::from_ref(&poly),
        shape: &s,
        clearance_m: 0.0,
        smooth_m: 2.0,
        r_pref_m: None,
    };
    let sides = vec!["left".to_string(), "right".to_string()];
    let r = rolling_circle_with_water(
        &Geometry::LineString(cl),
        R_MIN,
        10.0,
        2,
        &sides,
        None,
        None,
        water.then_some(&wi),
    )
    .unwrap();
    (r, poly, s)
}

/// The channel width the water lacks, summed over 5 m stations (m2): where the water is narrower
/// than the bottom width the local radius needs, the shortfall; off the water, the whole width.
fn missing_width(line: &[Pt], poly: &Polygon, s: &ChannelShape, chord: f64) -> f64 {
    let len = linear::length(line);
    let mut sum = 0.0;
    let mut at = 0.0;
    while at < len {
        let need = s.bottom_width_at_m(local_radius_on(line, at, chord));
        let have = water_width_on(line, at, core::slice::from_ref(poly));
        sum += (need - have).max(0.0) * STATION_M;
        at += STATION_M;
    }
    sum
}

#[test]
fn the_water_line_matches_the_plain_line_on_these_rivers_where_the_channel_does_not_fit() {
    for c in &CASES {
        let (plain, poly, s) = roll(c, false);
        let (water, _, _) = roll(c, true);
        let chord = water.result.radius_check_chord_m;
        let name = c.name;
        assert!(
            !is_self_intersecting(&water.result.centreline),
            "{name}: the water line crosses itself"
        );
        let radius = min_radius_on_chord(&water.result.centreline, chord).unwrap();
        assert!(
            radius_ok(radius, R_MIN),
            "{name}: radius {radius} on the chord"
        );
        let (mp, mw) = (
            missing_width(&plain.result.centreline, &poly, &s, chord),
            missing_width(&water.result.centreline, &poly, &s, chord),
        );
        assert!(
            mw <= mp + 1e-6,
            "{name}: width missing from the water {mw:.0} m2, plain line {mp:.0} m2"
        );
    }
}

#[test]
fn a_bend_that_ends_infeasible_is_still_reported_with_its_stretch() {
    let (water, _, _) = roll(&CASES[0], true);
    assert!(!water.rungs.is_empty());
    assert!(water.rungs.iter().all(|b| b.rung == Rung::Infeasible));
    let line = &water.result.centreline;
    for b in &water.rungs {
        let (a, c) = (
            linear::project(line, b.tangent_point),
            linear::project(line, b.contact_point),
        );
        assert!(
            water
                .infeasible
                .iter()
                .any(|s| s.end_chainage_m >= a.min(c) && s.start_chainage_m <= a.max(c)),
            "bend at {:?} has no stretch",
            b.tangent_point
        );
    }
}
