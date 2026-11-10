//! Ticket C5: the longest vessel for each beam. For one beam, the sweep searches the length: the
//! longest single hull of that beam that fits the river as it stands, to a stated step, between
//! stated bounds. It says what stops the next length.
//!
//! Label: property
//!
//! Closed form, on rivers built in the test. The widths and the radius multiple come from the
//! rule set (through the sized shape of a hull), not from the search.
//!
//! - One bend of centreline radius R, in water just wide enough for the hull of length R / k at
//!   that bend (k is the rule set's radius multiple: R_min = k × length). The centreline itself
//!   fits every length up to R / k. A line inside the water can ease the bend, but only so far:
//!   with a spare half-width s beside the straight fairway, no line through a quarter turn has a
//!   radius over R + (3 + 2√2) × s. So the longest length lies from R / k to that radius over k.
//! - A straight river of width W. A beam whose straight fairway is narrower than W fits at every
//!   length. A beam whose straight fairway is wider than W fits at no length.

use owd_core::envelope::{
    BeamResult, LengthSearch, Settings, Size, envelope, longest_for_beam, size_result,
};
use owd_core::geom::{Polygon, Pt};
use owd_core::rules::custom::{CustomVessel, Formation};
use owd_core::rules::engine::selection_for_old_ruleset;
use owd_core::rules::rulesets::{ChannelShape, channel_shape_for_custom};
use std::f64::consts::{FRAC_PI_2, SQRT_2};

/// The channel the default rule set sizes for a single hull, as `size_result` sizes it.
fn shape(loa_m: f64, beam_m: f64) -> ChannelShape {
    let d = Settings::default();
    let v = CustomVessel {
        name: "hull".into(),
        loa_m,
        beam_m,
        draught_m: d.draught_m,
        formation: Formation::Motor,
        n_abreast: None,
        n_in_line: None,
        gb_class: None,
        air_draught_m: None,
        deadweight_t: None,
    };
    channel_shape_for_custom(&selection_for_old_ruleset(&d.rule_set).unwrap(), &v).unwrap()
}

/// Water `width` wide along `centre` (points with their left normals).
fn water_along(c: &[(Pt, Pt)], width: f64) -> Polygon {
    let h = width / 2.0;
    let mut ring: Vec<Pt> = c
        .iter()
        .map(|(p, n)| [p[0] - h * n[0], p[1] - h * n[1]])
        .collect();
    ring.extend(
        c.iter()
            .rev()
            .map(|(p, n)| [p[0] + h * n[0], p[1] + h * n[1]]),
    );
    ring.push(ring[0]);
    Polygon {
        exterior: ring,
        holes: vec![],
    }
}

fn straight(len: f64) -> Vec<(Pt, Pt)> {
    (0..=(len / 10.0) as usize)
        .map(|i| ([10.0 * i as f64, 0.0], [0.0, 1.0]))
        .collect()
}

/// `tail` m straight, a quarter turn to the left of radius `rc`, `tail` m straight.
fn bend(rc: f64, tail: f64) -> Vec<(Pt, Pt)> {
    let nt = (tail / 10.0) as usize;
    let mut c: Vec<(Pt, Pt)> = (0..nt)
        .map(|i| ([-tail + 10.0 * i as f64, 0.0], [0.0, 1.0]))
        .collect();
    let n = (rc * FRAC_PI_2 / 5.0).ceil() as usize;
    for i in 0..n {
        let t = -FRAC_PI_2 + FRAC_PI_2 * i as f64 / n as f64;
        let (s, co) = (libm::sin(t), libm::cos(t));
        c.push(([rc * co, rc + rc * s], [-co, -s]));
    }
    for i in 0..=nt {
        c.push(([rc, rc + 10.0 * i as f64], [-1.0, 0.0]));
    }
    c
}

/// The reference line: the centre, 20 m short of the water's two ends.
fn line(c: &[(Pt, Pt)]) -> Vec<Pt> {
    c[2..c.len() - 2].iter().map(|(p, _)| *p).collect()
}

fn settings(min_m: f64, max_m: f64, step_m: f64) -> Settings {
    Settings {
        length_search: LengthSearch {
            min_m,
            max_m,
            step_m,
        },
        ..Settings::default()
    }
}

