//! Cut-through candidates of the river-following line (spec OPT-4, part 2), on synthetic rivers.

use owd_core::alignment::river_following::cuts::{CutParams, candidates, splice};
use owd_core::alignment::river_following::fixtures;
use owd_core::alignment::river_following::seed::{SeedParams, seed};
use owd_core::geom::linear;

const R_MIN: f64 = 300.0;
const LOA: f64 = 75.0;

fn cut_params() -> CutParams {
    CutParams::scaled(R_MIN, 1.0556 * R_MIN, 2.0 * LOA, 1.3, 100.0, LOA)
}

#[test]
fn an_omega_loop_has_a_cut_through_across_its_neck() {
    let river = fixtures::omega(320.0, 115f64.to_radians(), 1500.0);
    let p = SeedParams::scaled(R_MIN, 1.0333, LOA, 2.0);
    let s = seed(&river, &p, None);
    let cp = cut_params();
    let river_at = |q: [f64; 2]| linear::project(&river, q);
    let c = candidates(&s.chain, &cp, &river_at);
    for x in &c {
        eprintln!(
            "{} s {:.0}-{:.0} river {:.0}-{:.0} stay {:.0} cut {:.0} saved {:.0} ratio {:.2} options {}",
            x.id,
            x.s_from,
            x.s_to,
            x.river_from,
            x.river_to,
            x.stay_length_m,
            x.first().length_m,
            x.saved_m(0),
            x.ratio(0),
            x.options.len()
        );
    }
    assert!(!c.is_empty());
    // The best candidate crosses the neck: it saves most of the loop.
    let best = c
        .iter()
        .max_by(|a, b| a.saved_m(0).total_cmp(&b.saved_m(0)))
        .unwrap();
    assert!(best.saved_m(0) > 900.0, "{}", best.saved_m(0));
    assert!(best.ratio(0) >= 1.3);
    // Each candidate holds the rules: arcs at the cut radius, tangent joins, no short reversal,
    // R_min on the spliced line.
    for x in &c {
        for o in &x.options {
            let line = splice(&s.chain, x.s_from, x.s_to, &o.pieces);
            let (th, pos) = line.largest_join_jump();
            assert!(th < 1e-9 && pos < 1e-6, "{}: join {th} {pos}", x.id);
            assert!(line.least_radius() >= cp.r_cut.min(p.r_clip) - 1e-9);
            assert!(
                line.short_reversals(cp.tight_reverse_r, cp.min_straight_m)
                    .is_empty(),
                "{}",
                x.id
            );
            // The spliced line ends where the seed ends.
            let (a, b) = (line.end(), s.chain.end());
            assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6);
        }
    }
    // The ids are stable: the same river gives the same ids.
    let again = candidates(&seed(&river, &p, None).chain, &cp, &river_at);
    let ids = |v: &[owd_core::alignment::river_following::cuts::Candidate]| {
        v.iter().map(|x| x.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(&c), ids(&again));
}

#[test]
fn a_gentle_river_has_no_cut_through() {
    let river = fixtures::sine(200.0, 2400.0, 12000.0);
    let p = SeedParams::scaled(R_MIN, 1.0333, LOA, 2.0);
    let s = seed(&river, &p, None);
    let river_at = |q: [f64; 2]| linear::project(&river, q);
    assert!(candidates(&s.chain, &cut_params(), &river_at).is_empty());
}

mod choice {
    use super::*;
    use owd_core::alignment::river_following::choice::{Choice, ChoiceParams, DecidedBy, choose};
    use owd_core::alignment::river_following::mask::RiverMask;
    use owd_core::alignment::river_following::score::PlainScorer;
    use std::collections::BTreeMap;

    /// R4's river: the omega loop, 60 m wide, 1 m deep, the land `h` metres above the water.
    struct R4 {
        river: Vec<[f64; 2]>,
        mask: RiverMask,
        /// The neck: the closest two points of the river a loop apart.
        neck: ([f64; 2], [f64; 2]),
        /// The river grown to 150 m from its centreline.
        near: RiverMask,
    }

    fn r4() -> R4 {
        let river = fixtures::omega(320.0, 115f64.to_radians(), 1500.0);
        let poly = fixtures::polygon(&river, 60.0);
        let mut neck = (f64::INFINITY, [0.0; 2], [0.0; 2]);
        for i in (0..river.len()).step_by(5) {
            for j in (i + 1200..river.len()).step_by(5) {
                let d = libm::hypot(river[i][0] - river[j][0], river[i][1] - river[j][1]);
                if d < neck.0 {
                    neck = (d, river[i], river[j]);
                }
            }
        }
        R4 {
            near: RiverMask::new(std::slice::from_ref(&poly), 120.0),
            mask: RiverMask::new(&[poly], 0.0),
            neck: (neck.1, neck.2),
            river,
        }
    }

