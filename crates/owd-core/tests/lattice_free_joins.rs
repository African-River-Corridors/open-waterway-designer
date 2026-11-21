//! The joins of the pool search (OPT-3). A long pool is searched in pieces, and every piece used to
//! be pinned to the baseline's exact point and heading at both ends, as every pool is at its weirs.
//! A pinned pose on a bend forces a closing curve of R_min next to it, and a pinned pose off the
//! river forces the line off the river there.
//!
//! Three settings of `LatticeConfig` answer it:
//!
//! - `seams_on_straights`: the seams of a long pool go where the baseline is straight and in the
//!   river (`seams_on_straights`, `local_radius_m`).
//! - `free_seams`: at a seam the line passes a gate on the cross-line and the next piece starts from
//!   the pose the line ended on; at a weir site between two pools the line is straight for
//!   `weir_straight_m` through the gate.
//! - `min_straight_m`: a straight of at least that length between a left curve and a right curve.
//!
//! The river is synthetic: straights of 500 m between bends of radius 400 m that turn 50 degrees,
//! left, right, right, left, so it runs east. It is 80 m wide at -12 m, in ground at +3 m; the
//! channel is 50 m wide at the bed, 3.25 m under a pool at 0 m, so the river is free and the banks
//! cost. R_min is 340 m. The lattice is coarse (pitch 24 m, cell 8 m) so a case takes seconds.
//!
//! Label: property

use owd_core::alignment::radius_check::{default_check_chord, min_radius_on_chord};
use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::overlay::contains_xy;
use owd_core::geom::{Geometry, Polygon, Pt, linear};
use owd_core::optimise::lattice::{
    HalfWidthSpec, JointKind, LatticeConfig, PoolOptions, PoolsResult, SEAM_STRAIGHT_FACTOR,
    SectionSpec, WEIR_NO_STRAIGHT_REASON, local_radius_m, optimise_pools, seam_span_m,
    seams_on_straights, straight_through_m,
};
use std::time::Instant;

const R_MIN: f64 = 340.0;
const RIVER_HALF: f64 = 40.0;
const STRAIGHT: f64 = 500.0;
const BEND_R: f64 = 400.0;
const BEND_DEG: f64 = 50.0;
/// One straight and one bend, along the line.
const UNIT: f64 = STRAIGHT + BEND_R * BEND_DEG * core::f64::consts::PI / 180.0;

/// A line of straights and arcs in 5 m steps: `(length, radius, sign)` each, radius 0 for a
/// straight.
fn path(parts: &[(f64, f64, f64)]) -> Vec<Pt> {
    let (mut x, mut y, mut th) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut out = vec![[x, y]];
    for &(len, r, sgn) in parts {
        let n = (len / 5.0).ceil() as usize;
        let ds = len / n as f64;
        for _ in 0..n {
            let dth = if r > 0.0 { sgn * ds / r } else { 0.0 };
            // The chord of the step: exact for an arc.
            let mid = th + 0.5 * dth;
            let chord = if r > 0.0 {
                2.0 * r * libm::sin(0.5 * ds / r)
            } else {
                ds
            };
            x += chord * libm::cos(mid);
            y += chord * libm::sin(mid);
            th += dth;
            out.push([x, y]);
        }
    }
    out
}

/// `n` units of the river: a straight, then a bend; the bends go left, right, right, left.
fn river_line(n: usize) -> Vec<Pt> {
    let arc = BEND_R * BEND_DEG.to_radians();
    let signs = [1.0, -1.0, -1.0, 1.0];
    let mut parts = Vec::new();
    for k in 0..n {
        parts.push((STRAIGHT, 0.0, 0.0));
        parts.push((arc, BEND_R, signs[k % 4]));
    }
    parts.push((STRAIGHT, 0.0, 0.0));
    path(&parts)
}

fn polygons(cl: &[Pt], half: f64) -> Vec<Polygon> {
    match buffer_line(cl, half, 16, Cap::Round).unwrap() {
        Geometry::Polygon(p) => vec![p],
        Geometry::MultiPolygon(p) => p,
        other => panic!("the river came out as {other:?}"),
    }
}

fn spec() -> SectionSpec {
    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(25.0));
    spec.side_slope_h_per_v = 3.0;
    spec.rake_half_m = 100.0;
    spec.rake_step_m = 8.0;
    spec.ds_step_m = 16.0;
    // A small cost per metre, so the line in the free river is the short one and costs compare.
    spec.length_weight = 1.0;
    spec
}

