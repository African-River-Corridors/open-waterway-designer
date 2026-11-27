//! The river-following line as an alignment method (OPT-4), through the sizing and river stages, on
//! synthetic rivers only.
//!
//! Label: property

use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::alignment::river_following::fixtures;
use owd_core::geom::Pt;
use owd_core::rules::engine;
use owd_format::design::design_line::{AlignmentIn, DesignLine, alignment};
use owd_format::design::river::{RiverIn, river};
use owd_format::design::sizing::{ChannelSize, SizingIn, size};
use owd_format::design::{AlignMethod, AlignmentSettings, VolumesSettings};
use owd_format::types::{RiverPolygon, TerrainSampler};
use serde_json::{Value, json};

/// The size of a custom motor vessel `loa` metres long with the built-in rules at quality `q`.
fn sized(loa: f64, beam: f64, q: &str) -> ChannelSize {
    let mut s: SizingIn = serde_json::from_value(json!({
        "rule_sets": [{"id": "pianc-wg141", "version": "0.3.0"}, {"id": "owd", "version": "0.3.0"}],
        "set_inputs": [{"name": "se_quality", "value": q}],
        "custom_vessel": {"name": "Test barge", "loa_m": loa, "beam_m": beam, "draught_m": 2.5,
            "formation": "motor"},
    }))
    .unwrap();
    for set in &mut s.rule_sets {
        set.hash = engine::built_in()
            .find(&set.id, &set.version)
            .map(|x| x.hash.clone());
    }
    size(&s, engine::built_in()).unwrap()
}

fn input(centre: &[Pt], width: f64, size: ChannelSize, settings: Value) -> AlignmentIn {
    input_with(
        centre,
        Some(fixtures::polygon(centre, width)),
        size,
        settings,
    )
}

fn input_with(
    centre: &[Pt],
    poly: Option<owd_core::geom::Polygon>,
    size: ChannelSize,
    settings: Value,
) -> AlignmentIn {
    let mut river_in: RiverIn = serde_json::from_value(json!({
        "centreline": centre,
        "water_profile": [[0.0, 10.0]],
    }))
    .unwrap();
    if let Some(poly) = poly {
        let ring: Vec<Vec<f64>> = poly.exterior.iter().map(|p| p.to_vec()).collect();
        river_in.river_polygon =
            RiverPolygon::from_geometry(&json!({"type": "Polygon", "coordinates": [ring]}));
    }
    let mut s = AlignmentSettings {
        method: AlignMethod::RiverFollowing,
        ..Default::default()
    };
    s.river_following = s.river_following.apply(&settings).unwrap();
    AlignmentIn {
        river: river(&river_in).unwrap(),
        size,
        settings: s,
        seed: None,
        constraints: None,
        // The volumes section, coarser across than the default so the tests run in seconds: the
        // same one scorer, sampled every 2 m out to 100 m.
        volumes: Some(VolumesSettings {
            dx_m: 2.0,
            maxw_m: 100.0,
            ..Default::default()
        }),
    }
}

fn run(i: &AlignmentIn, terrain: Option<TerrainSampler>) -> DesignLine {
    alignment(i, terrain).unwrap()
}

#[test]
fn sizes_of_the_test_vessels() {
    for (loa, q) in [
        (75.0, "A"),
        (75.0, "B"),
        (75.0, "C"),
        (50.0, "C"),
        (90.0, "A"),
        (90.0, "C"),
    ] {
        let s = sized(loa, 11.4, q);
        eprintln!(
            "LOA {loa} quality {q}: R_min {} bottom {} depth {} widening {}",
            s.min_bend_radius_m,
            s.bottom_width_straight_m,
            s.design_depth_m,
            s.bend_widening_at_rmin_m
        );
    }
}

