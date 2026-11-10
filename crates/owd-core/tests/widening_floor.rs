//! Ticket C29: bend widening with a named floor. With the floor at the minimum radius (the
//! default) a bend tighter than the minimum radius gets the widening of the minimum radius. With
//! no floor it gets the widening of its own radius.
//!
//! Label: property
//!
//! Closed form. The rule set here (PIANC WG 141) widens a bend by `c × L² / R`, so the widening
//! is in inverse proportion to the radius. The test reads the widening at the minimum radius from
//! the sized shape and works out the rest: at half the minimum radius the widening is twice that,
//! at twice the minimum radius it is half of it.

use owd_core::cut_model::{
    WIDEN_RMAX, WideningFloor, widened_hb_from_shape, widened_hb_from_shape_floor,
};
use owd_core::geom::Pt;
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::rules::engine::selection_for_old_ruleset;
use owd_core::rules::rulesets::{ChannelShape, channel_shape_for_custom};
use std::f64::consts::FRAC_PI_2;

/// A single hull of 50 m by 10 m, sized under PIANC WG 141 quality C.
fn shape() -> ChannelShape {
    let v = CustomVessel {
        name: "test hull".into(),
        loa_m: 50.0,
        beam_m: 10.0,
        draught_m: 2.0,
        formation: Formation::Motor,
        n_abreast: None,
        n_in_line: None,
        gb_class: None,
        air_draught_m: None,
        deadweight_t: None,
    };
    channel_shape_for_custom(&selection_for_old_ruleset("PIANC-C").unwrap(), &v).unwrap()
}

/// Stations: 300 m straight, a quarter circle of radius `r` to the left in 16 equal steps, 300 m
/// straight. Returns the stations and the index of the middle of the bend.
fn line_with_bend(r: f64) -> (Vec<Pt>, usize) {
    let mut xy: Vec<Pt> = (0..30)
        .map(|i| [-300.0 + 10.0 * f64::from(i), 0.0])
        .collect();
    let first = xy.len();
    for i in 0..=16 {
        let t = FRAC_PI_2 * f64::from(i) / 16.0;
        xy.push([r * libm::sin(t), r - r * libm::cos(t)]);
    }
    for i in 1..=30 {
        xy.push([r, r + 10.0 * f64::from(i)]);
    }
    (xy, first + 8)
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what}: got {got}, want {want}");
}

#[test]
fn a_bend_of_half_the_minimum_radius_gets_its_own_widening_with_the_floor_off() {
    let s = shape();
    let (r_min, straight, at_r_min) = (
        s.min_bend_radius_m,
        s.bottom_width_straight_m,
        s.bend_widening_at_rmin_m,
    );
    assert!(at_r_min > 0.0, "the shape widens at the minimum radius");
    // The rule: c × L² / R. `c` from the numbers the shape holds.
    let c = at_r_min * r_min / (s.loa_m * s.loa_m);
    let r = r_min / 2.0;
    let own = c * s.loa_m * s.loa_m / r;
    close(own, 2.0 * at_r_min, "the closed form");

    let (xy, mid) = line_with_bend(r);
    let off = widened_hb_from_shape_floor(&xy, &s, WIDEN_RMAX, WideningFloor::None);
    let on = widened_hb_from_shape_floor(&xy, &s, WIDEN_RMAX, WideningFloor::MinRadius);
    assert_eq!(off.len(), xy.len());
    assert_eq!(on.len(), xy.len());
    close(
        off[mid],
        straight / 2.0 + own / 2.0,
        "the floor off, mid bend",
    );
    close(
        on[mid],
        straight / 2.0 + at_r_min / 2.0,
        "the floor on, mid bend",
    );
    // On the straights, well away from the bend, there is no widening either way.
    for i in [0, 10, xy.len() - 11, xy.len() - 1] {
        close(off[i], straight / 2.0, "the floor off, straight");
        close(on[i], straight / 2.0, "the floor on, straight");
    }
    // No station is narrower with the floor off than with it on.
    for (a, b) in off.iter().zip(&on) {
        assert!(a >= b, "{a} < {b}");
    }
}

#[test]
fn the_default_is_the_floor_at_the_minimum_radius() {
    let s = shape();
    assert_eq!(WideningFloor::default(), WideningFloor::MinRadius);
    let (xy, _) = line_with_bend(s.min_bend_radius_m / 2.0);
    assert_eq!(
        widened_hb_from_shape(&xy, &s, WIDEN_RMAX),
        widened_hb_from_shape_floor(&xy, &s, WIDEN_RMAX, WideningFloor::MinRadius),
        "the function with no choice keeps the floor"
    );
}

#[test]
fn a_bend_wider_than_the_minimum_radius_is_the_same_either_way() {
    let s = shape();
    let (xy, mid) = line_with_bend(2.0 * s.min_bend_radius_m);
    let off = widened_hb_from_shape_floor(&xy, &s, WIDEN_RMAX, WideningFloor::None);
    let on = widened_hb_from_shape_floor(&xy, &s, WIDEN_RMAX, WideningFloor::MinRadius);
    assert_eq!(off, on);
    close(
        off[mid],
        s.bottom_width_straight_m / 2.0 + s.bend_widening_at_rmin_m / 4.0,
        "twice the minimum radius, mid bend",
    );
}

#[test]
fn the_choice_has_a_name() {
    assert_eq!(WideningFloor::MinRadius.name(), "min_radius");
    assert_eq!(WideningFloor::None.name(), "none");
    assert_eq!(
        WideningFloor::from_name("min_radius"),
        Some(WideningFloor::MinRadius)
    );
    assert_eq!(WideningFloor::from_name("none"), Some(WideningFloor::None));
    assert_eq!(WideningFloor::from_name("0"), None);
}