fn cfg(max_piece_m: f64) -> LatticeConfig {
    let mut c = LatticeConfig::new(R_MIN);
    c.pitch_m = 24.0;
    c.max_pool_piece_m = max_piece_m;
    c.weir_straight_m = 180.0;
    c.validated().unwrap()
}

const OPTS: PoolOptions = PoolOptions {
    corridor_half_m: 300.0,
    cell_m: 8.0,
};

fn run_on(cl: &[Pt], pools: &[(f64, f64, f64)], c: &LatticeConfig, opts: PoolOptions) -> PoolsResult {
    let river = polygons(cl, RIVER_HALF);
    let wet = river.clone();
    let terrain = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                if contains_xy(&wet, [x, y]) {
                    -12.0
                } else {
                    3.0
                }
            })
            .collect()
    };
    optimise_pools(cl, &terrain, &river, pools, 3.25, &spec(), c, None, opts, true).unwrap()
}

/// The whole river as one pool at 0 m.
fn run(cl: &[Pt], c: &LatticeConfig) -> PoolsResult {
    run_on(cl, &[(0.0, linear::length(cl), 0.0)], c, OPTS)
}

fn window(line: &[Pt], a: f64, b: f64) -> Vec<Pt> {
    match linear::substring(line, a.max(0.0), b.min(linear::length(line))) {
        Geometry::LineString(w) => w,
        other => panic!("a window came out as {other:?}"),
    }
}

/// The line's minimum radius on the check chord within `reach` metres of chainage `s` of it.
fn radius_within(line: &[Pt], s: f64, reach: f64) -> f64 {
    let chord = default_check_chord(R_MIN);
    min_radius_on_chord(&window(line, s - reach, s + reach), chord).unwrap()
}

fn heading(a: Pt, b: Pt) -> f64 {
    libm::atan2(b[1] - a[1], b[0] - a[0])
}

fn wrap(d: f64) -> f64 {
    let t = d.rem_euclid(2.0 * core::f64::consts::PI);
    if t > core::f64::consts::PI {
        t - 2.0 * core::f64::consts::PI
    } else {
        t
    }
}

/// The shortest straight between a left curve and a right curve of `line` (m), read on its
/// vertices: a vertex turns when its two segments differ by more than 0.0001 rad. Infinite when the
/// line never reverses.
fn shortest_straight_between_opposite_curves(line: &[Pt]) -> f64 {
    let mut shortest = f64::INFINITY;
    let mut last: Option<(f64, f64)> = None; // (sign, chainage of the last turning vertex)
    let mut s = 0.0;
    for w in line.windows(3) {
        s += libm::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let turn = wrap(heading(w[1], w[2]) - heading(w[0], w[1]));
        if turn.abs() <= 1e-4 {
            continue;
        }
        let sign = turn.signum();
        if let Some((was, at)) = last
            && was != sign
        {
            shortest = shortest.min(s - at);
        }
        last = Some((sign, s));
    }
    shortest
}

fn total_cost(r: &PoolsResult) -> f64 {
    r.pools.iter().map(|p| p.result.cost).sum()
}

fn all_used(r: &PoolsResult) {
    assert!(
        r.pools.iter().all(|p| p.used),
        "{:?}",
        r.pools
            .iter()
            .map(|p| p.kept_baseline_because)
            .collect::<Vec<_>>()
    );
}

/// Ten units, about 8.5 km, in pieces of 2.5 km at most: four pieces, three seams.
fn long_river() -> Vec<Pt> {
    river_line(10)
}

