//! The sideways shift of the river-following line (spec OPT-4, part 4), on synthetic rivers: the
//! curvature formula (R6), the shift off a hill (R7), the same-curves rule (R8), the step rule.

use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::alignment::river_following::choice::ChoiceParams;
use owd_core::alignment::river_following::cuts::CutParams;
use owd_core::alignment::river_following::elements::{Chain, advance};
use owd_core::alignment::river_following::mask::RiverMask;
use owd_core::alignment::river_following::score::{PlainScorer, totals};
use owd_core::alignment::river_following::seed::SeedParams;
use owd_core::alignment::river_following::shift::{
    ShiftParams, offset_step, shifted_curvature, station_spacing,
};
use owd_core::alignment::river_following::{Params, fixtures, run};
use owd_core::geom::{Pt, linear};
use std::collections::BTreeMap;

const R_MIN: f64 = 300.0;
const LOA: f64 = 75.0;

/// The signed curvature of a dense line at point `i`, from the circle through three points.
fn numeric_k(p: &[Pt], i: usize, k: usize) -> f64 {
    let (a, b, c) = (p[i - k], p[i], p[i + k]);
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let la = libm::hypot(b[0] - c[0], b[1] - c[1]);
    let lb = libm::hypot(a[0] - c[0], a[1] - c[1]);
    let lc = libm::hypot(a[0] - b[0], a[1] - b[1]);
    2.0 * cross / (la * lb * lc)
}

/// R6: the curvature of the shifted line against the numerical curvature of the line built.
#[test]
fn r6_the_curvature_formula_matches_the_built_line() {
    let r0 = 500.0;
    // d = 0 gives the line's own curvature; a constant d on an arc of R0 gives 1 / (R0 - d).
    assert!((shifted_curvature(1.0 / r0, 0.0, 0.0, 0.0, 0.0) - 1.0 / r0).abs() < 1e-15);
    for d in [-40.0, -5.0, 12.0, 60.0] {
        let k = shifted_curvature(1.0 / r0, 0.0, d, 0.0, 0.0);
        assert!((k - 1.0 / (r0 - d)).abs() < 1e-9 * k.abs(), "{d}: {k}");
        let k = shifted_curvature(-1.0 / r0, 0.0, d, 0.0, 0.0);
        assert!((k + 1.0 / (r0 + d)).abs() < 1e-9 * k.abs(), "{d}: {k}");
    }
    // Offsets a sin²(2πs/λ) on an arc of each hand and on a straight, and on a line whose curvature
    // changes (an arc into a tighter arc, so k0' is not zero there).
    let (a, lambda) = (25.0, 900.0);
    let ds = 0.5;
    let mut worst: f64 = 0.0;
    for k0 in [1.0 / r0, -1.0 / r0, 0.0, 1.0 / 1200.0] {
        let n = (2.0 * lambda / ds) as usize;
        let w = 2.0 * core::f64::consts::PI / lambda;
        let mut pose = (0.0, 0.0, 0.3);
        let mut pts = Vec::with_capacity(n + 1);
        let mut want = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let s = i as f64 * ds;
            let sn = libm::sin(w * s);
            let cs = libm::cos(w * s);
            let d = a * sn * sn;
            let d1 = 2.0 * a * w * sn * cs;
            let d2 = 2.0 * a * w * w * (cs * cs - sn * sn);
            pts.push([
                pose.0 - d * libm::sin(pose.2),
                pose.1 + d * libm::cos(pose.2),
            ]);
            want.push(shifted_curvature(k0, 0.0, d, d1, d2));
            pose = advance(pose, k0, ds);
        }
        for i in (20..n - 20).step_by(37) {
            let got = numeric_k(&pts, i, 4);
            let rel = (got - want[i]).abs() / want[i].abs().max(1.0 / 5000.0);
            worst = worst.max(rel);
        }
    }
    eprintln!("R6: largest relative gap {:.4} %", 100.0 * worst);
    assert!(worst < 0.001, "largest relative gap {worst}");
}