#[test]
fn one_bend_sets_the_longest_length_between_the_two_closed_form_bounds() {
    let (rc, tail, beam) = (200.0, 600.0, 2.0);
    // The radius multiple of the rule set, the same at two lengths.
    let k = shape(50.0, beam).min_bend_radius_m / 50.0;
    assert_eq!(shape(120.0, beam).min_bend_radius_m, k * 120.0);
    // The hull whose minimum radius is the bend's radius, and the width it needs there.
    let at_bend = shape(rc / k, beam);
    assert_eq!(at_bend.min_bend_radius_m, rc);
    let water_width = at_bend.bottom_width_at_rmin_m + 10.0;
    let spare = (water_width - at_bend.bottom_width_straight_m) / 2.0;
    let (lowest, highest) = (rc / k, (rc + (3.0 + 2.0 * SQRT_2) * spare) / k);

    let c = bend(rc, tail);
    let (reference, water) = (line(&c), [water_along(&c, water_width)]);
    let s = settings(20.0, 200.0, 1.0);
    assert!(highest < s.length_search.max_m);
    let r: BeamResult = longest_for_beam(&reference, &water, &s, beam).unwrap();

    assert_eq!(r.beam_m, beam);
    assert!(
        !r.at_max,
        "the bend stops the search before its upper bound"
    );
    let longest = r.longest_m.expect("a length fits");
    assert_eq!(
        longest.fract(),
        0.0,
        "a length of the search: 20 m plus whole steps"
    );
    assert!(
        (lowest..=highest + 1.0).contains(&longest),
        "the longest length is {longest} m; the bend allows from {lowest} to {highest} m"
    );

    // What binds: the stretches of the next length, one step longer. They are in the bend.
    let (bend_from, bend_to) = (tail - 20.0, tail - 20.0 + rc * FRAC_PI_2);
    assert!(!r.binding.is_empty());
    for st in &r.binding {
        assert!(
            st.start_chainage_m >= bend_from && st.end_chainage_m <= bend_to,
            "a {} stretch from {} to {} m; the bend is from {bend_from} to {bend_to} m",
            st.kind.as_str(),
            st.start_chainage_m,
            st.end_chainage_m
        );
    }
    // The answer by its meaning, on the existing size sweep: the length fits, one step more does
    // not, and the stretches of that next length are what binds.
    let at = |loa_m: f64| {
        size_result(
            &reference,
            &water,
            &s,
            Size {
                loa_m,
                beam_m: beam,
            },
        )
        .unwrap()
    };
    assert!(at(longest).passes);
    let next = at(longest + 1.0);
    assert!(!next.passes);
    assert_eq!(r.binding, next.stretches);
}

#[test]
fn a_straight_river_fits_every_length_of_a_narrow_beam_and_no_length_of_a_wide_one() {
    let (river_width, narrow, wide) = (20.0, 4.0, 10.0);
    let s = settings(20.0, 200.0, 1.0);
    // The straight fairway of each beam, the same at both ends of the search.
    let fairway = |beam: f64| {
        let w = shape(s.length_search.min_m, beam).bottom_width_straight_m;
        assert_eq!(
            w,
            shape(s.length_search.max_m, beam).bottom_width_straight_m
        );
        w
    };
    assert!(fairway(narrow) < river_width && river_width < fairway(wide));

    let c = straight(2000.0);
    let (reference, water) = (line(&c), [water_along(&c, river_width)]);

    let r = longest_for_beam(&reference, &water, &s, narrow).unwrap();
    assert_eq!(r.longest_m, Some(200.0));
    assert!(
        r.at_max,
        "the upper bound of the search stops it, not the river"
    );
    assert!(r.binding.is_empty());
    for loa_m in [20.0, 57.0, 123.0, 200.0] {
        let size = Size {
            loa_m,
            beam_m: narrow,
        };
        assert!(size_result(&reference, &water, &s, size).unwrap().passes);
    }

    let r = longest_for_beam(&reference, &water, &s, wide).unwrap();
    assert_eq!(r.longest_m, None, "no length of the wide beam fits");
    assert!(!r.at_max);
    assert!(!r.binding.is_empty());
    assert!(r.binding.iter().all(|st| st.kind.as_str() == "width"));
}

#[test]
fn the_sweep_answers_for_each_beam_of_the_settings_in_their_order() {
    let c = straight(2000.0);
    let (reference, water) = (line(&c), [water_along(&c, 20.0)]);
    let s = Settings {
        sizes: vec![Size {
            loa_m: 10.0,
            beam_m: 2.0,
        }],
        beams_m: vec![10.0, 4.0],
        ..settings(20.0, 200.0, 1.0)
    };
    let e = envelope(&reference, &water, s.clone(), None).unwrap();
    assert_eq!(e.by_beam.len(), 2);
    for (got, beam) in e.by_beam.iter().zip([10.0, 4.0]) {
        assert_eq!(
            got,
            &longest_for_beam(&reference, &water, &s, beam).unwrap()
        );
    }
    // With no beam, the sweep is the fixed series alone.
    assert!(Settings::default().beams_m.is_empty());
    let e = envelope(&reference, &water, Settings::default(), None).unwrap();
    assert!(e.by_beam.is_empty());
}

#[test]
fn a_search_that_makes_no_sense_is_refused() {
    let c = straight(500.0);
    let (reference, water) = (line(&c), [water_along(&c, 100.0)]);
    for (min_m, max_m, step_m) in [
        (20.0, 200.0, 0.0),
        (20.0, 200.0, -1.0),
        (200.0, 20.0, 1.0),
        (0.0, 200.0, 1.0),
        (20.0, f64::NAN, 1.0),
    ] {
        let s = settings(min_m, max_m, step_m);
        assert!(
            longest_for_beam(&reference, &water, &s, 4.0).is_err(),
            "{min_m} to {max_m} by {step_m}"
        );
    }
    let s = settings(20.0, 200.0, 1.0);
    assert!(longest_for_beam(&reference, &water, &s, 0.0).is_err());
    // A hull is longer than it is wide: the shortest length must be more than the beam.
    assert!(longest_for_beam(&reference, &water, &s, 20.0).is_err());
}
