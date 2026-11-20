//! The plan cut on a river polygon whose rings are not closed (issue #510). A digitised river
//! often has an island ring whose last point is not its first. A polygon constructor that closes
//! such a ring (as shapely does) must give the same cut as the same polygon with closed rings;
//! before the fix an unclosed island ring made the whole fairway one cut piece.
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

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
}

fn unclosed(mut ring: Vec<Pt>) -> Vec<Pt> {
    ring.pop();
    ring
}

fn cut(exterior: Vec<Pt>, holes: Vec<Vec<Pt>>) -> (usize, f64) {
    let river = vec![Polygon { exterior, holes }];
    let line = vec![[0.0, 0.0], [L, 0.0]];
    let c = plan_cut(&line, &FAIRWAY, &river, &line, 50.0).unwrap();
    (c.pieces.len(), c.area_m2())
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

fn bank() -> Vec<Pt> {
    rect(-100.0, L + 100.0, -100.0, 100.0)
}

#[test]
fn an_unclosed_island_ring_cuts_the_same_as_the_closed_ring() {
    // One island across the fairway (100 m x 40 m) and one clear of it.
    let islands = vec![
        rect(400.0, 500.0, -20.0, 20.0),
        rect(100.0, 150.0, 60.0, 90.0),
    ];
    let want = cut(bank(), islands.clone());
    assert_eq!(want.0, 1);
    assert!(close(want.1, 100.0 * 40.0), "{want:?}");
    let got = cut(bank(), islands.into_iter().map(unclosed).collect());
    assert_eq!(got.0, want.0, "pieces");
    assert!(close(got.1, want.1), "{got:?} != {want:?}");
}

#[test]
fn an_unclosed_exterior_ring_cuts_the_same_as_the_closed_ring() {
    let want = cut(bank(), vec![rect(400.0, 500.0, -20.0, 20.0)]);
    let got = cut(unclosed(bank()), vec![rect(400.0, 500.0, -20.0, 20.0)]);
    assert_eq!(got.0, want.0);
    assert!(close(got.1, want.1), "{got:?} != {want:?}");
}

#[test]
fn unclosed_rings_in_either_orientation_cut_the_same() {
    let island = rect(400.0, 500.0, -20.0, 20.0);
    let want = cut(bank(), vec![island.clone()]);
    let mut flipped = unclosed(island);
    flipped.reverse();
    let got = cut(unclosed(bank()), vec![flipped]);
    assert_eq!(got.0, want.0);
    assert!(close(got.1, want.1), "{got:?} != {want:?}");
}
