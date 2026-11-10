//! The plan cut in the core: the fairway along a line, at the water surface, less the river.
//!
//! Every case is closed-form geometry. The fairway width at the surface, W, is worked out here from
//! the stated rule: W = 2 x (bottom half-width + side slope x design depth). It is never read back
//! from the function under test.
//!
//! Label: property

use owd_core::geom::overlay::polygon_area;
use owd_core::geom::{Polygon, Pt};
use owd_core::plan_cut::{Fairway, plan_cut};
use owd_core::volumes::sections::HalfWidth;

const L: f64 = 1000.0;
const HB: f64 = 20.0;
const SLOPE: f64 = 3.0;
const DEPTH: f64 = 3.5;
const STATION: f64 = 25.0;
/// The surface half-width: 20 + 3 x 3.5 = 30.5 m. So W = 61 m.
const HALF_W: f64 = HB + SLOPE * DEPTH;
const MIN_PIECE: f64 = 50.0;

fn fairway(hb: HalfWidth<'_>) -> Fairway<'_> {
    Fairway {
        bottom_half_width: hb,
        wet_slope: SLOPE,
        depth_m: DEPTH,
        station_m: STATION,
    }
}

/// A line along +x at height `y`, from x = 0 to x = L. Left of it is +y.
fn line(y: f64) -> Vec<Pt> {
    vec![[0.0, y], [L, y]]
}

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
}

