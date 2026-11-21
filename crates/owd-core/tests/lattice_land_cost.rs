//! The pool search knows that leaving the river has a cost: `SectionSpec::land_weight`, a cost per
//! m2 of the channel's footprint (its plan width at the water surface) outside the river polygon.
//!
//! The fault it answers: ground the pool drowns deeper than the channel bed costs nothing to cut,
//! river or floodplain alike, so with no land cost the search runs straight over drowned land beside
//! the river. The cases are synthetic: a flat plain at -10 m under a pool at 0 m (a 3.5 m channel
//! has its bed at -3.5 m, so nothing anywhere needs cutting), and a river polygon 120 m wide round a
//! sine meander (heading = 50 degrees x sin(2 pi s / 1,600 m)) whose bends, at 292 m, are tighter
//! than R_min (340 m). The channel is 40 m wide at the bed with 1:3 sides: 61 m at the water
//! surface, which the rake reads as 15 samples of 4 m (60 m).
//!
//! Label: property

use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::overlay::contains_xy;
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    FieldScorer, HalfWidthSpec, LatticeConfig, PoolOptions, PoolsResult, Prim, SectionSpec,
    build_terrain_grid, optimise_pools,
};

const R_MIN: f64 = 340.0;
const DEPTH: f64 = 3.5;
const RIVER_HALF: f64 = 60.0;
/// A small cost per metre of line, on in every run here: on a plain where nothing costs anything
/// the search has no reason to prefer any line, and the length cost makes the line at land weight 0
/// the shortest one, which is the fault as it shows on a real river.
const LENGTH_WEIGHT: f64 = 1.0;
const OPTS: PoolOptions = PoolOptions {
    corridor_half_m: 300.0,
    cell_m: 10.0,
};

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

fn river(cl: &[Pt]) -> Vec<Polygon> {
    match buffer_line(cl, RIVER_HALF, 16, Cap::Round).unwrap() {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(p) => p,
        other => panic!("the river came out as {other:?}"),
    }
}

/// The drowned plain: -10 m everywhere, river and land alike.
fn plain(xs: &[f64], _: &[f64]) -> Vec<f64> {
    vec![-10.0; xs.len()]
}

fn spec(land_weight: f64) -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(20.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 80.0;
    spec.rake_step_m = 4.0;
    spec.ds_step_m = 8.0;
    spec.length_weight = LENGTH_WEIGHT;
    spec.land_weight = land_weight;
    spec.land_depth_m = DEPTH;
    spec
}

fn cfg() -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.pitch_m = 20.0;
    c.validated().unwrap()
}

fn meander() -> Vec<Pt> {
    centreline(50.0, 1600.0, 4800.0)
}

/// Two pools of the meander, joined at 2,400 m, at land weight `w`.
fn run(w: f64) -> PoolsResult {
    let cl = meander();
    let total = linear::length(&cl);
    optimise_pools(
        &cl,
        &plain,
        &river(&cl),
        &[(0.0, 2400.0, 0.0), (2400.0, total, 0.0)],
        DEPTH,
        &spec(w),
        &cfg(),
        None,
        OPTS,
        true,
    )
    .unwrap()
}

/// The share of the line's length whose centre is outside the river polygon, read every 5 m.
fn share_outside(line: &[Pt], river: &[Polygon]) -> f64 {
    let len = linear::length(line);
    let n = (len / 5.0).floor() as usize;
    let out = (0..=n)
        .filter(|&i| !contains_xy(river, linear::interpolate(line, i as f64 * 5.0).unwrap()))
        .count();
    out as f64 / (n + 1) as f64
}

/// The furthest the line goes from the river's centreline (m), read every 5 m.
fn furthest_from(line: &[Pt], cl: &[Pt]) -> f64 {
    let len = linear::length(line);
    (0..=(len / 5.0).floor() as usize)
        .map(|i| {
            let p = linear::interpolate(line, i as f64 * 5.0).unwrap();
            let q = linear::interpolate(cl, linear::project(cl, p)).unwrap();
            libm::hypot(p[0] - q[0], p[1] - q[1])
        })
        .fold(0.0, f64::max)
}

