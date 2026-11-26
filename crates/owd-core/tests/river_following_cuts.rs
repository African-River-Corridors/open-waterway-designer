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