/// With `seams_on_straights` every seam is on a straight of the river, inside the polygon, within
/// the window of its even split; the even splits themselves are not all on straights.
#[test]
fn seams_land_on_straights_inside_the_river() {
    let cl = long_river();
    let river = polygons(&cl, RIVER_HALF);
    let mut c = cfg(2500.0);
    c.seams_on_straights = true;
    c.seam_window_m = 600.0;
    let span = seam_span_m(&c);
    assert_eq!(span, 180.0, "two ship lengths of 90 m either side");
    let seams = seams_on_straights(&cl, &river, 0.0, linear::length(&cl), &c);
    assert_eq!(seams.len(), 3);
    let mut moved = 0;
    for s in &seams {
        println!(
            "seam: even split {:.0} m (local radius {:.0} m), placed at {:.0} m (local radius {:.0} m): {}",
            s.nominal_m,
            local_radius_m(&cl, s.nominal_m, span),
            s.at_m,
            s.local_radius_m,
            s.placed_because
        );
        assert!((s.at_m - s.nominal_m).abs() <= 600.0 + 1e-9);
        assert!(s.in_river);
        assert!(contains_xy(&river, linear::interpolate(&cl, s.at_m).unwrap()));
        // Straight: 180 m of straight river either side reads far above any bend of the river.
        assert!(
            s.local_radius_m >= SEAM_STRAIGHT_FACTOR * R_MIN,
            "the seam at {:.0} m reads {:.0} m",
            s.at_m,
            s.local_radius_m
        );
        // Read again here on the plain geometry: no bend within 180 m of the seam.
        let along = s.at_m % UNIT;
        assert!(
            along >= 180.0 - 5.0 && along <= STRAIGHT - 180.0 + 5.0,
            "the seam is {along:.0} m into its unit: not clear of the bends"
        );
        if local_radius_m(&cl, s.nominal_m, span) < 2.0 * BEND_R {
            moved += 1;
        }
    }
    assert!(moved >= 1, "at least one even split sits on a bend");
    // The stage records them: one joint per seam, with where it was put and why.
    let r = run(&cl, &c);
    assert_eq!(r.joints.len(), 3);
    for (j, s) in r.joints.iter().zip(&seams) {
        assert_eq!(j.kind, JointKind::Seam);
        assert_eq!(
            (j.at_m, j.nominal_m, j.placed_because),
            (s.at_m, Some(s.nominal_m), Some(s.placed_because))
        );
        assert!(!j.free && j.pinned_because.is_none());
    }
    // A seam off the river: with the polygon moved away, the reason says so.
    let far = polygons(&[[0.0, 5000.0], [100.0, 5000.0]], 10.0);
    let off = seams_on_straights(&cl, &far, 0.0, linear::length(&cl), &c);
    assert!(off.iter().all(|s| !s.in_river
        && s.placed_because.contains("no point of the window is inside the river polygon")));
}

/// With `free_seams` (and the seams on straights) the pieces meet with no gap in position or
/// heading, no seam sits on a tight curve, the line costs no more than with pinned seams, R_min
/// holds and the line has no loop.
#[test]
fn free_seams_join_with_no_gap_and_no_tight_curve() {
    let cl = long_river();
    let river = polygons(&cl, RIVER_HALF);
    let mut pinned_cfg = cfg(2500.0);
    pinned_cfg.seams_on_straights = true;
    pinned_cfg.seam_window_m = 600.0;
    let mut free_cfg = pinned_cfg.clone();
    free_cfg.free_seams = true;

    let t = Instant::now();
    let pinned = run(&cl, &pinned_cfg);
    let t_pinned = t.elapsed();
    let t = Instant::now();
    let free = run(&cl, &free_cfg);
    let t_free = t.elapsed();
    let expansions = |r: &PoolsResult| r.pools.iter().map(|p| p.result.expansions).sum::<usize>();
    println!(
        "pinned seams: {t_pinned:.2?}, {} expansions, cost {:.0}; free seams: {t_free:.2?}, {} expansions, cost {:.0}",
        expansions(&pinned),
        total_cost(&pinned),
        expansions(&free),
        total_cost(&free)
    );
    all_used(&pinned);
    all_used(&free);
    assert_eq!(free.pools.len(), 4);
    assert_eq!(free.joints.len(), 3);

    for (i, w) in free.pools.windows(2).enumerate() {
        let (a, b) = (
            w[0].result.line.as_ref().unwrap(),
            w[1].result.line.as_ref().unwrap(),
        );
        let (end, start) = (a[a.len() - 1], b[0]);
        // No gap in position, read on the two lines themselves.
        let gap = libm::hypot(end[0] - start[0], end[1] - start[1]);
        assert!(gap < 1e-6, "seam {i}: the pieces are {gap} m apart");
        // No gap in heading: the last step of one against the first step of the next. An arc of
        // R_min in 2 m steps turns 0.34 degrees a step, so half a degree is no kink.
        let kink = wrap(heading(b[0], b[1]) - heading(a[a.len() - 2], a[a.len() - 1]))
            .abs()
            .to_degrees();
        assert!(kink < 0.5, "seam {i}: a kink of {kink:.3} degrees");
        let j = &free.joints[i];
        assert!(j.free, "seam {i} is pinned: {:?}", j.pinned_because);
        assert!(w[0].free_end && w[1].free_start);
        assert!(w[0].result.goal_closed);
        // The line passed the gate: on the cross-line, inside the river, within the tolerance.
        let off = j.offset_m.unwrap();
        assert!(j.gate_lo_m.unwrap() <= off && off <= j.gate_hi_m.unwrap());
        assert!(j.heading_off_rad.unwrap().abs() <= 30f64.to_radians() + 1e-9);
        assert!(contains_xy(&river, end), "seam {i}: the line is off the river");
        let on_line = linear::interpolate(&cl, j.at_m).unwrap();
        let ahead = heading(on_line, linear::interpolate(&cl, j.at_m + 1.0).unwrap());
        let u = (end[0] - on_line[0]) * libm::cos(ahead) + (end[1] - on_line[1]) * libm::sin(ahead);
        assert!(u.abs() < 0.5, "seam {i}: {u:.2} m off the cross-line");
        // No tight curve at the seam: within 150 m of it the joined line is no tighter than
        // 1.4 x R_min (476 m). The river is straight there, so a straight was to be had.
        let s = linear::project(&free.line, end);
        let near = radius_within(&free.line, s, 150.0);
        println!(
            "seam {i} at {:.0} m: offset {off:.1} m, heading off {:.1} deg, radius within 150 m {near:.0} m, straight through {:.0} m",
            j.at_m,
            j.heading_off_rad.unwrap().to_degrees(),
            j.straight_through_m
        );
        assert!(
            near >= 1.4 * R_MIN,
            "seam {i}: {near:.0} m within 150 m of it"
        );
    }
    // The existing join checks agree.
    assert_eq!(free.failed_joins().count(), 0, "{:?}", free.joins);
    for j in &free.joins {
        assert!(j.position_gap_m < 1e-6 && j.heading_gap_rad < 1e-9);
    }
    assert!(
        total_cost(&free) <= total_cost(&pinned) + 1e-6,
        "free {} against pinned {}",
        total_cost(&free),
        total_cost(&pinned)
    );
    assert!(free.radius_ok, "reads {} m", free.measured_min_radius_m);
    assert_eq!(free.self_crossing_m, None);
    assert!(!owd_core::alignment::loops::is_self_intersecting(&free.line));
}