fn sound(r: &PoolsResult, w: f64) {
    assert!(r.pools.iter().all(|p| p.used), "weight {w}: {:?}", {
        r.pools
            .iter()
            .map(|p| p.kept_baseline_because)
            .collect::<Vec<_>>()
    });
    assert!(r.pools.iter().all(|p| p.result.goal_closed), "weight {w}");
    assert!(
        r.radius_ok,
        "weight {w}: reads {} m",
        r.measured_min_radius_m
    );
    assert_eq!(r.failed_joins().count(), 0, "weight {w}: {:?}", r.joins);
    assert!(!r.joins.is_empty(), "weight {w}");
    assert_eq!(r.self_crossing_m, None, "weight {w}");
}

/// On the drowned plain the line at land weight 0 leaves the river: about two thirds of its length
/// is outside the polygon, up to 300 m from the river's centreline, over land that needs no cut
/// (today's fault, pinned). A land weight brings it back. The weights are 0.005, 0.02 and 3.5 m3
/// per m2 (3.5 is the channel depth). Against the length cost of 1 per metre, a metre of line wholly
/// on land costs 0.3, 1.2 and 213: at 0.005 the land is still cheaper than the length it saves, so
/// the line still cuts across some bends; from 0.02 it stays in the river. The share of the line
/// outside the polygon and the land its footprint takes do not rise as the weight rises; at 0.02
/// and above under a twentieth of the line is outside and it stays within 60 m of the centreline
/// (the river's own half-width). At every weight R_min holds, every pool closes on its end pose,
/// every join passes and the line does not cross itself.
///
/// Measured (debug build, 2026-11-20): share outside 0.641, 0.318, 0.000, 0.000; furthest from the
/// centreline 301, 309, 39, 29 m; land 82,207, 2,375 and 80 m2 at the three weights.
#[test]
fn a_land_weight_keeps_the_line_in_the_river_on_a_drowned_plain() {
    let cl = meander();
    let poly = river(&cl);
    let off = run(0.0);
    sound(&off, 0.0);
    let share_off = share_outside(&off.line, &poly);
    let far_off = furthest_from(&off.line, &cl);
    println!("land weight 0: {share_off:.3} of the line outside, up to {far_off:.0} m away");
    assert!(share_off > 0.5, "{share_off}");
    assert!(far_off > 150.0, "{far_off}");
    assert!(off.pools.iter().all(|p| p.result.land_area_m2.is_none()));

    let mut last_share = share_off;
    let mut last_area = f64::INFINITY;
    for w in [0.005, 0.02, 3.5] {
        let r = run(w);
        sound(&r, w);
        let share = share_outside(&r.line, &poly);
        let far = furthest_from(&r.line, &cl);
        let area: f64 = r.pools.iter().map(|p| p.result.land_area_m2.unwrap()).sum();
        println!(
            "land weight {w}: {share:.3} of the line outside, up to {far:.0} m away, \
             {area:.0} m2 of land, {:.0} m long, {} expansions",
            linear::length(&r.line),
            r.pools.iter().map(|p| p.result.expansions).sum::<usize>()
        );
        assert!(
            share <= last_share + 1e-9,
            "weight {w}: {share} > {last_share}"
        );
        assert!(area <= last_area + 1e-6, "weight {w}: {area} > {last_area}");
        if w >= 0.02 {
            assert!(share < 0.05, "weight {w}: {share}");
            assert!(far < RIVER_HALF, "weight {w}: {far}");
        }
        (last_share, last_area) = (share, area);
    }
    assert!(last_share < 0.5 * share_off);
}

/// A straight of `len` metres, in 2 m steps.
fn straight(len: f64) -> Prim {
    let n = (len / 2.0) as usize;
    Prim {
        r: f64::INFINITY,
        sign: 0,
        pts: (1..=n)
            .map(|i| {
                let v = len * i as f64 / n as f64;
                [v, v, 0.0, 0.0]
            })
            .collect(),
        end: (len, 0.0, 0.0),
        arc_len: len,
    }
}