/// The step rule of spec 4.8: δ / h² at most 1 / (6 R_min).
#[test]
fn the_step_sizes_follow_r_min() {
    for r in [100.0, 180.0, 300.0, 360.0, 1000.0] {
        let h = station_spacing(r);
        let d = offset_step(h, r);
        assert!((20.0..=50.0).contains(&h));
        assert!(
            d / (h * h) <= 1.0 / (6.0 * r) + 1e-12 || d == 0.25,
            "{r}: {h} {d}"
        );
        eprintln!("R_min {r}: stations {h:.1} m, offset step {d} m");
    }
    assert_eq!(station_spacing(300.0), 50.0);
    assert_eq!(offset_step(50.0, 300.0), 1.25);
    assert_eq!(station_spacing(180.0), 30.0);
    assert_eq!(offset_step(30.0, 180.0), 0.75);
}

fn params(r_min: f64, loa: f64, shift: bool) -> Params {
    // The default tolerance: 5 m (one arc per bend).
    let seed = SeedParams::scaled(r_min, 1.0333, loa, 5.0);
    let mut sp = ShiftParams::scaled(r_min, 1.0694 * r_min, 60.0, 2.0 * loa);
    sp.same_curves = true;
    Params {
        cut: CutParams::scaled(r_min, 1.0556 * r_min, 2.0 * loa, 1.3, 100.0, loa),
        choice: ChoiceParams {
            min_save_m: 2.0 * loa,
            min_gain: 0.05,
            face_max_m: 30.0,
            step_m: 25.0,
        },
        shift: shift.then_some(sp),
        off_river_min_m: 2.0 * loa,
        forced_room_m: 15.0,
        seed,
    }
}

/// R7: a bend against a hill on its inside: the shift moves the line out, holds the limit, and
/// lowers the tallest face.
#[test]
fn r7_the_shift_moves_off_a_hill_on_the_inside_of_a_bend() {
    use core::f64::consts::PI;
    let width = 120.0;
    let river = fixtures::bends(1500.0, &[(PI / 2.0, 600.0)]);
    let poly = fixtures::polygon(&river, width);
    let mask = RiverMask::new(std::slice::from_ref(&poly), -3.0);
    let water = RiverMask::new(std::slice::from_ref(&poly), 0.0);
    // The bend's centre: left turn of R 600 from (1500, 0) heading +x.
    let centre = [1500.0, 600.0];
    let ground = |q: Pt| {
        if water.contains(q) {
            return -1.0;
        }
        // Inside the bend (nearer the centre than the river): rising 1 : 2 from the inner bank.
        let r = libm::hypot(q[0] - centre[0], q[1] - centre[1]);
        let inner = 600.0 - width / 2.0;
        if r < inner && q[0] > 1400.0 && q[1] < 700.0 {
            1.0 + (inner - r) / 2.0
        } else {
            1.0
        }
    };
    let widen = |r: f64| if r < 5.0 * R_MIN { 3000.0 / r } else { 0.0 };
    let scorer = PlainScorer {
        ground: Some(&ground),
        water: &|_| 0.0,
        depth_m: 4.0,
        bottom_m: 30.0,
        side_slope: 3.0,
        widening: &widen,
        river: Some(&water),
        wet_weight: 1.0,
        dry_weight: 1.0,
        land_weight: 0.0,
    };
    let p = params(R_MIN, LOA, true);
    let river_at = |q: Pt| linear::project(&river, q);
    let out = run(
        &river,
        Some(&mask),
        &p,
        &scorer,
        &widen,
        &BTreeMap::new(),
        &river_at,
    );
    let (st, sh) = out.shift.as_ref().unwrap();
    let mean_off: f64 = sh.offsets.iter().sum::<f64>() / sh.offsets.len() as f64;
    eprintln!(
        "R7: cost {:.0} -> {:.0}, offsets {:.1} .. {:.1} (mean {mean_off:.2}), {} stations, gap {:.2} m, refined {}",
        sh.cost_zero,
        sh.cost,
        sh.offsets.iter().copied().fold(f64::INFINITY, f64::min),
        sh.offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        st.len(),
        out.tracking_gap_m,
        sh.refined
    );
    assert!(sh.cost < sh.cost_zero);
    // A left bend: out is to the right, negative offsets.
    assert!(mean_off < 0.0);
    let line = out.line.points();
    let r = min_radius_on_chord(&line, R_MIN / 3.0).unwrap();
    assert!(radius_ok(r, R_MIN), "{r}");
    assert!(out.line.least_radius() >= R_MIN - 1e-9);
    let face = |c: &Chain| totals(c, 0.0, c.length(), 10.0, &scorer, 30.0).face_max_m;
    let (f0, f1) = (face(&out.after_cuts), face(&out.line));
    eprintln!("R7: tallest face {f0:.2} -> {f1:.2} m");
    assert!(f1 < f0);
    assert!(out.tracking_gap_m <= 5.0, "{}", out.tracking_gap_m);
}

