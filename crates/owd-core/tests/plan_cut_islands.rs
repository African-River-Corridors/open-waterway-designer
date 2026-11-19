//! The plan cut on a river polygon with islands (inner rings): an island is land, so the fairway
//! over it is cut and the fairway over open water is not (issue #510).
//!
//! The expected areas are closed-form: an island that lies inside the fairway cuts its own area.
//!
//! Label: property

use owd_core::geom::{Polygon, Pt};
use owd_core::plan_cut::{Fairway, plan_cut};
use owd_core::volumes::sections::HalfWidth;

const L: f64 = 1000.0;
/// The surface half-width: 20 + 3 x 3.5 = 30.5 m.
const FAIRWAY: Fairway<'static> = Fairway {
    bottom_half_width: HalfWidth::Fixed(20.0),
    wet_slope: 3.0,
    depth_m: 3.5,
    station_m: 25.0,
};
const MIN_PIECE: f64 = 50.0;

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
}

fn reversed(ring: Vec<Pt>) -> Vec<Pt> {
    ring.into_iter().rev().collect()
}

/// A river 200 m wide about the x axis, with the given islands.
fn river(exterior: Vec<Pt>, holes: Vec<Vec<Pt>>) -> Vec<Polygon> {
    vec![Polygon { exterior, holes }]
}

fn cut_area(river: &[Polygon]) -> (usize, f64) {
    let line = vec![[0.0, 0.0], [L, 0.0]];
    let cut = plan_cut(&line, &FAIRWAY, river, &line, MIN_PIECE).unwrap();
    (cut.pieces.len(), cut.area_m2())
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

/// One island across the fairway (100 m x 40 m, inside its 61 m width); two others far from it,
/// one clear of the fairway and one beside it.
fn islands() -> Vec<Vec<Pt>> {
    vec![
        rect(400.0, 500.0, -20.0, 20.0),
        rect(100.0, 150.0, 60.0, 90.0),
        rect(700.0, 760.0, -80.0, -45.0),
    ]
}

#[test]
fn an_island_across_the_fairway_is_the_only_cut() {
    let r = river(rect(-100.0, L + 100.0, -100.0, 100.0), islands());
    let (n, area) = cut_area(&r);
    assert_eq!(n, 1, "one cut piece: the island under the fairway");
    assert!(close(area, 100.0 * 40.0), "area {area}");
}

#[test]
fn the_same_river_without_islands_is_not_cut() {
    let r = river(rect(-100.0, L + 100.0, -100.0, 100.0), vec![]);
    assert_eq!(cut_area(&r), (0, 0.0));
}

#[test]
fn the_ring_orientation_changes_nothing() {
    let want = cut_area(&river(rect(-100.0, L + 100.0, -100.0, 100.0), islands()));
    let flipped_holes: Vec<Vec<Pt>> = islands().into_iter().map(reversed).collect();
    let a = cut_area(&river(
        reversed(rect(-100.0, L + 100.0, -100.0, 100.0)),
        islands(),
    ));
    let b = cut_area(&river(
        rect(-100.0, L + 100.0, -100.0, 100.0),
        flipped_holes.clone(),
    ));
    let c = cut_area(&river(
        reversed(rect(-100.0, L + 100.0, -100.0, 100.0)),
        flipped_holes,
    ));
    for got in [a, b, c] {
        assert_eq!(got.0, want.0);
        assert!(close(got.1, want.1), "{got:?} != {want:?}");
    }
}

#[test]
fn thirty_islands_cut_only_where_they_lie_under_the_fairway() {
    // Thirty 10 m x 10 m islands, a 30 m pitch: ten of them on the line (inside the fairway),
    // ten clear above it and ten clear below it.
    let mut holes = Vec::new();
    for i in 0..10 {
        let x = 50.0 + 90.0 * i as f64;
        holes.push(rect(x, x + 10.0, -5.0, 5.0));
        holes.push(rect(x, x + 10.0, 50.0, 60.0));
        holes.push(rect(x, x + 10.0, -60.0, -50.0));
    }
    let r = river(rect(-100.0, L + 100.0, -100.0, 100.0), holes);
    let line = vec![[0.0, 0.0], [L, 0.0]];
    let cut = plan_cut(&line, &FAIRWAY, &r, &line, 10.0).unwrap();
    // The islands at y 50..60 are outside the 30.5 m half-width: only the ten on the line cut.
    assert_eq!(cut.pieces.len(), 10, "pieces: {}", cut.pieces.len());
    assert!(close(cut.area_m2(), 10.0 * 100.0), "{}", cut.area_m2());
}

#[test]
fn islands_touching_the_fairway_edge_do_not_cut_it_beyond_their_overlap() {
    // An island straddling the fairway's left edge (y = 30.5): 10 m x 20 m, half of it inside.
    let r = river(
        rect(-100.0, L + 100.0, -100.0, 100.0),
        vec![rect(300.0, 310.0, 20.5, 40.5)],
    );
    let (n, area) = cut_area(&r);
    assert_eq!(n, 1);
    assert!(close(area, 10.0 * 10.0), "area {area}");
}

/// A regular polygon: an island of radius `r` about `(cx, cy)`, `n` corners.
fn island(cx: f64, cy: f64, r: f64, n: usize) -> Vec<Pt> {
    let mut ring: Vec<Pt> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [cx + r * libm::cos(a), cy + r * libm::sin(a)]
        })
        .collect();
    ring.push(ring[0]);
    ring
}

