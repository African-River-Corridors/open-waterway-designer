//! The pool search pays for length: `SectionSpec::length_weight`, a cost per metre of design line.
//!
//! Water deeper than the channel bed costs nothing to cut, so without a length cost extra length in
//! deep water is free, and on a meander whose lobes nest the cheapest lattice line can be a loop
//! (issue #569, where only a guard keeps it out). A small cost per metre makes the search's own line
//! simple. A larger one trades cut for length: the line shortens and cuts more dry ground, up to
//! cutting the neck of a meander.
//!
//! The cases are synthetic: the sine-generated meander of `lattice_self_crossing.rs`, heading =
//! `th0` x sin(2 pi s / wavelength), water a strip of 65 m round it at -8 m, ground +2 m, on a 5 m
//! grid; R_min 340 m; the coarse lattice of that test (pitch 40 m, cell 20 m, corridor 200 m) so the
//! tests run in seconds. With the water level at 0 and a depth of 3.5 m, the bed is at -3.5 m: the
//! water costs nothing to pass through and the dry ground costs 5.5 m of cut plus its side slopes.
//!
//! Label: property

use owd_core::alignment::loops::{first_self_crossing_m, is_self_intersecting};
use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolAlignment, PoolOptions, PoolsResult, SectionSpec,
    optimise_pool, optimise_pools,
};

const R_MIN: f64 = 340.0;
const LEN: f64 = 8000.0;
const DEPTH: f64 = 3.5;
const OPTS: PoolOptions = PoolOptions {
    corridor_half_m: 200.0,
    cell_m: 20.0,
};

/// The smallest length weight, in steps of 0.1, at which the first pool of the #569 meander
/// (0 to 3,720 m) closes with a simple line of its own: found by `smallest_weight_scan` below.
///
/// At 0 the search's line is a loop 5,060 m long that cuts 312,794 m3; the simple line it finds at
/// 0.9 is 2,450 m long and cuts 314,988 m3. The break-even is (314,988 - 312,794) / (5,060 - 2,450)
/// = 0.84 per metre, so 0.9 is the first step of 0.1 above it.
///
/// A larger weight does not always keep the loop out on this coarse lattice: at 7 to 11, 100 and
/// 150 the search returns a looped line again, with or without the heuristic, although a simple line
/// would cost less. A state is a snapped position and heading, so states reached by different
/// paths merge, and the order in which they are reached, which the weight changes, decides which
/// lines remain. The guard of #569 still catches those; the weight is not a replacement for it.
const SMALLEST_LOOP_FREE_WEIGHT: f64 = 0.9;

/// A sine meander of `th0` degrees, in 5 m steps.
fn centreline(th0_deg: f64, lam: f64, len: f64) -> Vec<Pt> {
    let th0 = th0_deg.to_radians();
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

fn spec(length_weight: f64) -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 250.0;
    spec.rake_step_m = 4.0;
    spec.ds_step_m = 8.0;
    spec.length_weight = length_weight;
    spec
}

fn cfg() -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.pitch_m = 40.0;
    c.validated().unwrap()
}

/// One pool of the meander from `a` to `b` at water level 0.
fn pool(cl: &[Pt], a: f64, b: f64, w: f64, cfg: &LatticeConfig) -> PoolAlignment {
    let t = terrain(cl, 65.0);
    optimise_pool(
        cl,
        &t,
        &river(cl),
        a,
        b,
        0.0,
        DEPTH,
        &spec(w),
        cfg,
        None,
        OPTS,
    )
    .unwrap()
}

/// The #569 meander (110 deg, wavelength 2,000 m) with its weir at `weir_m`.
fn run_569(weir_m: f64, w: f64) -> PoolsResult {
    let cl = centreline(110.0, 2000.0, LEN);
    let total = linear::length(&cl);
    let t = terrain(&cl, 65.0);
    optimise_pools(
        &cl,
        &t,
        &river(&cl),
        &[(0.0, weir_m, 0.0), (weir_m, total, 0.5)],
        DEPTH,
        &spec(w),
        &cfg(),
        None,
        OPTS,
        true,
    )
    .unwrap()
}

/// The line's length as the search built it: the sum of its primitives' and closing pieces' lengths.
fn line_length(p: &PoolAlignment) -> f64 {
    p.result.prim_seq.iter().map(|q| q.2).sum()
}

/// The search's cut (weights 1 and 1, so m3): its cost less the length term.
fn cut(p: &PoolAlignment, w: f64) -> f64 {
    p.result.cost - w * line_length(p)
}