/// The land cost is charged on drowned land where nothing is cut, and not inside the river polygon.
/// The plain is at -10 m under a bed at -3.5 m, so the cut is 0 everywhere. The river is the band
/// y in -100..100. A 200 m straight along y = 0 is inside it: no land, no cost. The same straight
/// along y = 300 is on land: 15 rake samples of 4 m, 60 m of footprint, 12,000 m2, charged at the
/// weight although the cut is 0. Along the bank (y = 100) the half of the footprint on land pays.
/// At weight 0 nothing is charged anywhere.
#[test]
fn land_is_charged_where_nothing_is_cut_and_never_inside_the_river() {
    let band = [Polygon {
        exterior: vec![
            [-500.0, -100.0],
            [1500.0, -100.0],
            [1500.0, 100.0],
            [-500.0, 100.0],
            [-500.0, -100.0],
        ],
        holes: vec![],
    }];
    let area = [Polygon {
        exterior: vec![
            [-500.0, -600.0],
            [1500.0, -600.0],
            [1500.0, 600.0],
            [-500.0, 600.0],
            [-500.0, -600.0],
        ],
        holes: vec![],
    }];
    let grid = build_terrain_grid(&plain, &band, &area, 2.0, 20.0).unwrap();
    let prim = straight(200.0);
    let mut on = spec(3.5);
    on.length_weight = 0.0;
    let mut off = spec(0.0);
    off.length_weight = 0.0;
    let grade = -DEPTH;
    let sc = FieldScorer::new(&grid, grade, 20.0, &on);
    let sc_off = FieldScorer::new(&grid, grade, 20.0, &off);
    assert_eq!(sc.land_half_width_m(), 20.0 + 3.0 * DEPTH);

    let in_river = (100.0, 0.0, 0.0);
    assert_eq!(sc.land_area(&prim, in_river), 0.0);
    assert_eq!(sc.score(&prim, in_river), (0.0, true));

    let on_land = (100.0, 300.0, 0.0);
    let a = sc.land_area(&prim, on_land);
    assert!((a - 15.0 * 4.0 * 200.0).abs() < 1e-6, "{a}");
    let (c, ok) = sc.score(&prim, on_land);
    assert!(ok);
    assert!((c - 3.5 * a).abs() < 1e-6, "{c}");
    // The cut there is 0: with the weight off the same straight costs nothing.
    assert_eq!(sc_off.score(&prim, on_land), (0.0, true));

    // Along the bank: the samples on the land side of y = 100 pay, the others do not.
    let on_bank = (100.0, 100.0, 0.0);
    let half = sc.land_area(&prim, on_bank);
    assert!(half > 0.4 * a && half < 0.6 * a, "{half} of {a}");
    assert!((sc.score(&prim, on_bank).0 - 3.5 * half).abs() < 1e-6);
}

/// The baseline is priced with the same land cost as the lattice line, so the comparison that keeps
/// a cheaper baseline is like for like: here the baseline is the river's own centreline, wholly
/// inside the polygon, so it takes no land and costs its length only.
#[test]
fn the_baseline_pays_the_same_land_cost() {
    let r = run(3.5);
    for p in &r.pools {
        let land = p.baseline_land_area_m2.unwrap();
        assert!(land < 1.0, "{land}");
        let len = p.end_m - p.start_m;
        assert!((p.baseline_cost - LENGTH_WEIGHT * len).abs() < 1e-6 + 3.5 * land);
        // The lattice line holds R_min where the baseline does not, so it is the one used, and its
        // cost is its length and its land, with no cut.
        let line_len: f64 = p.result.prim_seq.iter().map(|q| q.2).sum();
        let area = p.result.land_area_m2.unwrap();
        assert!(
            (p.result.cost - LENGTH_WEIGHT * line_len - 3.5 * area).abs() < 1e-3 * p.result.cost,
            "cost {} length {line_len} land {area}",
            p.result.cost
        );
    }
}

/// A weight that is not a finite number >= 0 is refused by name.
#[test]
fn a_bad_land_weight_is_refused() {
    let cl = meander();
    for (w, d) in [(-1.0, DEPTH), (f64::NAN, DEPTH), (1.0, -2.0)] {
        let mut s = spec(w);
        s.land_depth_m = d;
        let e = optimise_pools(
            &cl,
            &plain,
            &river(&cl),
            &[(0.0, 500.0, 0.0)],
            DEPTH,
            &s,
            &cfg(),
            None,
            OPTS,
            true,
        )
        .unwrap_err();
        assert!(e.contains("land_"), "{e}");
    }
}
