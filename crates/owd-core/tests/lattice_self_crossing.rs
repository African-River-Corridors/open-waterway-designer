//! A pool lattice line that crosses itself is never spliced (issue #569).
//!
//! The case is synthetic: a sine-generated meander, heading = 110 deg x sin(2 pi s / wavelength).
//! The river runs backwards at each apex and its lobes nest, but the river line does not cross
//! itself. Water is a strip of 65 m round it at -8 m, ground is +2 m, on a 5 m grid; the river is
//! 8 km long, the wavelength 2,000 m, R_min 340 m. The lattice is coarse (pitch 40 m, cell 20 m,
//! corridor 200 m) so the tests run in seconds; the loop is the same at pitch 12 m and cell 5 m.
//!
//! Why the search loops: water below the grade costs nothing to cut, so length in the water is free
//! (`c_min` is 0 and A* runs as Dijkstra), and a search state is only a position, heading and turn,
//! with no memory of the path. From the start pose (heading east) to the pose at 3,500 m (heading
//! back, -110 deg) the least-cost lattice line runs 4.8 km through the water of the nested lobes and
//! crosses itself twice. It is the least-cost line under the search's own cost, not a heuristic
//! artefact: with no heuristic the search returns the same line.
//!
//! Label: property

use owd_core::alignment::loops::{first_self_crossing_m, is_self_intersecting};
use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, SectionSpec, optimise_pool,
    optimise_pools,
};

const R_MIN: f64 = 340.0;
const LEN: f64 = 8000.0;
const DEPTH: f64 = 3.5;
const OPTS: PoolOptions = PoolOptions {
    corridor_half_m: 200.0,
    cell_m: 20.0,
};

/// The meander's centreline, in 5 m steps.
fn centreline(lam: f64, len: f64) -> Vec<Pt> {
    let th0 = 110f64.to_radians();
    let ds = 5.0;
    let n = (len / ds) as usize;
    let mut out = vec![[0.0, 0.0]];
    for i in 0..n {
        let th = th0 * libm::sin(2.0 * core::f64::consts::PI * (i as f64 * ds) / lam);
        let [x, y] = out[out.len() - 1];
        out.push([x + ds * libm::cos(th), y + ds * libm::sin(th)]);
    }
    out
}

/// The water strip on a 5 m grid: -8 m on a cell whose centre is within `half` of the centreline,
/// +2 m elsewhere; each point takes its nearest cell.
fn terrain(cl: &[Pt], half: f64) -> impl Fn(&[f64], &[f64]) -> Vec<f64> + use<> {
    const CELL: f64 = 5.0;
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in cl {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let pad = 1200.0;
    let (x0, y0) = (x0 - pad, y0 - pad);
    let cols = ((x1 + pad - x0) / CELL).ceil() as usize;
    let rows = ((y1 + pad - y0) / CELL).ceil() as usize;
    let mut wet = vec![false; rows * cols];
    for s in cl.windows(2) {
        let ([ax, ay], [bx, by]) = (s[0], s[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let c0 = ((ax.min(bx) - half - x0) / CELL).floor().max(0.0) as usize;
        let c1 = (((ax.max(bx) + half - x0) / CELL).ceil() as usize).min(cols - 1);
        let r0 = ((ay.min(by) - half - y0) / CELL).floor().max(0.0) as usize;
        let r1 = (((ay.max(by) + half - y0) / CELL).ceil() as usize).min(rows - 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (x, y) = (x0 + (c as f64 + 0.5) * CELL, y0 + (r as f64 + 0.5) * CELL);
                let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let (px, py) = (ax + t * dx - x, ay + t * dy - y);
                if px * px + py * py <= half * half {
                    wet[r * cols + c] = true;
                }
            }
        }
    }
    move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let c = ((x - x0) / CELL).floor();
                let r = ((y - y0) / CELL).floor();
                if c < 0.0 || r < 0.0 || c >= cols as f64 || r >= rows as f64 {
                    return f64::NAN;
                }
                if wet[r as usize * cols + c as usize] {
                    -8.0
                } else {
                    2.0
                }
            })
            .collect()
    }
}

fn river(cl: &[Pt]) -> Vec<Polygon> {
    match buffer_line(cl, 60.0, 16, Cap::Round).unwrap() {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(p) => p,
        other => panic!("the river came out as {other:?}"),
    }
}

fn spec() -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 250.0;
    spec.rake_step_m = 4.0;
    spec.ds_step_m = 8.0;
    spec
}

fn cfg() -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.pitch_m = 40.0;
    c.validated().unwrap()
}

