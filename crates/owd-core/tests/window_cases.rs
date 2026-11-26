//! The window optimiser on the synthetic rivers of OPT-5 section 5, at a coarse lattice so a debug
//! build runs them in seconds (the full lattice is measured in release by the `window_measure`
//! example, and the numbers are in the change log).
//!
//! Label: property

use owd_core::alignment::loops::cumulative_turn_deg;
use owd_core::geom::overlay::PolygonIndex;
use owd_core::optimise::window::line::radius_reading;
use owd_core::optimise::window::run::{
    Adopted, WindowConfig, WindowInputs, WindowsResult, optimise_windows,
};
use owd_core::optimise::window::scorer::HalfWidthRule;
use owd_core::optimise::window::seams::SeamKind;
use owd_core::synthetic::window::{self as fx, WindowFixture};

const LOA: f64 = 75.0;
const BW: f64 = 30.0;

fn rule(r_min: f64) -> HalfWidthRule {
    HalfWidthRule::new(0.5 * BW, 2000.0, move |r: f64| {
        0.6 * LOA * LOA / r.max(r_min)
    })
}

/// The defaults, coarser: 32 headings, a 16 m pitch, 6 radii, a 4 m grid, sections every 12 m and
/// samples every 4 m in the search, the measure's sections 120 m wide every 2 m, two polish
/// spacings.
fn coarse(r_min: f64) -> WindowConfig {
    let mut c = WindowConfig::defaults(r_min, LOA, BW, 4.0);
    c.lattice.n_heading = 32;
    c.lattice.pitch_m = 16.0;
    c.lattice.straight_len_m = Some(32.0);
    c.lattice.radii_m = Some(owd_core::optimise::window::run::geometric_radii(
        r_min, 6, 8.0,
    ));
    c.lattice.goal_pos_tol_m = Some(8.0);
    c.lattice = c.lattice.validated().unwrap();
    c.cell_m = 4.0;
    c.search_dx_m = 4.0;
    c.search_ds_m = 12.0;
    c.section.maxw_m = 120.0;
    c.section.dx_m = 2.0;
    c.corridor_half_m = 250.0;
    if let Some(p) = c.polish.as_mut() {
        p.spacings_m = vec![r_min / 2.4, r_min / 1.44];
        p.station_m = 30.0;
    }
    c.window_time_s = 1.0e9;
    c
}

fn run_with(
    f: &WindowFixture,
    cfg: &WindowConfig,
    weirs: Vec<f64>,
    plan_area: bool,
) -> WindowsResult {
    let index = PolygonIndex::new(&f.river);
    let sampler = f.sampler();
    let flat = |xs: &[f64], _: &[f64]| vec![-1000.0; xs.len()];
    let level = |_: f64| fx::WATER_LEVEL_M;
    let clock = || 0.0;
    let never = || false;
    let r = rule(cfg.lattice.r_min_m);
    let inp = WindowInputs {
        line: &f.start_line,
        river: &f.river,
        river_index: &index,
        centreline: Some(&f.centreline),
        terrain: if plan_area { &flat } else { &sampler },
        plan_area,
        level: &level,
        rule: &r,
        weirs_m: weirs,
        cut_throughs_m: vec![],
        forced_stretches_m: vec![],
        permitted_m: vec![],
        frozen_m: vec![],
        clock: &clock,
        cancel: &never,
    };
    optimise_windows(&inp, cfg).unwrap()
}

fn run_permitted(
    f: &WindowFixture,
    cfg: &WindowConfig,
    permitted: Vec<(f64, f64, f64)>,
) -> WindowsResult {
    let index = PolygonIndex::new(&f.river);
    let sampler = f.sampler();
    let level = |_: f64| fx::WATER_LEVEL_M;
    let clock = || 0.0;
    let never = || false;
    let r = rule(cfg.lattice.r_min_m);
    let inp = WindowInputs {
        line: &f.start_line,
        river: &f.river,
        river_index: &index,
        centreline: Some(&f.centreline),
        terrain: &sampler,
        plan_area: false,
        level: &level,
        rule: &r,
        weirs_m: vec![],
        cut_throughs_m: vec![],
        forced_stretches_m: vec![],
        permitted_m: permitted,
        frozen_m: vec![],
        clock: &clock,
        cancel: &never,
    };
    optimise_windows(&inp, cfg).unwrap()
}

fn run(f: &WindowFixture, cfg: &WindowConfig) -> WindowsResult {
    run_with(f, cfg, vec![], false)
}

/// S3: a straight reach whose bed is below grade: nothing to gain, the line comes back bit for bit
/// and every window says why.
#[test]
fn s3_a_straight_reach_comes_back_bit_for_bit() {
    let f = fx::straight_reach(3000.0, 80.0, fx::BANK_M);
    let out = run(&f, &coarse(300.0));
    assert_eq!(out.line, f.start_line);
    assert!(out.windows.len() >= 2);
    for w in &out.windows {
        assert_eq!(w.adopted, Adopted::None, "{w:?}");
        assert!(
            matches!(w.reason, "under_threshold" | "dearer"),
            "{}",
            w.reason
        );
    }
    assert!(out.end_to_end_saving.abs() <= 0.005 * out.start_total.cost.max(1.0));
}