/// (a) A positive weight makes the search's OWN line simple on the #569 meander, with no help from
/// the guard: the first pool closes on its own end (it was not searched again with its end moved),
/// is used, keeps no baseline for a loop, and its line does not cross itself. Just below the
/// smallest such weight (in steps of 0.1) the first search still loops.
#[test]
fn a_length_weight_makes_the_searchs_own_line_simple() {
    let w = SMALLEST_LOOP_FREE_WEIGHT;
    assert!(w > 0.0);
    let r = run_569(3720.0, w);
    let p0 = &r.pools[0];
    println!(
        "weight {w}: pool 0 cost {:.0}, length {:.0} m, closed {}, used {}, moved {:?}, {:?}",
        p0.result.cost,
        line_length(p0),
        p0.result.goal_closed,
        p0.used,
        p0.end_moved_from_m,
        p0.kept_baseline_because
    );
    assert_eq!(p0.end_moved_from_m, None, "the pool was searched again");
    assert_eq!(r.pools[1].start_moved_from_m, None);
    assert!(p0.used && p0.result.goal_closed && p0.result.radius_ok);
    assert_eq!(p0.kept_baseline_because, None);
    assert_eq!(r.looped_pools().count(), 0);
    assert!(!is_self_intersecting(p0.result.line.as_deref().unwrap()));
    assert_eq!(r.self_crossing_m, None);

    // Just below it, the first pool's own search still loops.
    let cl = centreline(110.0, 2000.0, LEN);
    let below = pool(&cl, 0.0, 3720.0, w - 0.1, &cfg());
    assert!(
        first_self_crossing_m(below.result.line.as_deref().unwrap()).is_some(),
        "the search at {} does not loop",
        w - 0.1
    );
}

/// (b) At weight 0 the search is the one without it: the cost of the #569 test (312,794 for the
/// looped line of the first pool to 3,720 m), and the same pool, bit for bit, as a spec that never
/// sets the weight (its default is 0).
#[test]
fn at_weight_zero_the_search_is_unchanged() {
    let cl = centreline(110.0, 2000.0, LEN);
    let zero = pool(&cl, 0.0, 3720.0, 0.0, &cfg());
    assert_eq!(format!("{:.0}", zero.result.cost), "312794");
    assert_eq!(
        SectionSpec::new(HalfWidthSpec::Fixed(20.0)).length_weight,
        0.0
    );
    let mut plain = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    plain.side_slope_h_per_v = 3.0;
    let t = terrain(&cl, 65.0);
    let unset = optimise_pool(
        &cl,
        &t,
        &river(&cl),
        0.0,
        3720.0,
        0.0,
        DEPTH,
        &plain,
        &cfg(),
        None,
        OPTS,
    )
    .unwrap();
    assert_eq!(unset, zero);
}

/// The trade-off case: a meander of 90 deg (the river runs north and south at its apexes),
/// wavelength 2,000 m, one pool over a full wavelength, whose ends are 944 m apart. Between its two
/// bends is a neck of dry land. R_min is 150 m here, so the line can follow the river's bends
/// (their tightest radius is 203 m).
fn neck_river() -> Vec<Pt> {
    centreline(90.0, 2000.0, 3000.0)
}

fn neck_cfg() -> LatticeConfig {
    let mut c = LatticeConfig::new(150.0);
    c.pitch_m = 40.0;
    c.validated().unwrap()
}

fn neck_pool(w: f64) -> PoolAlignment {
    pool(&neck_river(), 0.0, 2000.0, w, &neck_cfg())
}

/// The weights of the trade-off table.
const TABLE_WEIGHTS: [f64; 5] = [0.0, 100.0, 200.0, 500.0, 3000.0];

