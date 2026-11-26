//! The river-following line's seed (spec OPT-4, part 1) on synthetic rivers: tests R1, R2, R3, R9
//! of the spec, and the radius check on fine and coarse chords.

use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::alignment::river_following::elements::Chain;
use owd_core::alignment::river_following::fixtures;
use owd_core::alignment::river_following::seed::{Seed, SeedParams, seed};
use owd_core::geom::{Pt, linear};

const R_MIN: f64 = 300.0;
const LOA: f64 = 75.0;

fn params(r_min: f64, loa: f64) -> SeedParams {
    SeedParams::scaled(r_min, 1.0333, loa, 2.0)
}

/// The largest distance from any point of `river` to the seed's dense line.
fn largest_distance(chain: &Chain, river: &[Pt]) -> f64 {
    let line = chain.points();
    river
        .iter()
        .map(|p| linear::point_line_distance(*p, &line))
        .fold(0.0, f64::max)
}

fn report(name: &str, s: &Seed) {
    eprintln!(
        "{name}: {} elements ({} arcs), gap {:.2} m (all {:.2}), passes {}, tight {}",
        s.chain.elements.len(),
        s.chain.elements.iter().filter(|e| e.is_arc()).count(),
        s.gap_m,
        s.gap_all_m,
        s.passes,
        s.tight.len()
    );
}

/// The guarantees every seed holds: no arc under R_clip, tangent joins, R_min on the chord check
/// on fine and coarse chords, the minimum straight between tight opposite arcs.
fn guarantees(s: &Seed, p: &SeedParams) {
    let (th, pos) = s.chain.largest_join_jump();
    assert!(th <= 1e-9 && pos <= 1e-9, "join jump {th} rad, {pos} m");
    assert!(
        s.chain.least_radius() >= p.r_clip - 1e-9,
        "least radius {}",
        s.chain.least_radius()
    );
    let line = s.chain.points();
    for chord in [p.r_min / 3.0, p.r_min / 10.0, p.r_min] {
        let r = min_radius_on_chord(&line, chord).unwrap();
        assert!(radius_ok(r, p.r_min), "chord {chord}: radius {r}");
        assert!(r >= p.r_min, "chord {chord}: radius {r} under R_min");
    }
    assert!(
        s.chain
            .short_reversals(p.tight_reverse_r, p.min_straight_m)
            .is_empty(),
        "short reversals {:?}",
        s.chain.short_reversals(p.tight_reverse_r, p.min_straight_m)
    );
}

#[test]
fn r1_straight_river_is_one_straight() {
    let river = fixtures::straight(5000.0);
    let p = params(R_MIN, LOA);
    let s = seed(&river, &p, None);
    report("R1", &s);
    guarantees(&s, &p);
    assert_eq!(s.chain.elements.len(), 1);
    assert!(!s.chain.elements[0].is_arc());
    assert!(largest_distance(&s.chain, &river) < 0.5);
}

/// R2. A sine river with bends wider than R_min. Departure from the spec: "within 2 m, and one arc
/// per bend (±1)" cannot both hold on this river. The best single arc per bend is about 4 m off
/// the sine at its worst (a sine's curvature rises and falls smoothly; an arc's is a step), the
/// best two about the same, the best three under 1 m. So at the default tolerance of 2 m the seed
/// takes three arcs per bend (a compound curve: a wide arc, the bend's own, a wide arc), and at a
/// tolerance of 5 m one arc per bend.
#[test]
fn r2_gentle_sine_is_followed_with_few_arcs_per_bend() {
    let (a, wl, len) = (200.0, 2400.0, 12000.0);
    let river = fixtures::sine(a, wl, len);
    let bends = (2.0 * len / wl).round() as usize;
    let arcs = |s: &Seed| s.chain.elements.iter().filter(|e| e.is_arc()).count();
    // The default tolerance: within 2 m, three arcs a bend at most.
    let p = params(R_MIN, LOA);
    let s = seed(&river, &p, None);
    report("R2 at 2 m", &s);
    guarantees(&s, &p);
    let d = largest_distance(&s.chain, &river);
    assert!(d <= 2.0, "largest distance from the centreline {d:.2} m");
    assert!(
        arcs(&s) <= 3 * bends + 1,
        "{} arcs for {bends} bends",
        arcs(&s)
    );
    assert!(s.tight.is_empty());
    assert!(s.settled);
    // A tolerance of 5 m: one arc a bend (±1), straights between.
    let p5 = SeedParams::scaled(R_MIN, 1.0333, LOA, 5.0);
    let s5 = seed(&river, &p5, None);
    report("R2 at 5 m", &s5);
    guarantees(&s5, &p5);
    assert!(largest_distance(&s5.chain, &river) <= 5.0);
    assert!(
        arcs(&s5) + 1 >= bends && arcs(&s5) <= bends + 1,
        "{} arcs for {bends} bends",
        arcs(&s5)
    );
    for w in s5.chain.elements.windows(2) {
        assert!(
            !(w[0].is_arc() && w[1].is_arc() && w[0].hand() != w[1].hand()),
            "a straight between bends"
        );
    }
}