/// S5 (b): a starting line that humps 30 m sideways into a 5 m bank: a window adopts a cheaper
/// line, and the whole line is cheaper end to end.
#[test]
fn s5_a_hump_into_a_bank_is_taken_out() {
    let f = fx::hump_reach(3000.0, 80.0, 30.0, 800.0, 15.0);
    let out = run(&f, &coarse(300.0));
    assert!(
        out.windows.iter().any(|w| w.adopted != Adopted::None),
        "{:?}",
        out.windows
            .iter()
            .map(|w| (w.reason, w.gain_pct))
            .collect::<Vec<_>>()
    );
    assert!(out.result_total.cost < 0.9 * out.start_total.cost);
    assert!(out.radius_ok);
    assert!(out.self_crossing_m.is_none());
}

/// S2: a bend against a spur: the line moves away from the spur, much cheaper, holding R_min, no
/// loop, joined without a gap at every seam; and the same with the fixture mirrored, and on four
/// threads (S10).
#[test]
fn s2_a_bend_against_a_spur_moves_away_from_it() {
    let r_min = 300.0;
    let mut cfg = coarse(r_min);
    cfg.window_max_m = 4000.0;
    let f = fx::bend_against_spur(250.0, 60.0, 800.0, r_min);
    let out = run(&f, &cfg);
    let found: Vec<_> = out
        .windows
        .iter()
        .filter(|w| w.adopted == Adopted::Found)
        .collect();
    assert!(!found.is_empty(), "{:?}", out.windows);
    assert!(
        out.result_total.cost <= 0.8 * out.start_total.cost,
        "{} -> {}",
        out.start_total.cost,
        out.result_total.cost
    );
    let (r, ok) = radius_reading(&out.line, r_min);
    assert!(ok, "{r}");
    assert!(out.self_crossing_m.is_none());
    for w in &out.windows {
        assert!(w.seam_gap_m <= 1e-6, "{}", w.seam_gap_m);
        assert!(w.seam_heading_gap_rad <= 1e-6);
        assert!(w.full_found.is_none_or(|m| m.cost.is_finite()));
    }
    assert_eq!(out.line[0], f.start_line[0]);
    assert_eq!(
        out.line[out.line.len() - 1],
        f.start_line[f.start_line.len() - 1]
    );
    // Never dearer, window by window.
    for w in &out.windows {
        if let (Adopted::Found, Some(m)) = (w.adopted, w.full_found) {
            assert!(m.cost <= w.full_start.cost);
        }
    }

    // S10: mirrored, the same costs; on four threads, the same line.
    let m = run(
        &fx::bend_against_spur(250.0, 60.0, 800.0, r_min).mirrored(),
        &cfg,
    );
    assert!(
        (m.result_total.cost - out.result_total.cost).abs()
            <= 1e-9 * out.result_total.cost.max(1.0),
        "{} against {}",
        m.result_total.cost,
        out.result_total.cost
    );
    for (p, q) in out.line.iter().zip(&m.line) {
        assert!((p[0] - q[0]).abs() <= 1e-6 && (p[1] + q[1]).abs() <= 1e-6);
    }
    let mut four = cfg.clone();
    four.threads = 4;
    let t = run(&f, &four);
    assert_eq!(t.line, out.line);
    assert_eq!(t.result_total, out.result_total);
}

/// S6 (c): deep water beside the river, with no length or land cost: water costs nothing either
/// way, and still the line does not cross itself and no window turns more than three turns.
#[test]
fn s6_deep_water_does_not_make_a_loop() {
    let mut f = fx::straight_reach(3000.0, 80.0, fx::BANK_M);
    f.ground = Box::new(|_, y| {
        if y.abs() < 400.0 {
            fx::BED_M - 5.0
        } else {
            fx::BANK_M
        }
    });
    let out = run(&f, &coarse(300.0));
    assert!(out.self_crossing_m.is_none());
    for w in &out.windows {
        if let Some(l) = w.loop_check {
            assert!(l.turn_deg <= 1080.0 || w.adopted == Adopted::None);
        }
    }
    assert!(cumulative_turn_deg(&out.line) <= 1080.0);
}

/// S8: a weir at the middle of a straight reach is a forced seam, no window crosses it, and the
/// line is straight for the weir straight through it.
#[test]
fn s8_a_weir_is_a_seam_the_line_runs_straight_through() {
    // The hump 300 m downstream of the weir: the starting line is straight through the weir.
    let mut f = fx::hump_reach(4000.0, 80.0, 30.0, 800.0, 15.0);
    f.start_line = f
        .start_line
        .iter()
        .map(|p| {
            let t = (p[0] - 2300.0) / 800.0;
            let d = if (0.0..=1.0).contains(&t) {
                15.0 * (1.0 - libm::cos(2.0 * std::f64::consts::PI * t))
            } else {
                0.0
            };
            [p[0], d]
        })
        .collect();
    let cfg = coarse(300.0);
    let out = run_with(&f, &cfg, vec![2000.0], false);
    assert!(
        out.windows.iter().any(|w| w.adopted != Adopted::None),
        "{:?}",
        out.windows
    );
    assert!(
        out.seams
            .iter()
            .any(|s| s.kind == SeamKind::Weir && (s.at_m - 2000.0).abs() < 1e-9)
    );
    for w in &out.windows {
        // On the first pass's line the weir has not moved: no window holds it inside.
        assert!(
            !(w.start_m < 2000.0 - 1e-6 && w.end_m > 2000.0 + 1e-6),
            "window {}-{} crosses the weir",
            w.start_m,
            w.end_m
        );
    }
    let len = owd_core::optimise::lattice::straight_through_m(&out.line, 2000.0);
    assert!(len >= cfg.weir_straight_m - 1.0, "{len}");
}

