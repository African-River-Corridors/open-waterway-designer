//! The cut of each cross-section by kind: in the river, widening of the river, cut-through.
//!
//! Every case is closed form. The terrain is flat at the water level, 10 m. The design depth is
//! 2 m, the bottom half-width 20 m and the wet slope 3, so the channel under water reaches 26 m
//! each side of the line. Samples are 1 m apart, at whole metres. The cut at a sample is 2 m on the
//! bottom (41 samples) and 2 - k/3 m at k metres up a slope (k = 1..5, both sides): 82 + 10 = 92 m2
//! per section. The line is 1000 m long with a station every 25 m: 41 sections.
//!
//! Label: property

use owd_core::geom::{Polygon, Pt};
use owd_core::volumes::sections::{
    DEFAULT_CHUNK, HalfWidth, Level, SectionVolumes, section_volumes,
};

const SECTION_M2: f64 = 92.0;
const STATION_M: f64 = 25.0;
const STATIONS: f64 = 41.0;

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![10.0; xs.len()]
}

/// A piece of river: a box from `x0` to `x1` along the line and from `y0` to `y1` across it.
fn river(x0: f64, x1: f64, y0: f64, y1: f64) -> Polygon {
    Polygon {
        exterior: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]],
        holes: vec![],
    }
}

fn walk(river: &[Polygon]) -> SectionVolumes {
    let line: Vec<Pt> = vec![[0.0, 0.0], [1000.0, 0.0]];
    section_volumes(
        &line,
        &flat,
        river,
        &Level::Fixed(10.0),
        2.0,
        &HalfWidth::Fixed(20.0),
        "a",
        3.0,
        STATION_M,
        100.0,
        1.0,
        None,
        None,
        DEFAULT_CHUNK,
    )
    .unwrap()
}

fn close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

/// The three kinds add up to the total the walk already gives, and the walk's own numbers hold.
fn adds_up(v: &SectionVolumes) {
    close(
        v.in_river_m3() + v.widening_m3() + v.cut_through_m3(),
        v.total_m3(),
        "the three kinds add up to the total",
    );
    close(v.in_river_m3(), v.dredge_m3(), "in the river is the dredge");
    close(
        v.widening_m3() + v.cut_through_m3(),
        v.excav_m3(),
        "widening and cut-through are the dry cut",
    );
    close(
        v.total_m3(),
        SECTION_M2 * STATION_M * STATIONS,
        "the total is the closed form",
    );
    for s in &v.stations {
        close(
            s.in_river_area_m2() + s.widening_area_m2() + s.cut_through_area_m2(),
            s.dredge_area_m2 + s.excav_area_m2,
            "the kinds of one section add up",
        );
    }
}

#[test]
fn a_line_inside_a_wide_river_is_all_in_the_river() {
    let v = walk(&[river(-100.0, 1100.0, -150.0, 150.0)]);
    adds_up(&v);
    close(v.in_river_m3(), v.total_m3(), "in the river");
    assert_eq!(v.widening_m3(), 0.0);
    assert_eq!(v.cut_through_m3(), 0.0);
}

#[test]
fn a_line_across_dry_land_is_all_cut_through() {
    // The river is 5 km away: no section reaches it.
    let v = walk(&[river(-100.0, 1100.0, 5000.0, 5100.0)]);
    adds_up(&v);
    close(v.cut_through_m3(), v.total_m3(), "cut-through");
    assert_eq!(v.in_river_m3(), 0.0);
    assert_eq!(v.widening_m3(), 0.0);
}

#[test]
fn a_river_narrower_than_the_channel_splits_in_the_known_ratio() {
    // The river is 21 m wide on the line: 21 samples of 2 m are in it. The rest is widening.
    let v = walk(&[river(-100.0, 1100.0, -10.5, 10.5)]);
    adds_up(&v);
    close(v.in_river_m3(), 42.0 * STATION_M * STATIONS, "in the river");
    close(v.widening_m3(), 50.0 * STATION_M * STATIONS, "widening");
    assert_eq!(v.cut_through_m3(), 0.0);
    for s in &v.stations {
        close(s.in_river_area_m2(), 42.0, "in the river, one section");
        close(s.widening_area_m2(), 50.0, "widening, one section");
        assert_eq!(s.cut_through_area_m2(), 0.0);
    }
}

#[test]
fn a_river_beside_the_channel_but_outside_it_does_not_make_the_cut_widening() {
    // The river runs along the line from 40.5 m to 90.5 m to one side. The profile reaches it
    // (it is 100 m long each side), but the channel under water stops at 26 m.
    let v = walk(&[river(-100.0, 1100.0, 40.5, 90.5)]);
    adds_up(&v);
    close(v.cut_through_m3(), v.total_m3(), "cut-through");
    assert_eq!(v.in_river_m3(), 0.0);
    assert_eq!(v.widening_m3(), 0.0);
}

#[test]
fn a_river_that_meets_only_the_slope_makes_the_cut_widening() {
    // The river starts 22.5 m from the line, on the slope. The samples at 23, 24 and 25 m are in
    // it and are cut: 1 + 2/3 + 1/3 = 2 m2. The other 90 m2 widen that river.
    let v = walk(&[river(-100.0, 1100.0, 22.5, 90.5)]);
    adds_up(&v);
    close(v.in_river_m3(), 2.0 * STATION_M * STATIONS, "in the river");
    close(v.widening_m3(), 90.0 * STATION_M * STATIONS, "widening");
    assert_eq!(v.cut_through_m3(), 0.0);
}

#[test]
fn one_line_through_all_three_kinds_adds_up() {
    // Stations 0 to 300 m (13) lie in a wide river, 325 to 700 m (16) in a river 21 m wide, and
    // 725 to 1000 m (12) on dry land.
    let v = walk(&[
        river(-100.0, 312.5, -150.0, 150.0),
        river(312.5, 712.5, -10.5, 10.5),
    ]);
    adds_up(&v);
    close(
        v.in_river_m3(),
        (13.0 * 92.0 + 16.0 * 42.0) * STATION_M,
        "in the river",
    );
    close(v.widening_m3(), 16.0 * 50.0 * STATION_M, "widening");
    close(v.cut_through_m3(), 12.0 * 92.0 * STATION_M, "cut-through");
}