#[test]
fn a_gentle_river_gives_a_line_of_few_arcs_that_holds_r_min() {
    let size = sized(75.0, 11.4, "C");
    let r_min = size.min_bend_radius_m;
    let wl = 2400.0 * r_min / 300.0;
    let centre = fixtures::sine(200.0 * r_min / 300.0, wl, 5.0 * wl);
    let i = input(&centre, 60.0, size, json!({}));
    let l = run(&i, None);
    assert_eq!(l.method, "river_following");
    assert_eq!(l.id, "river-following");
    assert_eq!(l.checks["radius_ok"], true, "{}", l.checks);
    let rec = l.river_following.as_ref().unwrap();
    assert!(rec.seed.settled, "{:?}", rec.seed.gap_m);
    assert!(rec.elements.len() < 80, "{} elements", rec.elements.len());
    let r = min_radius_on_chord(&l.line.0, r_min / 3.0).unwrap();
    assert!(radius_ok(r, r_min));
    // Written as a design line and read back, it is the same.
    let back: DesignLine = serde_json::from_value(serde_json::to_value(&l).unwrap()).unwrap();
    assert_eq!(back, l);
}

/// The ground of a river `width` wide about `centre`: the bed level inside, `land` outside.
fn ground(centre: &[Pt], width: f64, bed: f64, land: f64) -> impl Fn(&[f64], &[f64]) -> Vec<f64> {
    let mask = owd_core::alignment::river_following::mask::RiverMask::new(
        &[fixtures::polygon(centre, width)],
        0.0,
    );
    move |xs: &[f64], ys: &[f64]| {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| if mask.contains([x, y]) { bed } else { land })
            .collect()
    }
}

/// The ground of the sine river `y = a sin(2π x / wl)`: the bed within `half` metres of it across
/// (measured along y, near enough for a gentle sine), `land` outside. Cheap to read, so the tests
/// time the method, not the fixture.
fn sine_ground(
    a: f64,
    wl: f64,
    half: f64,
    bed: f64,
    land: f64,
) -> impl Fn(&[f64], &[f64]) -> Vec<f64> {
    move |xs: &[f64], ys: &[f64]| {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let c = a * libm::sin(2.0 * core::f64::consts::PI * x / wl);
                if (y - c).abs() < half { bed } else { land }
            })
            .collect()
    }
}

/// The guarantees every delivered line holds.
fn guarantees(l: &DesignLine, r_min: f64) {
    let rec = l.river_following.as_ref().unwrap();
    let r = min_radius_on_chord(&l.line.0, r_min / 3.0).unwrap();
    assert!(radius_ok(r, r_min), "radius {r} against {r_min}");
    assert!(rec.checks.tangency_max_rad <= 1e-6);
    assert!(
        rec.checks.short_reversals.is_empty(),
        "{:?}",
        rec.checks.short_reversals
    );
    assert!(!rec.checks.self_crossing);
}

/// The owner's change: rule quality A to C (R_min 360 m to 180 m) on the same river. The line is
/// built again from the river, follows more of it, and holds every guarantee.
#[test]
fn a_change_of_rule_quality_builds_the_line_again() {
    let wl = 2000.0;
    let a = fixtures::sine_amplitude_for_radius(250.0, wl);
    let centre = fixtures::sine(a, wl, 6.0 * wl);
    let g = sine_ground(a, wl, 40.0, 6.0, 11.0);
    let mut lines = Vec::new();
    for q in ["A", "C"] {
        let size = sized(90.0, 11.4, q);
        let r_min = size.min_bend_radius_m;
        let i = input(&centre, 80.0, size, json!({}));
        let l = run(&i, Some(&g as TerrainSampler));
        guarantees(&l, r_min);
        let rec = l.river_following.as_ref().unwrap();
        let tight = rec.forced.iter().filter(|f| f.kind == "tight_bend").count();
        eprintln!(
            "quality {q}: R_min {r_min}, {} elements, tight bends {tight}, mean from centreline \
             {:.1} m, largest {:.1} m, derived clip {:.1}, straight {:.1}",
            rec.elements.len(),
            rec.checks.mean_from_centreline_m,
            rec.checks.largest_from_centreline_m,
            rec.derived.r_clip_m,
            rec.derived.min_straight_m
        );
        // Every threshold scales with R_min: none is a literal.
        assert!((rec.derived.r_clip_m - 1.0333 * r_min).abs() < 1e-9);
        assert!((rec.derived.tight_reverse_r_m - 2.0 * r_min).abs() < 1e-9);
        let mean = rec.checks.mean_from_centreline_m;
        lines.push((l, tight, mean));
    }
    assert_ne!(lines[0].0.line, lines[1].0.line);
    // At 180 m the river's 250 m bends are followed; at 360 m they are tight.
    assert!(
        lines[0].1 > 0 && lines[1].1 == 0,
        "{} {}",
        lines[0].1,
        lines[1].1
    );
    assert!(lines[1].2 < lines[0].2);
}