#[test]
fn r3_bends_tighter_than_r_min_are_left_and_recorded() {
    let wl = 1600.0;
    let a = fixtures::sine_amplitude_for_radius(200.0, wl);
    let len = 8000.0;
    let river = fixtures::sine(a, wl, len);
    let p = params(R_MIN, LOA);
    let s = seed(&river, &p, None);
    report("R3", &s);
    guarantees(&s, &p);
    let bends = (2.0 * len / wl).round() as usize;
    // Every apex of the river is a tight bend of the seed.
    assert!(
        s.tight.iter().filter(|t| t.reason == "tight").count() >= bends - 1,
        "{:?}",
        s.tight
    );
    for t in &s.tight {
        assert!(t.river_radius_m < R_MIN, "{t:?}");
    }
}

#[test]
fn r9_small_r_min_holds_the_minimum_straight() {
    let r_min = 100.0;
    let wl = 900.0;
    let a = fixtures::sine_amplitude_for_radius(150.0, wl);
    let river = fixtures::sine(a, wl, 6000.0);
    let p = params(r_min, 50.0);
    let s = seed(&river, &p, None);
    report("R9", &s);
    guarantees(&s, &p);
}

#[test]
fn tight_sine_with_reversals_holds_the_minimum_straight() {
    // Opposite bends of radius 320 m, 2 R_min apart at most: no straight in the river between them.
    let wl = 1400.0;
    let a = fixtures::sine_amplitude_for_radius(320.0, wl);
    let river = fixtures::sine(a, wl, 7000.0);
    let p = params(R_MIN, LOA);
    let s = seed(&river, &p, None);
    report("reversals", &s);
    guarantees(&s, &p);
}

#[test]
fn bends_of_arcs_and_straights_are_rebuilt_as_themselves() {
    use core::f64::consts::PI;
    let river = fixtures::bends(
        400.0,
        &[(PI / 3.0, 500.0), (-PI / 2.0, 400.0), (PI / 4.0, 900.0)],
    );
    let p = params(R_MIN, LOA);
    let s = seed(&river, &p, None);
    report("arcs", &s);
    guarantees(&s, &p);
    assert_eq!(s.chain.elements.iter().filter(|e| e.is_arc()).count(), 3);
    let radii: Vec<f64> = s
        .chain
        .elements
        .iter()
        .filter(|e| e.is_arc())
        .map(|e| e.radius())
        .collect();
    for (r, want) in radii.iter().zip([500.0, 400.0, 900.0]) {
        assert!((r - want).abs() < 0.03 * want, "{radii:?}");
    }
    assert!(largest_distance(&s.chain, &river) < 2.0);
}

/// Performance: a synthetic river of about 200 km with 300 bends at R_min 180 m. Run with
/// `cargo test --release -- --ignored seed_on_a_long_river`.
#[test]
#[ignore]
fn seed_on_a_long_river() {
    let river = fixtures::meanders(300, 150.0, 900.0, 300.0, 7);
    let len = linear::length(&river);
    let p = params(180.0, 110.0);
    let t0 = std::time::Instant::now();
    let s = seed(&river, &p, None);
    let dt = t0.elapsed();
    eprintln!(
        "river {:.1} km: seed in {:.2} s, {} elements ({} arcs), gap {:.2} m, tight {}",
        len / 1000.0,
        dt.as_secs_f64(),
        s.chain.elements.len(),
        s.chain.elements.iter().filter(|e| e.is_arc()).count(),
        s.gap_m,
        s.tight.len()
    );
    guarantees(&s, &p);
}
