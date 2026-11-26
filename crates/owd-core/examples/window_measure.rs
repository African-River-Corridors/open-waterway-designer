//! The window optimiser (OPT-5) on the synthetic rivers: what it saves and what it costs to run.
//! Release build: `cargo run --release -p owd-core --example window_measure -- [case] [args]`.
//!
//! Cases: `s2` (bend against a spur), `s1 H` (omega loop, hill H m), `s3` (straight reach),
//! `budget R_MIN` (one window of 0.5, 1, 1.5, 2 and 4 km on a sine river: states, expansions, time),
//! `river KM` (a whole sine river end to end, in parallel).

use owd_core::geom::overlay::PolygonIndex;
use owd_core::optimise::window::run::{WindowConfig, WindowInputs, optimise_windows};
use owd_core::optimise::window::scorer::HalfWidthRule;
use owd_core::synthetic::window::{self as fx, WindowFixture};
use std::time::Instant;

fn rule(bw: f64, loa: f64, r_min: f64) -> HalfWidthRule {
    // A widening of 0.6 LOA^2 / R (floored at R_min), none at 2,000 m and above.
    HalfWidthRule::new(0.5 * bw, 2000.0, move |r: f64| {
        0.6 * loa * loa / r.max(r_min)
    })
}

fn run(name: &str, f: &WindowFixture, cfg: &WindowConfig, rule: &HalfWidthRule) {
    let t = Instant::now();
    let clock = move || t.elapsed().as_secs_f64();
    let never = || false;
    let level = |_: f64| fx::WATER_LEVEL_M;
    let index = PolygonIndex::new(&f.river);
    let sampler = f.sampler();
    let inp = WindowInputs {
        line: &f.start_line,
        river: &f.river,
        river_index: &index,
        centreline: Some(&f.centreline),
        terrain: &sampler,
        plan_area: false,
        level: &level,
        rule,
        weirs_m: vec![],
        cut_throughs_m: vec![],
        forced_stretches_m: vec![],
        frozen_m: vec![],
        clock: &clock,
        cancel: &never,
    };
    let out = optimise_windows(&inp, cfg).expect("runs");
    println!(
        "{name}: {:.1} s, start {:.0} -> result {:.0} ({:+.2} %), plain m3 {:.0} -> {:.0}, R {:.0} ok {}, crossing {:?}, in flight {}",
        out.seconds,
        out.start_total.cost,
        out.result_total.cost,
        100.0 * (out.result_total.cost / out.start_total.cost - 1.0),
        out.start_total.total_m3(),
        out.result_total.total_m3(),
        out.min_radius_m,
        out.radius_ok,
        out.self_crossing_m,
        out.windows_in_flight
    );
    for w in &out.windows {
        println!(
            "  w{} p{} {:.0}-{:.0} m: search {:.0} -> {:?}, full {:.0} -> {:?} ({:?} %), {} ({}), exp {}, states {}, {:.1} s, face {:.1}->{:?}",
            w.id,
            w.pass,
            w.start_m,
            w.end_m,
            w.search_cost_start,
            w.search_cost_found.map(|c| c.round()),
            w.full_start.cost,
            w.full_found.map(|m| m.cost.round()),
            w.gain_pct.map(|g| (g * 100.0).round() / 100.0),
            w.adopted.name(),
            w.reason,
            w.expansions,
            w.states,
            w.seconds,
            w.full_start.max_face_h_m,
            w.full_found.map(|m| (m.max_face_h_m * 10.0).round() / 10.0),
        );
    }
    for wn in &out.warnings {
        println!("  warning: {wn}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let case = args.get(1).map_or("s2", String::as_str);
    let (loa, bw, depth) = (75.0, 30.0, 4.0);
    match case {
        "s2" => {
            let r_min = 300.0;
            let f = fx::bend_against_spur(250.0, 60.0, 800.0, r_min);
            let mut cfg = WindowConfig::defaults(r_min, loa, bw, depth);
            cfg.window_max_m = 4000.0;
            run("S2", &f, &cfg, &rule(bw, loa, r_min));
        }
        "s3" => {
            let r_min = 300.0;
            let f = fx::straight_reach(3000.0, 80.0, fx::BANK_M);
            let cfg = WindowConfig::defaults(r_min, loa, bw, depth);
            run("S3", &f, &cfg, &rule(bw, loa, r_min));
        }
        "s1" => {
            let r_min = 300.0;
            let h: f64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(2.0);
            let f = fx::omega_loop(r_min, 60.0, 600.0, h);
            let mut cfg = WindowConfig::defaults(r_min, loa, bw, depth);
            cfg.window_max_m = 6000.0;
            cfg.weights.length = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(100.0);
            run(&format!("S1 h={h}"), &f, &cfg, &rule(bw, loa, r_min));
        }
        "river" => {
            let km: f64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(30.0);
            let r_min = 180.0;
            let f = fx::sine_river(km * 1000.0, 1600.0, 220.0, 60.0);
            let mut cfg = WindowConfig::defaults(r_min, 60.0, 44.0, depth);
            cfg.threads = std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(8);
            run(
                &format!("river {km} km"),
                &f,
                &cfg,
                &rule(44.0, 60.0, r_min),
            );
        }
        "budget" => {
            // One window of the given length on a gentle sine river: the search's states,
            // expansions and seconds, and the saving.
            let km: f64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(1.0);
            let r_min: f64 = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(180.0);
            let f = fx::sine_river(km * 1000.0, 2000.0, 150.0, 60.0);
            let mut cfg = WindowConfig::defaults(r_min, 60.0, 44.0, depth);
            cfg.window_target_m = km * 1000.0;
            cfg.window_min_m = cfg.window_min_m.min(km * 1000.0);
            cfg.window_max_m = cfg.window_max_m.max(km * 1000.0 + 1.0);
            cfg.second_pass = false;
            cfg.window_time_s = 3600.0;
            if args.get(4).is_some_and(|a| a == "nopolish") {
                cfg.polish = None;
            }
            run(
                &format!("budget {km} km R_min {r_min}"),
                &f,
                &cfg,
                &rule(44.0, 60.0, r_min),
            );
        }
        other => eprintln!("unknown case {other}"),
    }
}