/// S7: with no ground model the windows run on plan area: no line takes more land than the
/// starting line, and no volume is cut.
#[test]
fn s7_with_no_ground_the_windows_run_on_plan_area() {
    let r_min = 300.0;
    let mut cfg = coarse(r_min);
    cfg.window_max_m = 4000.0;
    cfg.weights.land = 1.0;
    let f = fx::bend_against_spur(250.0, 60.0, 800.0, r_min);
    let out = run_with(&f, &cfg, vec![], true);
    assert!(out.result_total.land_m2 <= out.start_total.land_m2 + 1e-6);
    assert_eq!(out.result_total.total_m3(), 0.0);
    assert!(out.radius_ok);
}

/// S13 and S14: a window that hits the expansion cap keeps its start and says `cap`; a line
/// shorter than the shortest window is one window; never a window of no length.
#[test]
fn s13_s14_a_capped_window_and_a_short_line() {
    let f = fx::hump_reach(3000.0, 80.0, 30.0, 800.0, 15.0);
    let mut cfg = coarse(300.0);
    cfg.lattice.max_expand = 50;
    let out = run(&f, &cfg);
    assert_eq!(out.line, f.start_line);
    assert!(
        out.windows.iter().all(|w| w.adopted == Adopted::None)
            && out
                .windows
                .iter()
                .any(|w| w.reason == "cap" && w.expansions == 50),
        "{:?}",
        out.windows
            .iter()
            .map(|w| (w.reason, w.expansions))
            .collect::<Vec<_>>()
    );

    let short = fx::straight_reach(500.0, 80.0, fx::BANK_M);
    let out = run(&short, &coarse(300.0));
    assert_eq!(out.windows.len(), 1);
    assert!(out.windows[0].end_m - out.windows[0].start_m > 400.0);
}

/// S14: a reach where no seam can stand on a straight in the river: the seams are placed anyway and
/// the record says so (`forced_off_river`).
#[test]
fn s14_an_empty_band_places_seams_and_says_so() {
    // An omega loop of R_min arcs with short arms: no seam can stand on a straight inside the band.
    let f = fx::omega_loop(300.0, 60.0, 100.0, 2.0);
    let mut cfg = coarse(300.0);
    cfg.window_max_m = 1500.0;
    cfg.window_target_m = 1200.0;
    cfg.lattice.max_expand = 1; // the search is not what this test is about
    cfg.second_pass = false;
    let out = run(&f, &cfg);
    assert!(
        out.seams.iter().any(|s| s.kind == SeamKind::ForcedOffRiver),
        "{:?}",
        out.seams
    );
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("no seam in the band"))
    );
    for w in &out.windows {
        assert!(w.end_m - w.start_m >= cfg.window_min_m - 1e-6 || w.end_m - w.start_m > 0.0);
    }
}

/// S9: a river S over a drowned floodplain (ground below grade beside the river, so cutting it is
/// free) with a cost per metre of line. Free, the line leaves the river to shorten the S; held to
/// the river (`stay_in_river`), no point of it leaves the river (to the grid's cell).
#[test]
fn s9_held_to_the_river_the_line_makes_no_new_departure() {
    let r_min = 300.0;
    let mut f = fx::long_s(700.0, 1.0, 120.0, 300.0, 0.0);
    f.ground = Box::new(|_, _| fx::BED_M);
    let mut cfg = coarse(r_min);
    cfg.window_max_m = 4000.0;
    cfg.weights.length = 50.0;
    cfg.polish = None;
    let free = run(&f, &cfg);
    let outside = |line: &[[f64; 2]]| {
        line.iter()
            .map(|&p| owd_core::geom::overlay::point_polygon_distance(p, &f.river))
            .fold(0.0, f64::max)
    };
    assert!(outside(&free.line) > 20.0, "free: {}", outside(&free.line));
    cfg.stay_in_river = true;
    let held = run(&f, &cfg);
    assert!(
        outside(&held.line) <= cfg.cell_m,
        "held: {}",
        outside(&held.line)
    );
    // A permitted stretch of the starting line (a recorded cut-through, or a given line's own
    // off-river stretch) opens its room to the line again.
    let len = owd_core::geom::linear::length(&f.start_line);
    let opened = run_permitted(&f, &cfg, vec![(0.0, len, 400.0)]);
    assert!(
        outside(&opened.line) > 20.0,
        "opened: {}",
        outside(&opened.line)
    );
}
