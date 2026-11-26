//! Buildings and the channel (BLD-1, part 4): in the cut, near it, and on land a cut-through
//! severs, on a synthetic meander with a cut-through. A report, not a cost. No real data.
//!
//! Label: property

use owd_core::geom::{Polygon, Pt};
use owd_format::design::channel::{Channel, ChannelStation};
use owd_format::design::feature_place::{CutIndex, CutRelation, cut_relation, severed};

/// A river 40 m wide that runs east along y = 0 to x = 400, loops north round a meander (up to
/// y = 600, across to x = 800), and comes back down to y = 0 and on east: its polygon. The design
/// line runs straight along y = 0, so from x = 420 to x = 780 it leaves the river: a cut-through
/// across the meander's neck.
fn meander() -> Vec<Polygon> {
    let w = 20.0;
    let ring = vec![
        [-100.0, -w],
        [400.0 + w, -w],
        [400.0 + w, 600.0 - w],
        [800.0 - w, 600.0 - w],
        [800.0 - w, -w],
        [1300.0, -w],
        [1300.0, w],
        [800.0 + w, w],
        [800.0 + w, 600.0 + w],
        [400.0 - w, 600.0 + w],
        [400.0 - w, w],
        [-100.0, w],
        [-100.0, -w],
    ];
    vec![Polygon {
        exterior: ring,
        holes: vec![],
    }]
}

fn line() -> Vec<Pt> {
    vec![[0.0, 0.0], [1200.0, 0.0]]
}

#[test]
fn land_inside_the_bypassed_loop_is_severed_and_land_outside_is_not() {
    let river = meander();
    let pts = [
        [600.0, 300.0],  // inside the loop: severed by the cut along y = 0
        [600.0, 800.0],  // north of the loop: mainland
        [600.0, -300.0], // south of the line: mainland
        [200.0, 200.0],  // west of the loop: mainland
    ];
    let cut = severed(&line(), &river, 15.0, &pts);
    assert_eq!(cut, vec![true, false, false, false]);
    // Without a cut-through (the line in the river all along) nothing is severed.
    let straight = vec![Polygon {
        exterior: vec![
            [-100.0, -20.0],
            [1300.0, -20.0],
            [1300.0, 20.0],
            [-100.0, 20.0],
            [-100.0, -20.0],
        ],
        holes: vec![],
    }];
    assert_eq!(severed(&line(), &straight, 15.0, &pts), vec![false; 4]);
}

fn station(x: f64) -> ChannelStation {
    ChannelStation {
        chainage_m: x,
        x,
        y: 0.0,
        heading_rad: 0.0,
        half_width_m: 15.0,
        water_level_m: 10.0,
        invert_m: 7.0,
        bench_left: true,
        bench_right: true,
        top_of_cut_left_m: Some(-30.0),
        top_of_cut_right_m: Some(30.0),
        daylight_left_m: Some(-32.0),
        daylight_right_m: Some(32.0),
        dredge_area_m2: 0.0,
        excav_area_m2: 0.0,
        max_face_h_m: 3.0,
        on_dem: true,
    }
}

#[test]
fn a_building_is_taken_inside_the_daylight_lines_near_within_the_distance_and_clear_beyond() {
    let channel = Channel {
        option_id: "x".into(),
        stations: (0..=48).map(|k| station(25.0 * f64::from(k))).collect(),
        wet_slope: 3.0,
        dx_m: 1.0,
        maxw_m: 200.0,
        from_volumes: true,
        has_ground: true,
    };
    let index = CutIndex::new(&channel);
    let rel = |p: Pt, area: f64| {
        index
            .locate(p, 250.0)
            .and_then(|c| cut_relation(&c, area, 50.0))
    };
    // Inside the daylight line on the left (offset +20: at_offset's positive side).
    assert_eq!(rel([500.0, 20.0], 100.0), Some(CutRelation::Taken));
    // 5 m outside the daylight line, a building of 314 m2 (radius 10 m) reaches in.
    assert_eq!(rel([500.0, 37.0], 314.159), Some(CutRelation::Taken));
    // The same point as a building of 10 m2 is near: 5 m out, less its 1.8 m radius.
    assert_eq!(rel([500.0, 37.0], 10.0), Some(CutRelation::Near));
    // 80 m out on the other side: near is 50 m, so clear.
    assert_eq!(rel([500.0, -112.0], 10.0), None);
    let place = index.locate([500.0, -60.0], 250.0).unwrap();
    assert!(
        (place.outside_m - 28.0).abs() < 1e-9 && place.daylight,
        "{place:?}"
    );
    assert_eq!(place.chainage_m, 500.0);
}