/// The reversals of the line's arcs: changes of hand between arcs in a row.
fn reversals(c: &Chain) -> usize {
    let hands: Vec<i8> = c
        .elements
        .iter()
        .filter(|e| e.is_arc() && e.radius() < 8.0 * R_MIN)
        .map(|e| e.hand())
        .collect();
    hands.windows(2).filter(|w| w[0] != w[1]).count()
}

/// R8: a gentle S of the river through flat ground: with the same-curves rule, the shift puts in no
/// extra reversal; without it, the record of how many appear.
#[test]
fn r8_the_same_curves_rule_keeps_a_gentle_s_gentle() {
    use core::f64::consts::PI;
    let river = fixtures::bends(800.0, &[(PI / 3.0, 3.0 * R_MIN), (-PI / 3.0, 3.0 * R_MIN)]);
    let poly = fixtures::polygon(&river, 200.0);
    let mask = RiverMask::new(std::slice::from_ref(&poly), -3.0);
    let water = RiverMask::new(std::slice::from_ref(&poly), 0.0);
    // Flat ground with a little relief, so the shift has something to gain.
    let ground = |q: Pt| {
        if water.contains(q) {
            -1.0 + 0.5 * libm::sin(q[0] / 37.0) * libm::cos(q[1] / 23.0)
        } else {
            1.0
        }
    };
    let widen = |_: f64| 0.0;
    let scorer = PlainScorer {
        ground: Some(&ground),
        water: &|_| 0.0,
        depth_m: 4.0,
        bottom_m: 30.0,
        side_slope: 3.0,
        widening: &widen,
        river: Some(&water),
        wet_weight: 1.0,
        dry_weight: 1.0,
        land_weight: 0.0,
    };
    let river_at = |q: Pt| linear::project(&river, q);
    let seed_rev = {
        let p = params(R_MIN, LOA, false);
        reversals(
            &run(
                &river,
                Some(&mask),
                &p,
                &scorer,
                &widen,
                &BTreeMap::new(),
                &river_at,
            )
            .line,
        )
    };
    let mut on = params(R_MIN, LOA, true);
    on.shift.as_mut().unwrap().w_bend = 0.0;
    let with = run(
        &river,
        Some(&mask),
        &on,
        &scorer,
        &widen,
        &BTreeMap::new(),
        &river_at,
    );
    let mut off = on.clone();
    off.shift.as_mut().unwrap().same_curves = false;
    let without = run(
        &river,
        Some(&mask),
        &off,
        &scorer,
        &widen,
        &BTreeMap::new(),
        &river_at,
    );
    eprintln!(
        "R8: reversals seed {seed_rev}, shifted with the rule {}, without {}",
        reversals(&with.line),
        reversals(&without.line)
    );
    assert!(reversals(&with.line) <= seed_rev);
}

/// R9 through the shift: a small R_min (100 m) on a sine river of 150 m bends with some relief: the
/// delivered line has no short S-bend, and the stations and offset step come from R_min.
#[test]
fn r9_the_shifted_line_has_no_short_s_bend_at_a_small_r_min() {
    let r_min = 100.0;
    let loa = 50.0;
    let wl = 900.0;
    let river = fixtures::sine(fixtures::sine_amplitude_for_radius(150.0, wl), wl, 5400.0);
    let poly = fixtures::polygon(&river, 60.0);
    let mask = RiverMask::new(std::slice::from_ref(&poly), -3.0);
    let water = RiverMask::new(std::slice::from_ref(&poly), 0.0);
    let ground = |q: Pt| {
        if water.contains(q) {
            -1.0 + 0.8 * libm::sin(q[0] / 29.0) * libm::cos(q[1] / 17.0)
        } else {
            2.0
        }
    };
    let widen = |r: f64| if r < 5.0 * r_min { 600.0 / r } else { 0.0 };
    let scorer = PlainScorer {
        ground: Some(&ground),
        water: &|_| 0.0,
        depth_m: 3.0,
        bottom_m: 25.0,
        side_slope: 3.0,
        widening: &widen,
        river: Some(&water),
        wet_weight: 1.0,
        dry_weight: 1.0,
        land_weight: 0.0,
    };
    let p = params(r_min, loa, true);
    let sp = p.shift.as_ref().unwrap();
    assert_eq!((sp.station_m, sp.offset_step_m), (20.0, 0.5));
    let river_at = |q: Pt| linear::project(&river, q);
    let out = run(
        &river,
        Some(&mask),
        &p,
        &scorer,
        &widen,
        &BTreeMap::new(),
        &river_at,
    );
    let short = out.line.short_reversals(2.0 * r_min, loa);
    eprintln!(
        "R9: {} elements, short S-bends {}, gap {:.2} m",
        out.line.elements.len(),
        short.len(),
        out.tracking_gap_m
    );
    assert!(short.is_empty(), "{short:?}");
    assert!(out.line.least_radius() >= r_min - 1e-9);
}

