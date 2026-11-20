//! Joins between optimised pools (issue #6). Each pool's lattice line holds R_min by construction,
//! but the joined line must hold it too, across every pool join: the line of one pool must end on
//! the pose the next one starts from (position and heading), so the two meet tangentially.
//!
//! The river is a closed-form sine: amplitude 250 m, wavelength 1,600 m (centreline radius about
//! 260 m) against R_min 340 m; a river 80 m wide at -12 m in ground at +3 m; a channel half-width
//! of 25 m. The baseline is the centreline, which breaks R_min, so every pool's lattice line is used.
//!
//! Label: property

use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord, radius_ok};
use owd_core::geom::{Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, SectionSpec, optimise_pools,
};

const R_MIN: f64 = 340.0;
const AMP: f64 = 250.0;
const WAVE: f64 = 1600.0;
const CORRIDOR_HALF_M: f64 = 300.0;

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

/// Distance from `p` to the centreline, read on the vertices within 400 m of x (the sine's x
/// grows by 10 m a vertex, and no point of the river is further than that from its own x).
fn dist_to_centre(p: Pt, centre: &[Pt]) -> f64 {
    let n = centre.len();
    let i = (p[0] / 10.0).round().clamp(0.0, (n - 1) as f64) as usize;
    let (a, b) = (i.saturating_sub(40), (i + 40).min(n - 1));
    linear::point_line_distance(p, &centre[a..=b])
}

/// Run the lattice over `n` pools of equal length that cover the whole centreline.
fn run(len: f64, n: usize) -> (Vec<Pt>, Vec<(f64, f64)>, PoolsResult) {
    let mut ring = sine(len, 40.0);
    let mut right = sine(len, -40.0);
    right.reverse();
    ring.extend(right);
    ring.push(ring[0]);
    let poly = Polygon {
        exterior: ring,
        holes: vec![],
    };
    let centre = sine(len, 0.0);
    let total = linear::length(&centre);
    let pools: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            (
                total * i as f64 / n as f64,
                total * (i + 1) as f64 / n as f64,
            )
        })
        .collect();
    let c2 = centre.clone();
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if dist_to_centre([x, y], &c2) <= 45.0 {
                    -12.0
                } else {
                    3.0
                }
            })
            .collect()
    };
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(25.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 100.0;
    let lat = LatticeConfig::new(R_MIN).validated().unwrap();
    let levels: Vec<(f64, f64, f64)> = pools.iter().map(|&(a, b)| (a, b, 0.0)).collect();
    let r = optimise_pools(
        &centre,
        &terrain,
        core::slice::from_ref(&poly),
        &levels,
        3.25,
        &spec,
        &lat,
        None,
        PoolOptions {
            corridor_half_m: CORRIDOR_HALF_M,
            cell_m: 5.0,
        },
        true,
    )
    .unwrap();
    (centre, pools, r)
}

/// The radius of `line` read on the default chord over a window of one chord either side of the
/// point at chainage `s` (every three-point reading whose middle is within half a chord of it).
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

/// Every check the issue asks for, on one case. Returns the radius at each join.
fn check(len: f64, n: usize) -> Vec<f64> {
    let (centre, pools, r) = run(len, n);
    for (n, p) in r.pools.iter().enumerate() {
        assert!(p.used, "pool {n} is used (its baseline breaks R_min)");
    }
    // The radius across each pool join (a boundary between two pools, inside the line).
    let mut radii = Vec::new();
    for w in pools.windows(2) {
        let at = linear::interpolate(&centre, w[0].1).unwrap();
        let s = linear::project(&r.line, at);
        radii.push(radius_near(&r.line, s));
    }
    println!(
        "{len} m, pools {pools:?}: join radii {radii:?}, line {} m",
        r.measured_min_radius_m
    );
    for (j, &rj) in radii.iter().enumerate() {
        assert!(
            radius_ok(rj, R_MIN),
            "join {j} of {pools:?}: {rj:.1} m against R_min {R_MIN} m"
        );
    }
    assert!(
        r.radius_ok,
        "the joined line reads {:.1} m against R_min {R_MIN} m",
        r.measured_min_radius_m
    );
    // No self-crossing (segments not next to each other never cross).
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
    // Inside the corridor: within its half-width of the baseline (the corridor's buffer).
    for p in l {
        assert!(
            dist_to_centre(*p, &centre) <= CORRIDOR_HALF_M + 1e-6,
            "{p:?} is outside the corridor"
        );
    }
    radii
}

/// The river's x runs 0 to `len`; its centreline is about 1.3 times as long.
#[test]
fn two_pools_join_at_r_min() {
    check(4800.0, 2);
}

#[test]
fn three_shorter_pools_join_at_r_min() {
    check(4800.0, 3);
}