/// (c) Raising the weight shortens the line and raises the cut, monotonically, and at a large weight
/// the line cuts the neck (it runs nearly straight from end to end). The table is printed.
#[test]
fn raising_the_weight_trades_cut_for_length() {
    let chord = {
        let cl = neck_river();
        let (a, b) = (
            linear::interpolate(&cl, 0.0).unwrap(),
            linear::interpolate(&cl, 2000.0).unwrap(),
        );
        libm::hypot(b[0] - a[0], b[1] - a[1])
    };
    println!("river 2000 m, ends {chord:.0} m apart");
    println!("| weight | line length (m) | cut (m3) |");
    println!("|---|---|---|");
    let mut rows = Vec::new();
    for w in TABLE_WEIGHTS {
        let p = neck_pool(w);
        assert!(p.result.converged && p.result.goal_closed, "weight {w}");
        let (l, c) = (line_length(&p), cut(&p, w));
        println!("| {w} | {l:.0} | {c:.0} |");
        rows.push((w, l, c));
    }
    for pair in rows.windows(2) {
        let ((w0, l0, c0), (w1, l1, c1)) = (pair[0], pair[1]);
        assert!(
            l1 < l0,
            "the line did not shorten from {w0} to {w1}: {l0} -> {l1}"
        );
        assert!(
            c1 > c0,
            "the cut did not rise from {w0} to {w1}: {c0} -> {c1}"
        );
    }
    // At weight 0 the line follows the water and cuts (almost) nothing.
    assert!(rows[0].2 < 1.0, "{}", rows[0].2);
    // At the largest weight the line cuts the neck: it is within 5 % of the straight line (it must
    // still leave and meet the river's line on its own heading at both ends, at R_min).
    let (_, l_big, _) = rows[rows.len() - 1];
    assert!(l_big < 1.05 * chord, "{l_big} vs {chord}");
}

/// (d) The `c_min` heuristic stays admissible with the weight on: on a small case it gives the same
/// cost as the search with no heuristic (plain Dijkstra).
#[test]
fn the_heuristic_stays_admissible_with_a_length_weight() {
    let cl = neck_river();
    for w in [5.0, 200.0] {
        let with = pool(&cl, 0.0, 1200.0, w, &neck_cfg());
        let mut none = neck_cfg();
        none.heuristic = "none".into();
        let without = pool(&cl, 0.0, 1200.0, w, &none);
        println!(
            "weight {w}: c_min {:.6} cost {:.3}; none {:.3}; expansions {} vs {}",
            with.result.c_min_per_m.unwrap(),
            with.result.cost,
            without.result.cost,
            with.result.expansions,
            without.result.expansions
        );
        assert!(with.result.c_min_per_m.unwrap() >= w);
        assert!(
            (with.result.cost - without.result.cost).abs() <= 1e-6 * without.result.cost,
            "weight {w}: {} vs {}",
            with.result.cost,
            without.result.cost
        );
    }
}

/// The scan that found `SMALLEST_LOOP_FREE_WEIGHT` (slow; run by hand with `--ignored`).
#[test]
#[ignore]
fn smallest_weight_scan() {
    let cl = centreline(110.0, 2000.0, LEN);
    let ws: Vec<f64> = std::env::var("OWD_SCAN_W")
        .unwrap_or("1,2,5,10,20,50,100".into())
        .split(',')
        .map(|x| x.parse().unwrap())
        .collect();
    let mut c = cfg();
    if let Ok(h) = std::env::var("OWD_HEUR") {
        c.heuristic = h;
    }
    for w in ws {
        let p = pool(&cl, 0.0, 3720.0, w, &c);
        let line = p.result.line.as_deref().unwrap();
        println!(
            "weight {w}: cost {:.0}, length {:.0}, cut {:.0}, closed {}, crossing {:?}",
            p.result.cost,
            line_length(&p),
            cut(&p, w),
            p.result.goal_closed,
            first_self_crossing_m(line)
        );
    }
}

/// The scan for the trade-off table (run by hand with `--ignored`).
#[test]
#[ignore]
fn trade_off_scan() {
    let th0: f64 = std::env::var("OWD_TH0").map_or(90.0, |v| v.parse().unwrap());
    // Default R_min 150 m: the trade-off case of `neck_pool`.
    let rmin: f64 = std::env::var("OWD_RMIN").map_or(150.0, |v| v.parse().unwrap());
    let cl = centreline(th0, 2000.0, 3000.0);
    let mut c = LatticeConfig::new(rmin);
    c.pitch_m = 40.0;
    let c = c.validated().unwrap();
    let a = linear::interpolate(&cl, 0.0).unwrap();
    let b = linear::interpolate(&cl, 2000.0).unwrap();
    println!(
        "th0 {th0} rmin {rmin} chord {:.0}",
        libm::hypot(b[0] - a[0], b[1] - a[1])
    );
    for w in [
        0.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 3000.0,
    ] {
        let p = pool(&cl, 0.0, 2000.0, w, &c);
        println!(
            "weight {w}: length {:.0}, cut {:.0}, closed {}, crossing {:?}",
            line_length(&p),
            cut(&p, w),
            p.result.goal_closed,
            first_self_crossing_m(p.result.line.as_deref().unwrap())
        );
    }
}