/// R11: no DEM: the seed and the shift run on plan area, no cut-through is taken by the rule, the
/// record says so.
#[test]
fn r11_without_a_dem_cut_throughs_are_listed_not_taken() {
    let size = sized(75.0, 11.4, "A");
    let centre = fixtures::omega(320.0, 115f64.to_radians(), 1500.0);
    let i = input(&centre, 60.0, size, json!({}));
    let l = run(&i, None);
    let rec = l.river_following.as_ref().unwrap();
    assert_eq!(rec.costed_on, "plan_area");
    assert!(!rec.cut_throughs.is_empty());
    assert!(rec.cut_throughs.iter().all(|c| !c.accepted));
    assert!(rec.cut_throughs.iter().all(|c| {
        c.refused_because
            .as_deref()
            .is_some_and(|r| r.starts_with("no DEM"))
    }));
    assert!(rec.warnings.iter().any(|w| w.starts_with("no DEM")));
    // Forced on by its id, it is taken, and the line is shorter.
    let id = rec.cut_throughs[0].id.clone();
    let forced = input(
        &centre,
        60.0,
        sized(75.0, 11.4, "A"),
        json!({"cut_force": {id.clone(): true}}),
    );
    let lf = run(&forced, None);
    let rf = lf.river_following.as_ref().unwrap();
    let c = rf.cut_throughs.iter().find(|c| c.id == id).unwrap();
    assert!(c.accepted && c.decided_by == "forced on");
    assert!(owd_core::geom::linear::length(&lf.line.0) < owd_core::geom::linear::length(&l.line.0));
    // The window optimiser's permitted ground holds it.
    assert!(
        rf.permitted_stretches()
            .iter()
            .any(|p| p.id == id && p.kind == "cut_through")
    );
    // And in the line's details, where the window optimiser reads it first: `permitted_stretches`,
    // `{id, kind, from, to, room_m}` in design-line chainage; and the older `cut_throughs[]` with
    // `from_m` and `to_m`.
    let ps = lf.details["permitted_stretches"].as_array().unwrap();
    let p = ps.iter().find(|p| p["id"] == json!(id)).unwrap();
    assert_eq!(p["kind"], "cut_through");
    assert!(p["from"].as_f64().unwrap() < p["to"].as_f64().unwrap());
    assert_eq!(p["room_m"].as_f64().unwrap(), rf.derived.cut_through_room_m);
    let mut keys: Vec<String> = p.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, vec!["from", "id", "kind", "room_m", "to"]);
    let d = &lf.details["cut_throughs"][0];
    assert_eq!(d["id"], json!(id));
    assert!(d["from_m"].as_f64().unwrap() < d["to_m"].as_f64().unwrap());
    guarantees(&lf, 300.0);
    // A forced id that matches nothing is named.
    let stale = input(
        &centre,
        60.0,
        sized(75.0, 11.4, "A"),
        json!({"cut_force": {"cut-99999-99999": true}}),
    );
    let ls = run(&stale, None);
    let rs = ls.river_following.as_ref().unwrap();
    assert_eq!(
        rs.checks.force_unmatched,
        vec!["cut-99999-99999".to_string()]
    );
    assert!(rs.warnings.iter().any(|w| w.contains("cut-99999-99999")));
}

