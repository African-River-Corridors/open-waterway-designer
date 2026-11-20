//! A long pool (issue #559). The pool lattice searches each pool in one A* run with a cap on its
//! expansions. On a long open-river pool (tens of km at one water level) the search hits the cap and
//! returns no line, so the pool kept the raw river line. A pool longer than
//! `LatticeConfig::max_pool_piece_m` is now searched as consecutive pieces at the same water level,
//! each pinned to the baseline's pose at its ends, so the pieces meet tangentially and every piece
//! join is read for R_min like a pool join. A pool (or piece) that still hits the cap says so.
//!
//! The river is the closed-form sine of `lattice_pool_joins.rs`: amplitude 250 m, wavelength
//! 1,600 m (centreline radius about 260 m) against R_min 340 m; a river 80 m wide at -12 m in ground
//! at +3 m; a channel half-width of 25 m. The lattice runs on a coarse pitch and cell, and a reduced
//! expansion cap stands in for the default one, so a test stays short: the cap bites the whole pool
//! as the default cap bites a real, uneven pool tens of km long.
//!
//! Label: property

use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    DEFAULT_MAX_POOL_PIECE_M, HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, SectionSpec,
    optimise_pools, pool_pieces,
};

const R_MIN: f64 = 340.0;
const AMP: f64 = 250.0;
const WAVE: f64 = 1600.0;
const CORRIDOR_HALF_M: f64 = 300.0;
const RIVER_HALF_M: f64 = 15.0;

/// The sine, offset `off` metres to its left, every 10 m of x.
fn sine(len: f64, off: f64) -> Vec<Pt> {
    let k = 2.0 * core::f64::consts::PI / WAVE;
    (0..=(len / 10.0) as usize)
        .map(|i| {
            let x = i as f64 * 10.0;
            let y = AMP * libm::sin(k * x);
            let d = [1.0, AMP * k * libm::cos(k * x)];
            let m = libm::hypot(d[0], d[1]);
            [x - d[1] / m * off, y + d[0] / m * off]
        })
        .collect()
}

/// Distance from `p` to the centreline, read on the vertices within 400 m of x.
fn dist_to_centre(p: Pt, centre: &[Pt]) -> f64 {
    let n = centre.len();
    let i = (p[0] / 10.0).round().clamp(0.0, (n - 1) as f64) as usize;
    let (a, b) = (i.saturating_sub(40), (i + 40).min(n - 1));
    linear::point_line_distance(p, &centre[a..=b])
}

/// A coarse lattice, so a long case runs in seconds: pitch 24 m, cell 8 m.
fn coarse(max_expand: i64, max_piece_m: f64) -> LatticeConfig {
    let mut lat = LatticeConfig::new(R_MIN);
    lat.pitch_m = 24.0;
    lat.max_expand = max_expand;
    lat.max_pool_piece_m = max_piece_m;
    lat.validated().unwrap()
}

/// The lattice over the whole centreline of a river `len` m long in x, as one pool at one level.
fn run(len: f64, lat: &LatticeConfig) -> (Vec<Pt>, PoolsResult) {
    let mut ring = sine(len, RIVER_HALF_M);
    let mut right = sine(len, -RIVER_HALF_M);
    right.reverse();
    ring.extend(right);
    ring.push(ring[0]);
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let centre = sine(len, 0.0);
    let total = linear::length(&centre);
    let c2 = centre.clone();
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if dist_to_centre([x, y], &c2) <= RIVER_HALF_M + 5.0 {
                    -12.0
                } else {
                    3.0 + 1.5 * libm::sin(x / 130.0) * libm::sin(y / 170.0)
                }
            })
            .collect()
    };
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(25.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 100.0;
    spec.rake_step_m = 8.0;
    spec.ds_step_m = 16.0;
    let r = optimise_pools(
        &centre,
        &terrain,
        core::slice::from_ref(&poly),
        &[(0.0, total, 0.0)],
        3.25,
        &spec,
        lat,
        None,
        PoolOptions {
            corridor_half_m: CORRIDOR_HALF_M,
            cell_m: 8.0,
        },
        true,
    )
    .unwrap();
    (centre, r)
}