/// The meander, its terrain and river, and two pools at levels 0 and 0.5 m split at `weir_m`.
fn run(weir_m: f64) -> (Vec<Pt>, PoolsResult) {
    let cl = centreline(2000.0, LEN);
    let total = linear::length(&cl);
    let t = terrain(&cl, 65.0);
    let r = optimise_pools(
        &cl,
        &t,
        &river(&cl),
        &[(0.0, weir_m, 0.0), (weir_m, total, 0.5)],
        DEPTH,
        &spec(),
        &cfg(),
        None,
        OPTS,
        true,
    )
    .unwrap();
    for p in &r.pools {
        println!(
            "pool {:.0}..{:.0}: closed {}, used {}, cost {:.0}, first own crossing {:?}, end moved from {:?}, {:?}",
            p.start_m,
            p.end_m,
            p.result.goal_closed,
            p.used,
            p.result.cost,
            p.result.line.as_deref().map(first_self_crossing_m),
            p.end_moved_from_m,
            p.kept_baseline_because,
        );
    }
    println!("joined line crosses itself at {:?}", r.self_crossing_m);
    (cl, r)
}

/// The search itself: one pool from 0 to 3,500 m returns a line that closes on its goal, holds
/// R_min and crosses itself (twice): 4.8 km of line for a pool whose ends are about 1 km apart.
#[test]
fn the_search_returns_a_looped_line_for_the_first_pool() {
    let cl = centreline(2000.0, LEN);
    let t = terrain(&cl, 65.0);
    let p = optimise_pool(
        &cl,
        &t,
        &river(&cl),
        0.0,
        3500.0,
        0.0,
        DEPTH,
        &spec(),
        &cfg(),
        None,
        OPTS,
    )
    .unwrap();
    let line = p.result.line.unwrap();
    let first = first_self_crossing_m(&line);
    println!(
        "cost {:.0}, length {:.0} m, first crossing {first:?}",
        p.result.cost,
        linear::length(&line)
    );
    assert!(p.result.goal_closed && p.result.radius_ok);
    assert!(first.is_some(), "the line does not cross itself");
    assert!(linear::length(&line) > 4500.0);
}

/// The fault: with the weir at 3,720 m the first pool's own line crosses itself twice (the search
/// costs it 312,794), and it used to be spliced. Now the pool is searched again with its end moved
/// one step of R_min / 4 downstream, to 3,805 m, where the line is simple (99,567), and the joined
/// line is simple.
#[test]
fn a_pool_whose_own_line_crosses_itself_is_never_spliced() {
    let (_, r) = run(3720.0);
    for p in r.pools.iter().filter(|p| p.used) {
        let line = p.result.line.as_deref().unwrap();
        assert!(
            !is_self_intersecting(line),
            "the pool {:.0}..{:.0} is used with a line that crosses itself",
            p.start_m,
            p.end_m
        );
    }
    assert_eq!(r.self_crossing_m, None);
    assert!(!is_self_intersecting(&r.line));
    let (p0, p1) = (&r.pools[0], &r.pools[1]);
    assert_eq!(p0.end_moved_from_m, Some(3720.0));
    assert!(
        (p0.end_m - (3720.0 + R_MIN / 4.0)).abs() < 1e-9,
        "{}",
        p0.end_m
    );
    assert!(p0.used && p0.result.goal_closed);
    assert_eq!(p1.start_moved_from_m, Some(3720.0));
    assert_eq!(p1.start_m, p0.end_m);
    assert!(r.radius_ok, "{}", r.measured_min_radius_m);
}

/// Pools whose lines are simple give the line they gave before: each pool's own line, spliced end
/// to end, with no end moved and no pool dropped. The weir sits at 2,500 m.
#[test]
fn pools_with_a_simple_line_give_the_same_line() {
    let (cl, r) = run(2500.0);
    assert!(r.pools.iter().all(|p| p.used && p.result.goal_closed));
    assert_eq!(r.moved_boundaries().count(), 0);
    assert_eq!(r.self_crossing_m, None);
    let total = linear::length(&cl);
    let t = terrain(&cl, 65.0);
    let mut want: Vec<Pt> = Vec::new();
    for (a, b, wl) in [(0.0, 2500.0, 0.0), (2500.0, total, 0.5)] {
        let p = optimise_pool(
            &cl,
            &t,
            &river(&cl),
            a,
            b,
            wl,
            DEPTH,
            &spec(),
            &cfg(),
            None,
            OPTS,
        )
        .unwrap();
        for xy in p.result.line.unwrap() {
            if want
                .last()
                .is_none_or(|l| libm::hypot(xy[0] - l[0], xy[1] - l[1]) > 1e-9)
            {
                want.push(xy);
            }
        }
    }
    assert_eq!(r.line, want);
}
