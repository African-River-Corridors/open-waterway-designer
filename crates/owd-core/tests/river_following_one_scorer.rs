//! The river-following line costs with the one scorer (OPT-5 part 1): its stations are the volumes
//! stage's own sections, and the cut-through choice of R4 holds with them.
//!
//! Label: property

use owd_core::alignment::river_following::choice::{ChoiceParams, DecidedBy, choose};
use owd_core::alignment::river_following::cuts::{CutParams, candidates};
use owd_core::alignment::river_following::score::{OneScorer, totals};
use owd_core::alignment::river_following::seed::{SeedParams, seed};
use owd_core::cut_model::CUT_MODEL;
use owd_core::geom::overlay::PolygonIndex;
use owd_core::geom::{Pt, linear};
use owd_core::optimise::window::scorer::{
    Frame, HalfWidthRule, LevelAt, LineMeasure, SectionSettings, Weights,
};
use owd_core::synthetic::window as fx;
use owd_core::volumes::sections::SectionCutter;
use std::collections::BTreeMap;

const R_MIN: f64 = 300.0;
const LOA: f64 = 75.0;

fn rule() -> HalfWidthRule {
    HalfWidthRule::new(15.0, 2000.0, |r: f64| 0.6 * LOA * LOA / r.max(R_MIN))
}

fn sec() -> SectionSettings {
    SectionSettings {
        model: CUT_MODEL.into(),
        depth_m: 4.0,
        wet_slope: 3.0,
        maxw_m: 200.0,
        dx_m: 1.0,
        daylight_run_m: None,
    }
}

/// The seed of a river costed station by station agrees with the volumes walk of the same line.
#[test]
fn a_station_is_the_volumes_stages_section() {
    let f = fx::omega_loop(R_MIN, 60.0, 600.0, 12.0);
    let index = PolygonIndex::new(&f.river);
    let ground = f.sampler();
    let rule = rule();
    let cutter = SectionCutter::new(&index, 4.0, CUT_MODEL, 3.0, 200.0, 1.0, None).unwrap();
    let m = LineMeasure::new(
        cutter,
        &ground,
        &rule,
        LevelAt::Fixed(fx::WATER_LEVEL_M),
        Weights::default(),
        &sec(),
        25.0,
    );
    let s = seed(
        &f.centreline,
        &SeedParams::scaled(R_MIN, 1.0333, LOA, 2.0),
        None,
    );
    let line = s.chain.points();
    let walk = m.measure(&line, Frame::FromStart);
    let one = OneScorer::new(&m);
    let t = totals(&s.chain, 0.0, s.chain.length(), 25.0, &one, 30.0);
    let rel = (t.cost - walk.cost).abs() / walk.cost;
    eprintln!(
        "stations {:.0} against the walk {:.0}: {:.2} %",
        t.cost,
        walk.cost,
        100.0 * rel
    );
    assert!(rel < 0.03, "{rel}");
}

/// R4's choice with the one scorer, on OPT-5's omega loop (a river already 4.5 m deep, so staying
/// is cheap): both sets of numbers come from the volumes stage's section, and a 40 m neck is
/// refused. (The low-neck case is R4 in `river_following_cuts`, on a shallow river.)
#[test]
fn r4_with_the_one_scorer() {
    let rule = rule();
    let mut out = Vec::new();
    for hill in [1.5, 40.0] {
        let f = fx::omega_loop(R_MIN, 60.0, 600.0, hill);
        let index = PolygonIndex::new(&f.river);
        let ground = f.sampler();
        let cutter = SectionCutter::new(&index, 4.0, CUT_MODEL, 3.0, 200.0, 2.0, None).unwrap();
        let m = LineMeasure::new(
            cutter,
            &ground,
            &rule,
            LevelAt::Fixed(fx::WATER_LEVEL_M),
            Weights::default(),
            &sec(),
            25.0,
        );
        let one = OneScorer::new(&m);
        let s = seed(
            &f.centreline,
            &SeedParams::scaled(R_MIN, 1.0333, LOA, 2.0),
            None,
        );
        let river_at = |q: Pt| linear::project(&f.centreline, q);
        let c = candidates(
            &s.chain,
            &CutParams::scaled(R_MIN, 1.0556 * R_MIN, 2.0 * LOA, 1.3, 100.0, LOA),
            &river_at,
        );
        let cp = ChoiceParams {
            min_save_m: 2.0 * LOA,
            min_gain: 0.05,
            face_max_m: 30.0,
            step_m: 25.0,
        };
        let (ch, _, _) = choose(&s.chain, c, &one, &BTreeMap::new(), &cp);
        let best = ch
            .iter()
            .max_by(|a, b| a.candidate.saved_m(0).total_cmp(&b.candidate.saved_m(0)))
            .unwrap()
            .clone();
        eprintln!(
            "hill {hill}: {} stay {:.0} cut {:.0} face {:.1} accepted {} {:?}",
            best.candidate.id,
            best.stay.cost,
            best.options[best.best].cost,
            best.options[best.best].face_max_m,
            best.accepted,
            best.refused_because
        );
        assert_eq!(best.decided_by, DecidedBy::Rule);
        assert!(best.stay.cost > 0.0 && best.options[best.best].cost > 0.0);
        out.push((best.accepted, best.options[best.best].face_max_m));
    }
    assert!(!out[1].0);
    assert!(out[1].1 > out[0].1);
}