fn radius_near(line: &[Pt], s: f64) -> f64 {
    let chord = default_check_chord(R_MIN);
    let (a, b) = ((s - chord).max(0.0), (s + chord).min(linear::length(line)));
    match linear::substring(line, a, b) {
        owd_core::geom::Geometry::LineString(w) => min_radius_on_chord(&w, chord).unwrap(),
        other => panic!("a window came out as {other:?}"),
    }
}

fn seg_cross(p: Pt, q: Pt, r: Pt, s: Pt) -> bool {
    let o = |a: Pt, b: Pt, c: Pt| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let (d1, d2) = (o(r, s, p), o(r, s, q));
    let (d3, d4) = (o(p, q, r), o(p, q, s));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Without the split the whole pool hits the cap and keeps its baseline, and the record says so;
/// with it, every piece converges and the joined line holds R_min at every piece join, does not
/// cross itself, and stays in the corridor.
fn long_pool_case(len: f64, max_expand: i64, max_piece_m: f64) {
    // Before: one search over the whole pool.
    let (_, whole) = run(len, &coarse(max_expand, f64::INFINITY));
    assert_eq!(whole.pools.len(), 1);
    let p = &whole.pools[0];
    println!(
        "whole pool {:.0} m: converged {}, hit_cap {}, expansions {}, c_min {:?}, cost {}",
        p.end_m - p.start_m,
        p.result.converged,
        p.result.hit_cap,
        p.result.expansions,
        p.result.c_min_per_m,
        p.result.cost
    );
    assert!(p.result.hit_cap && !p.result.converged && p.result.line.is_none());
    assert!(!p.used && p.piece.is_none());
    assert_eq!(
        p.kept_baseline_because,
        Some("the search hit its expansion limit")
    );
    assert_eq!(whole.capped_pools().count(), 1);

    // After: the same pool, the same cap, searched as pieces.
    let (centre, r) = run(len, &coarse(max_expand, max_piece_m));
    let total = linear::length(&centre);
    let n = r.pools.len();
    assert!(n >= 2, "the pool is split ({n} pieces)");
    for (i, p) in r.pools.iter().enumerate() {
        let piece = p
            .piece
            .as_ref()
            .expect("each record is a piece of the pool");
        assert_eq!((piece.index, piece.count), (i, n));
        assert!(piece.pool_start_m.abs() < 1e-9 && (piece.pool_end_m - total).abs() < 1e-9);
        assert!(
            p.end_m - p.start_m <= 1.25 * max_piece_m + 1e-6,
            "piece {i} is {:.0} m",
            p.end_m - p.start_m
        );
        println!(
            "piece {i}: {:.0}-{:.0} m, expansions {}, converged {}",
            p.start_m, p.end_m, p.result.expansions, p.result.converged
        );
        assert!(
            !p.result.hit_cap && p.result.converged,
            "piece {i} converges"
        );
        assert!(
            p.used && p.result.goal_closed,
            "piece {i} is used and closed"
        );
        assert_eq!(p.water_level_m, 0.0);
    }
    for w in r.pools.windows(2) {
        assert!(
            (w[0].end_m - w[1].start_m).abs() < 1e-9,
            "pieces are consecutive"
        );
    }
    assert_eq!(r.capped_pools().count(), 0);
    // Every piece join is reported and holds R_min, read on the result and on the line.
    assert_eq!(r.joins.len(), n - 1);
    for (j, (rec, w)) in r.joins.iter().zip(r.pools.windows(2)).enumerate() {
        assert!(
            (rec.at_m - w[0].end_m).abs() < 1e-9,
            "join {j} is at the piece boundary"
        );
        assert!(rec.position_gap_m < 1e-6 && rec.heading_gap_rad < 1e-9);
        assert!(
            rec.radius_ok,
            "join {j} reads {}",
            rec.measured_min_radius_m
        );
        let at = linear::interpolate(&centre, rec.at_m).unwrap();
        let rj = radius_near(&r.line, linear::project(&r.line, at));
        assert!(
            radius_ok(rj, R_MIN),
            "join {j}: {rj:.1} m against {R_MIN} m"
        );
    }
    assert!(
        r.radius_ok,
        "the joined line reads {:.1} m against R_min {R_MIN} m",
        r.measured_min_radius_m
    );
    let l = &r.line;
    for i in 0..l.len().saturating_sub(1) {
        for k in i + 2..l.len() - 1 {
            assert!(
                !seg_cross(l[i], l[i + 1], l[k], l[k + 1]),
                "the joined line crosses itself at {:?}",
                l[i]
            );
        }
    }
    for p in l {
        assert!(
            dist_to_centre(*p, &centre) <= CORRIDOR_HALF_M + 1e-6,
            "{p:?} is outside the corridor"
        );
    }
}

/// The short twin, in the pull-request gate: a pool of about 9.7 km in pieces of 2.5 km at most.
#[test]
fn a_long_pool_is_searched_in_pieces_that_join_at_r_min() {
    long_pool_case(8000.0, 60_000, 2500.0);
}

/// The long case (nightly, `docs/agents/slow-tests.md`): a pool of about 31 km in pieces of the
/// default length.
#[test]
fn a_31_km_pool_is_searched_in_pieces_of_the_default_length() {
    long_pool_case(25_600.0, 300_000, DEFAULT_MAX_POOL_PIECE_M);
}

/// A pool that hits the cap is reported: the record says so, and it is not used.
#[test]
fn a_pool_that_hits_the_cap_says_so() {
    let (_, r) = run(4800.0, &coarse(500, f64::INFINITY));
    let p = &r.pools[0];
    assert!(p.result.hit_cap && p.result.expansions >= 499);
    assert!(!p.used && p.result.line.is_none());
    assert_eq!(
        p.kept_baseline_because,
        Some("the search hit its expansion limit")
    );
    assert_eq!(r.capped_pools().count(), 1);
    assert!(r.joins.is_empty());
}

/// A pool no longer than the piece length gives the same result as before the split existed.
#[test]
fn a_pool_shorter_than_the_piece_length_is_not_split() {
    let (centre, a) = run(2400.0, &coarse(4_000_000, DEFAULT_MAX_POOL_PIECE_M));
    assert!(linear::length(&centre) < DEFAULT_MAX_POOL_PIECE_M);
    let (_, b) = run(2400.0, &coarse(4_000_000, f64::INFINITY));
    assert_eq!(a, b);
    assert_eq!(a.pools.len(), 1);
    assert!(a.pools[0].piece.is_none() && a.pools[0].used);
}

/// The boundary rule: pieces as even as they can be, each boundary moved up to an eighth of a
/// piece to where the baseline turns least over R_min either side, so no piece end is pinned on a
/// tight bend.
#[test]
fn a_piece_boundary_moves_off_a_tight_bend() {
    // Straight east for 3,300 m, a 90-degree bend of radius 150 m, then straight north to 10 km.
    let r = 150.0;
    let mut line: Vec<Pt> = (0..=330).map(|i| [i as f64 * 10.0, 0.0]).collect();
    let c = [3300.0, r];
    for k in 1..=24 {
        let t = -core::f64::consts::FRAC_PI_2 + core::f64::consts::FRAC_PI_2 * k as f64 / 24.0;
        line.push([c[0] + r * libm::cos(t), c[1] + r * libm::sin(t)]);
    }
    let end = *line.last().unwrap();
    for i in 1..=640 {
        line.push([end[0], end[1] + i as f64 * 10.0]);
    }
    let len = linear::length(&line);
    let bend_mid = 3300.0 + r * core::f64::consts::FRAC_PI_4;
    // Even thirds put the first boundary about on the bend.
    assert!((len / 3.0 - bend_mid).abs() < 150.0, "{len}");
    let b = pool_pieces(&line, 0.0, len, 4000.0, R_MIN);
    assert_eq!(b.len(), 4, "{b:?}");
    assert_eq!((b[0], b[3]), (0.0, len));
    let nominal = len / 3.0;
    for (k, &s) in b[1..3].iter().enumerate() {
        assert!((s - nominal * (k + 1) as f64).abs() <= nominal / 8.0 + 1e-6);
    }
    // The first boundary is clear of the bend by at least R_min.
    let bend = (3300.0, 3300.0 + r * core::f64::consts::FRAC_PI_2);
    assert!(
        b[1] <= bend.0 - R_MIN + 1e-6 || b[1] >= bend.1 + R_MIN - 1e-6,
        "boundary at {:.0} m, bend at {bend:?}",
        b[1]
    );
    // A pool no longer than the piece length is one piece.
    assert_eq!(
        pool_pieces(&line, 100.0, 4100.0, 4000.0, R_MIN),
        vec![100.0, 4100.0]
    );
}