fn shifted(ring: &[Pt], dx: f64, dy: f64) -> Vec<Pt> {
    ring.iter().map(|p| [p[0] + dx, p[1] + dy]).collect()
}

#[test]
fn islands_in_projected_coordinates_cut_the_same_as_near_the_origin() {
    // Thirty round islands (a 16-gon of radius 12 m): ten on the line, ten above, ten below.
    let mut holes = Vec::new();
    for i in 0..10 {
        let x = 50.0 + 90.0 * i as f64;
        holes.push(island(x, 0.0, 12.0, 16));
        holes.push(island(x + 30.0, 70.0, 12.0, 16));
        holes.push(island(x + 30.0, -70.0, 12.0, 16));
    }
    let exterior = rect(-100.0, L + 100.0, -100.0, 100.0);
    let near = river(exterior.clone(), holes.clone());
    let (n0, a0) = cut_area(&near);
    assert_eq!(n0, 10);
    // A UTM-sized offset (easting 500 km, northing 600 km), the line moved with the river.
    let (dx, dy) = (500_000.0, 600_000.0);
    let far = river(
        shifted(&exterior, dx, dy),
        holes.iter().map(|h| shifted(h, dx, dy)).collect(),
    );
    let line = vec![[dx, dy], [dx + L, dy]];
    let cut = plan_cut(&line, &FAIRWAY, &far, &line, MIN_PIECE).unwrap();
    assert_eq!(cut.pieces.len(), n0, "pieces far from the origin");
    assert!(
        (cut.area_m2() - a0).abs() < 1.0,
        "{} != {a0}",
        cut.area_m2()
    );
}

#[test]
fn an_island_wider_than_the_fairway_cuts_the_whole_fairway_under_it_and_no_more() {
    // An island 200 m x 100 m over a 61 m wide fairway: the cut is the fairway's 200 m stretch.
    let r = river(
        rect(-100.0, L + 100.0, -200.0, 200.0),
        vec![rect(300.0, 500.0, -50.0, 50.0)],
    );
    let (n, area) = cut_area(&r);
    assert_eq!(n, 1);
    assert!(close(area, 200.0 * 61.0), "area {area}");
}

#[test]
fn islands_that_touch_the_bank_or_each_other_still_leave_the_open_water_uncut() {
    // Invalid as drawn (a hole touching the exterior, two holes sharing an edge): common in a
    // digitised river. The fairway over open water must not be cut.
    let holes = vec![
        rect(100.0, 200.0, 90.0, 100.0), // touches the top bank
        rect(300.0, 340.0, -10.0, 10.0),
        rect(340.0, 380.0, -10.0, 10.0), // shares an edge with the one before
        rect(600.0, 640.0, -40.0, 40.0),
        rect(620.0, 680.0, -10.0, 10.0), // overlaps the one before
    ];
    let r = river(rect(-100.0, L + 100.0, -100.0, 100.0), holes);
    let (n, area) = cut_area(&r);
    // Union of the islands inside the fairway, clipped to its width.
    // The fairway is 61 m wide (y within 30.5), so the 80 m tall island is clipped to 61 m.
    let want = 80.0 * 20.0 + (40.0 * 61.0 + 60.0 * 20.0 - 20.0 * 20.0);
    assert_eq!(
        n, 2,
        "pieces {n}: the touching pair is one piece, the overlapping pair another"
    );
    assert!(close(area, want), "area {area} != {want}");
}