/// Determinism: the same input gives the same line, to the bit.
#[test]
fn the_same_input_gives_the_same_line() {
    let centre = fixtures::omega(320.0, 115f64.to_radians(), 1500.0);
    let g = ground(&centre, 60.0, 8.0, 11.0);
    let a = run(
        &input(&centre, 60.0, sized(75.0, 11.4, "A"), json!({})),
        Some(&g as TerrainSampler),
    );
    let b = run(
        &input(&centre, 60.0, sized(75.0, 11.4, "A"), json!({})),
        Some(&g as TerrainSampler),
    );
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

/// A river narrower than the channel: the line still follows it, and the checks say how much of
/// it is out of the water.
#[test]
fn a_river_narrower_than_the_channel() {
    let size = sized(75.0, 11.4, "A");
    let bw = size.bottom_width_straight_m;
    let centre = fixtures::sine(150.0, 3000.0, 9000.0);
    let i = input(&centre, 0.5 * bw, size, json!({}));
    let g = sine_ground(150.0, 3000.0, 0.25 * bw, 8.0, 11.0);
    let l = run(&i, Some(&g as TerrainSampler));
    guarantees(&l, 300.0);
    let rec = l.river_following.as_ref().unwrap();
    eprintln!(
        "narrow: share outside {:?}, largest from centreline {:.1} m",
        rec.checks.share_outside, rec.checks.largest_from_centreline_m
    );
    // The line keeps its centre in the narrow river (half its width, less the margin).
    assert!(rec.checks.share_outside.unwrap() < 0.05);
    assert!(rec.checks.largest_from_centreline_m < 0.25 * bw);
}

/// R12: the river polygon 40 m off the ground's low line along a kilometre: the data check lists
/// the stretch; the line follows the polygon.
#[test]
fn r12_a_polygon_off_the_low_line_is_reported() {
    let centre = fixtures::straight(6000.0);
    // The ground's low channel: 40 m to the left from 2 km to 3 km.
    let low = |x: f64| {
        if (2000.0..=3000.0).contains(&x) {
            40.0
        } else {
            0.0
        }
    };
    let g = move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| if (y - low(x)).abs() < 30.0 { 6.0 } else { 11.0 })
            .collect()
    };
    let l = run(
        &input(
            &centre,
            60.0,
            sized(75.0, 11.4, "A"),
            json!({"shift": false}),
        ),
        Some(&g as TerrainSampler),
    );
    let rec = l.river_following.as_ref().unwrap();
    eprintln!("R12: {:?}", rec.low_line_mismatch);
    assert_eq!(rec.low_line_mismatch.len(), 1);
    let m = &rec.low_line_mismatch[0];
    assert!(m.river_from.0 >= 1800.0 && m.river_to.0 <= 3200.0 && m.largest_m >= 30.0);
    assert!(rec.warnings.iter().any(|w| w.contains("low line")));
    assert!(rec.checks.largest_from_centreline_m < 0.5);
}

/// R13: a delivered line moved out of the river over 300 m, outside any cut-through: the check
/// "off-river stretches are explained" fails and names the chainage.
#[test]
fn r13_a_line_moved_out_of_the_river_fails_the_check() {
    use owd_format::design::river_following::off_river_unexplained;
    let centre = fixtures::straight(6000.0);
    let l = run(
        &input(
            &centre,
            60.0,
            sized(75.0, 11.4, "A"),
            json!({"shift": false}),
        ),
        None,
    );
    let rec = l.river_following.as_ref().unwrap();
    let mask = owd_core::alignment::river_following::mask::RiverMask::new(
        &[fixtures::polygon(&centre, 60.0)],
        -3.0,
    );
    let ok = off_river_unexplained(
        &l.line.0,
        &mask,
        &rec.derived,
        &rec.cut_throughs,
        &rec.forced,
    );
    assert!(ok.is_empty());
    // Move the points from 3,000 m to 3,300 m 40 m to the left (the river is 30 m either side).
    let bad: Vec<Pt> = l
        .line
        .0
        .iter()
        .map(|p| {
            if (3000.0..=3300.0).contains(&p[0]) {
                [p[0], p[1] + 40.0]
            } else {
                *p
            }
        })
        .collect();
    let found = off_river_unexplained(&bad, &mask, &rec.derived, &rec.cut_throughs, &rec.forced);
    eprintln!("R13: {found:?}");
    assert_eq!(found.len(), 1);
    assert!(found[0][0] > 2950.0 && found[0][0] < 3050.0, "{found:?}");
}