/// Performance: the whole method on a synthetic river of about 200 km with 300 bends at R_min
/// 180 m, priced on plan area (the time of the geometry and the programme; the volume scorer's
/// time is its calls times its cost per call, reported apart). Run with
/// `cargo test --release -- --ignored a_long_river`.
#[test]
#[ignore]
fn a_long_river_end_to_end() {
    let r_min = 180.0;
    let loa = 90.0;
    let river = fixtures::meanders(300, 150.0, 700.0, 250.0, 7);
    let poly = fixtures::polygon(&river, 120.0);
    let mask = RiverMask::new(std::slice::from_ref(&poly), -3.0);
    let water = RiverMask::new(std::slice::from_ref(&poly), 0.0);
    let widen = |r: f64| {
        if r < 5.0 * r_min {
            0.5 * loa * loa / r
        } else {
            0.0
        }
    };
    let scorer = PlainScorer {
        ground: None,
        water: &|_| 0.0,
        depth_m: 4.0,
        bottom_m: 44.0,
        side_slope: 3.0,
        widening: &widen,
        river: Some(&water),
        wet_weight: 0.0,
        dry_weight: 0.0,
        land_weight: 1.0,
    };
    let mut p = params(r_min, loa, true);
    p.shift.as_mut().unwrap().cut_room_m = 88.0;
    let index = linear::ProjectIndex::new(&river);
    let river_at = |q: Pt| index.project(q);
    let t0 = std::time::Instant::now();
    let out = run(
        &river,
        Some(&mask),
        &p,
        &scorer,
        &widen,
        &BTreeMap::new(),
        &river_at,
    );
    let dt = t0.elapsed().as_secs_f64();
    let (st, sh) = out.shift.as_ref().unwrap();
    let calls: usize = st.iter().map(|s| (s.hi - s.lo + 1) as usize).sum::<usize>() * 4;
    eprintln!(
        "long river {:.1} km: {:.1} s; seed {} elements, {} cut-through candidates, {} stations, \
         {} table calls, cost {:.0} -> {:.0}, delivered {} elements, gap {:.2} m",
        linear::length(&river) / 1000.0,
        dt,
        out.seed.chain.elements.len(),
        out.choices.len(),
        st.len(),
        calls,
        sh.cost_zero,
        sh.cost,
        out.line.elements.len(),
        out.tracking_gap_m
    );
    gap_report(&out);
    assert!(out.line.least_radius() >= r_min - 1e-9);
}

/// Where the rebuilt line is farthest from the shifted target (a diagnostic for the timing run).
fn gap_report(out: &owd_core::alignment::river_following::Outcome) {
    if let Some(rb) = &out.rebuild {
        let g: Vec<f64> = owd_core::alignment::river_following::elements::walk_distances(
            &rb.chain,
            &rb.target.pts,
        )
        .into_iter()
        .map(|x| x.0)
        .collect();
        let bad: Vec<usize> = (0..g.len()).filter(|&i| g[i] > 5.0).collect();
        eprintln!("points over 5 m: {} of {}", bad.len(), g.len());
        if let Some(&m) = bad.iter().max_by(|a, b| g[**a].total_cmp(&g[**b])) {
            eprintln!(
                "worst at target s {:.0} kappa {:.5} gap {:.1}",
                rb.target.s[m], rb.target.kappa[m], g[m]
            );
            for c in rb
                .circles
                .iter()
                .filter(|c| c.a <= m + 200 && c.b + 200 >= m)
            {
                eprintln!(
                    "  circle r {:.0} a {} b {} tight {} moved {}",
                    c.r, c.a, c.b, c.tight, c.moved
                );
            }
        }
    }
}