    /// In the neck: at least 150 m from the river (so staying in it does not bite the hill) and
    /// within 300 m of the line across the neck.
    fn in_neck(r: &R4, q: [f64; 2]) -> bool {
        let (a, b) = r.neck;
        linear::point_segment_distance(q, a, b) < 300.0 && !r.near.contains(q)
    }

    fn decide(r: &R4, h: f64, force: &BTreeMap<String, bool>) -> Vec<Choice> {
        let p = SeedParams::scaled(R_MIN, 1.0333, LOA, 2.0);
        let s = seed(&r.river, &p, None);
        let river_at = |q: [f64; 2]| linear::project(&r.river, q);
        let c = candidates(&s.chain, &cut_params(), &river_at);
        // The river 1 m deep; the land 1 m above the water, but `h` in the neck: the middle half of
        // the line across it, 120 m either side.
        let ground = |q: [f64; 2]| {
            if r.mask.contains(q) {
                -1.0
            } else if in_neck(r, q) {
                h
            } else {
                1.0
            }
        };
        let widen = |radius: f64| {
            if radius < 5.0 * R_MIN {
                10.0 * R_MIN / radius
            } else {
                0.0
            }
        };
        let scorer = PlainScorer {
            ground: Some(&ground),
            water: &|_| 0.0,
            depth_m: 4.0,
            bottom_m: 30.0,
            side_slope: 3.0,
            widening: &widen,
            river: Some(&r.mask),
            wet_weight: 1.0,
            dry_weight: 1.0,
            land_weight: 0.0,
        };
        let cp = ChoiceParams {
            min_save_m: 2.0 * LOA,
            min_gain: 0.05,
            face_max_m: 30.0,
            step_m: 25.0,
        };
        choose(&s.chain, c, &scorer, force, &cp).0
    }

    fn neck(v: &[Choice]) -> &Choice {
        v.iter()
            .max_by(|a, b| a.candidate.saved_m(0).total_cmp(&b.candidate.saved_m(0)))
            .unwrap()
    }

    /// R4: low ground in the neck, the cut-through is taken; high ground, it is refused, with both
    /// sets of numbers; a sweep of the height changes the decision once.
    #[test]
    fn r4_the_cut_through_is_taken_on_low_ground_and_refused_on_high() {
        let r = r4();
        let none = BTreeMap::new();
        let low = decide(&r, 1.0, &none);
        let n = neck(&low);
        eprintln!(
            "h 1: {} stay {:.0} cut {:.0} accepted {} {:?}",
            n.candidate.id, n.stay.cost, n.options[n.best].cost, n.accepted, n.refused_because
        );
        assert!(n.passes && n.accepted, "{:?}", n.refused_because);
        assert_eq!(n.decided_by, DecidedBy::Rule);
        let high = decide(&r, 40.0, &none);
        let n = neck(&high);
        eprintln!(
            "h 40: {} stay {:.0} cut {:.0} face {:.1} {:?}",
            n.candidate.id,
            n.stay.cost,
            n.options[n.best].cost,
            n.options[n.best].face_max_m,
            n.refused_because
        );
        assert!(!n.accepted && n.refused_because.is_some());
        assert!(n.stay.cost > 0.0 && n.options[n.best].cost > n.stay.cost);
        // The sweep: the neck's decision changes once.
        let mut flips = 0;
        let mut last = true;
        for h in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, 40.0] {
            let now = neck(&decide(&r, h, &none)).accepted;
            if now != last {
                flips += 1;
            }
            last = now;
        }
        assert_eq!(flips, 1);
    }

    /// R5: the force list wins over the rule, says so, and the id holds over a re-run.
    #[test]
    fn r5_a_forced_cut_through_wins_over_the_rule() {
        let r = r4();
        let id = neck(&decide(&r, 1.0, &BTreeMap::new()))
            .candidate
            .id
            .clone();
        let off = decide(&r, 1.0, &BTreeMap::from([(id.clone(), false)]));
        let n = neck(&off);
        assert_eq!(n.candidate.id, id);
        assert!(n.passes && !n.accepted);
        assert_eq!(n.decided_by, DecidedBy::ForcedOff);
        let on = decide(&r, 40.0, &BTreeMap::from([(id.clone(), true)]));
        let n = neck(&on);
        assert_eq!(n.candidate.id, id);
        assert!(!n.passes && n.accepted);
        assert_eq!(n.decided_by, DecidedBy::ForcedOn);
    }
}
