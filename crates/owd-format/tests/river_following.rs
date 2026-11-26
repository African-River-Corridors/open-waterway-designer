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
use owd_format::design::{AlignMethod, AlignmentSettings};
use owd_format::types::{Polyline, RiverPolygon, TerrainSampler};
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
    let poly = fixtures::polygon(centre, width);
    let ring: Vec<Vec<f64>> = poly.exterior.iter().map(|p| p.to_vec()).collect();
    let river_in = RiverIn {
        centreline: Some(Polyline(centre.to_vec())),
        river_polygon: RiverPolygon::from_geometry(
            &json!({"type": "Polygon", "coordinates": [ring]}),
        ),
        ..Default::default()
    };
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