/// Two pools that meet at a weir on a straight: with `free_seams` the line is straight for the
/// required length through the weir site, half each side of its cross-line, and the two pools'
/// lines meet there with no gap.
#[test]
fn the_line_is_straight_through_a_weir_site_on_a_straight() {
    let cl = river_line(4);
    let total = linear::length(&cl);
    // The middle of the third straight.
    let weir = 2.0 * UNIT + 0.5 * STRAIGHT;
    let mut c = cfg(6000.0);
    c.free_seams = true;
    let r = run_on(&cl, &[(0.0, weir, 1.0), (weir, total, 0.0)], &c, OPTS);
    all_used(&r);
    assert_eq!(r.joints.len(), 1);
    let j = &r.joints[0];
    println!("{j:?}");
    assert_eq!(j.kind, JointKind::Weir);
    assert!(j.free, "{:?}", j.pinned_because);
    assert_eq!(j.straight_required_m, Some(180.0));
    assert_eq!(j.straight_ok, Some(true));
    assert!(j.straight_through_m >= 180.0);
    // Read here on the line: 90 m either side of where the line passes the weir's cross-line the
    // heading does not change.
    let (a, b) = (
        r.pools[0].result.line.as_ref().unwrap(),
        r.pools[1].result.line.as_ref().unwrap(),
    );
    let at = a[a.len() - 1];
    assert!(libm::hypot(at[0] - b[0][0], at[1] - b[0][1]) < 1e-6);
    let s = linear::project(&r.line, at);
    let w = window(&r.line, s - 90.0, s + 90.0);
    let h0 = heading(w[0], w[1]);
    for seg in w.windows(2) {
        if libm::hypot(seg[1][0] - seg[0][0], seg[1][1] - seg[0][1]) > 1e-6 {
            assert!(wrap(heading(seg[0], seg[1]) - h0).abs() < 1e-6, "a turn within 90 m of the weir");
        }
    }
    assert!(straight_through_m(&r.line, s) >= 180.0);
    assert!(r.radius_ok && r.self_crossing_m.is_none());
    assert_eq!(r.failed_joins().count(), 0);
}