/// A straight river about the x axis, `w` wide, longer than the line at both ends.
fn river(w: f64, holes: Vec<Vec<Pt>>) -> Vec<Polygon> {
    vec![Polygon {
        exterior: rect(-100.0, L + 100.0, -w / 2.0, w / 2.0),
        holes,
    }]
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

#[test]
fn a_river_narrower_than_the_fairway_is_cut_by_the_difference_in_width_half_on_each_bank() {
    let w = 30.0;
    let cut = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::Fixed(HB)),
        &river(w, vec![]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    // The fairway ends square at the two ends of the line, so the length is L exactly.
    let want = (2.0 * HALF_W - w) * L;
    assert!(close(cut.area_m2(), want), "{} != {want}", cut.area_m2());
    assert!(close(cut.left_m2(), want / 2.0), "left {}", cut.left_m2());
    assert!(
        close(cut.right_m2(), want / 2.0),
        "right {}",
        cut.right_m2()
    );
    assert_eq!(cut.pieces.len(), 2);
    assert!(close(cut.largest_piece_m2(), want / 2.0));
    assert_eq!(cut.dropped_pieces, 0);
    // Each piece is a polygon for export, and its area is the area reported.
    for p in &cut.pieces {
        assert!(close(polygon_area(&p.polygon), p.area_m2));
    }
}

#[test]
fn a_line_off_the_river_centreline_cuts_more_on_the_bank_it_is_near() {
    let (w, d) = (30.0, 5.0);
    // The design line is 5 m to the left of the river centreline.
    let cut = plan_cut(
        &line(d),
        &fairway(HalfWidth::Fixed(HB)),
        &river(w, vec![]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert!(close(cut.left_m2(), (HALF_W + d - w / 2.0) * L));
    assert!(close(cut.right_m2(), (HALF_W - d - w / 2.0) * L));
    assert!(close(cut.area_m2(), cut.left_m2() + cut.right_m2()));
}

#[test]
fn the_bottom_half_width_of_each_station_is_used() {
    let w = 30.0;
    // 41 stations at 25 m. The bottom half-width grows evenly from 20 m to 30 m, so the fairway
    // is a trapezoid: its mean surface width is 2 x (25 + 10.5) = 71 m.
    let hb: Vec<f64> = (0..=40).map(|i| 20.0 + 10.0 * i as f64 / 40.0).collect();
    let want = (71.0 - w) * L;
    let per_station = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::PerStation(hb.clone())),
        &river(w, vec![]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert!(
        close(per_station.area_m2(), want),
        "{}",
        per_station.area_m2()
    );
    // The form the volumes stage uses: a function of the station points.
    let from_stations =
        |xy: &[Pt]| -> Vec<f64> { xy.iter().map(|p| 20.0 + p[0] / 100.0).collect() };
    let cut = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::FromStations(&from_stations)),
        &river(w, vec![]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert!(close(cut.area_m2(), want), "{}", cut.area_m2());
}

#[test]
fn a_river_wider_than_the_fairway_everywhere_has_no_cut() {
    let cut = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::Fixed(HB)),
        &river(200.0, vec![]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert_eq!(cut.pieces.len(), 0);
    assert_eq!(cut.area_m2(), 0.0);
    assert_eq!(cut.left_m2(), 0.0);
    assert_eq!(cut.right_m2(), 0.0);
    assert_eq!(cut.largest_piece_m2(), 0.0);
    assert_eq!(cut.dropped_pieces, 0);
}

#[test]
fn an_island_inside_the_fairway_is_cut() {
    // A 60 m by 20 m island (a hole of the river polygon), left of the centreline, inside the
    // fairway: 5 m to 25 m from the line, and the fairway reaches 30.5 m.
    let island = rect(400.0, 460.0, 5.0, 25.0);
    let cut = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::Fixed(HB)),
        &river(200.0, vec![island]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert_eq!(cut.pieces.len(), 1);
    assert!(close(cut.area_m2(), 60.0 * 20.0), "{}", cut.area_m2());
    assert!(close(cut.left_m2(), 60.0 * 20.0));
    assert_eq!(cut.right_m2(), 0.0);

    // An island that the fairway edge crosses: only the part inside the fairway is cut.
    // It runs from 20 m to 50 m right of the line; the fairway reaches 30.5 m.
    let island = rect(400.0, 460.0, -50.0, -20.0);
    let cut = plan_cut(
        &line(0.0),
        &fairway(HalfWidth::Fixed(HB)),
        &river(200.0, vec![island]),
        &line(0.0),
        MIN_PIECE,
    )
    .unwrap();
    assert_eq!(cut.pieces.len(), 1);
    assert!(close(cut.right_m2(), 60.0 * (HALF_W - 20.0)));
    assert_eq!(cut.left_m2(), 0.0);
}

#[test]
fn a_piece_smaller_than_the_smallest_area_is_not_a_cut() {
    // A 6 m by 6 m island: 36 m2.
    let island = rect(400.0, 406.0, 5.0, 11.0);
    let run = |min_piece_m2: f64| {
        plan_cut(
            &line(0.0),
            &fairway(HalfWidth::Fixed(HB)),
            &river(200.0, vec![island.clone()]),
            &line(0.0),
            min_piece_m2,
        )
        .unwrap()
    };
    let dropped = run(50.0);
    assert_eq!(dropped.pieces.len(), 0);
    assert_eq!(dropped.area_m2(), 0.0);
    assert_eq!(dropped.dropped_pieces, 1);
    assert!(close(dropped.dropped_m2, 36.0), "{}", dropped.dropped_m2);

    let kept = run(10.0);
    assert_eq!(kept.pieces.len(), 1);
    assert!(close(kept.area_m2(), 36.0));
    assert_eq!(kept.dropped_pieces, 0);
}

#[test]
fn the_fairway_follows_a_bend() {
    // A quarter circle of radius 500 m, drawn with a point every half degree. The river is far
    // away, so the cut is the whole fairway: W x the length of the line, less the small loss of
    // straight chords between stations 25 m apart (under 0.1 %) and of the two end stations.
    let r = 500.0;
    let arc: Vec<Pt> = (0..=180)
        .map(|i| {
            let a = (i as f64 * 0.5).to_radians();
            [r * libm::sin(a), r - r * libm::cos(a)]
        })
        .collect();
    let far = vec![Polygon {
        exterior: rect(-5000.0, -4000.0, -5000.0, -4000.0),
        holes: vec![],
    }];
    let cut = plan_cut(&arc, &fairway(HalfWidth::Fixed(HB)), &far, &arc, MIN_PIECE).unwrap();
    let length = owd_core::geom::linear::length(&arc);
    let want = 2.0 * HALF_W * length;
    assert_eq!(cut.pieces.len(), 1);
    assert!(
        (cut.area_m2() - want).abs() < 0.005 * want,
        "{} against {want}",
        cut.area_m2()
    );
}