/// A weir on a bend of radius 150 m, tighter than R_min, with the corridor as a hard limit 60 m
/// either side: no straight of 300 m fits (its ends would be 150 m off the bend's line). The weir
/// site says so by name and is pinned to the baseline's pose, as it was before the setting.
#[test]
fn a_weir_on_a_bend_tighter_than_r_min_names_its_reason() {
    let quarter = 150.0 * core::f64::consts::FRAC_PI_2;
    let cl = path(&[(900.0, 0.0, 0.0), (quarter, 150.0, 1.0), (900.0, 0.0, 0.0)]);
    let total = linear::length(&cl);
    let weir = 900.0 + 0.5 * quarter;
    let mut c = cfg(6000.0);
    c.free_seams = true;
    c.weir_straight_m = 300.0;
    c.corridor_limit = true;
    let c = c.validated().unwrap();
    let opts = PoolOptions {
        corridor_half_m: 60.0,
        cell_m: 8.0,
    };
    let r = run_on(&cl, &[(0.0, weir, 1.0), (weir, total, 0.0)], &c, opts);
    assert_eq!(r.joints.len(), 1);
    let j = &r.joints[0];
    println!("{j:?}");
    assert_eq!(j.kind, JointKind::Weir);
    assert!(!j.free);
    assert_eq!(j.pinned_because, Some(WEIR_NO_STRAIGHT_REASON));
    assert_eq!(j.straight_ok, Some(false));
    assert!(j.local_radius_m < R_MIN, "the weir is on a bend tighter than R_min");
    // Pinned: no pool's search ended on a gate or started from a free pose.
    assert!(r.pools.iter().all(|p| !p.free_end && !p.free_start));
}

/// `min_straight_m` 90: on the line, no left curve meets a right curve over a straight shorter than
/// 90 m. Without it the lattice's own straight (two pitches, 48 m) is all that separates them.
#[test]
fn a_minimum_straight_separates_opposite_curves() {
    // Bends of 250 m, tighter than R_min, with 120 m between them: the line must swing wide.
    let arc = 250.0 * 60f64.to_radians();
    let cl = path(&[
        (400.0, 0.0, 0.0),
        (arc, 250.0, 1.0),
        (120.0, 0.0, 0.0),
        (arc, 250.0, -1.0),
        (120.0, 0.0, 0.0),
        (arc, 250.0, -1.0),
        (120.0, 0.0, 0.0),
        (arc, 250.0, 1.0),
        (400.0, 0.0, 0.0),
    ]);
    let run_with = |min_straight: f64| {
        let mut c = cfg(6000.0);
        c.min_straight_m = min_straight;
        let c = c.validated().unwrap();
        let t = Instant::now();
        let r = run(&cl, &c);
        let p = &r.pools[0];
        let short = shortest_straight_between_opposite_curves(p.result.line.as_ref().unwrap());
        println!(
            "min_straight_m {min_straight}: {:.2?}, {} expansions, {} states, cost {:.0}, shortest straight between opposite curves {short:.1} m",
            t.elapsed(),
            p.result.expansions,
            p.result.n_states,
            p.result.cost
        );
        (r, short)
    };
    let (off, short_off) = run_with(0.0);
    let (on, short_on) = run_with(90.0);
    assert!(off.pools[0].result.converged && on.pools[0].result.converged);
    assert!(short_off < 90.0, "the case has a short S-bend without the setting: {short_off}");
    assert!(short_on >= 90.0 - 1e-6, "a straight of {short_on:.1} m between opposite curves");
    assert!(on.pools[0].result.radius_ok);
    // A stricter search costs no less.
    assert!(on.pools[0].result.cost >= off.pools[0].result.cost - 1e-6);
}

/// The new settings refuse a value that is not a length, and say which.
#[test]
fn the_settings_are_validated() {
    for (name, set) in [
        ("min_straight_m", (|c| c.min_straight_m = -1.0) as fn(&mut LatticeConfig)),
        ("seam_window_m", |c| c.seam_window_m = f64::NAN),
        ("weir_straight_m", |c| c.weir_straight_m = -5.0),
        ("seam_offset_m", |c| c.seam_offset_m = Some(-1.0)),
        ("seam_heading_tol_deg", |c| c.seam_heading_tol_deg = 90.0),
        ("min_straight_m", |c| c.min_straight_m = 1.0e6),
    ] {
        let mut c = LatticeConfig::new(R_MIN);
        set(&mut c);
        let err = c.validated().unwrap_err();
        assert!(err.contains(name), "{name}: {err}");
    }
}
