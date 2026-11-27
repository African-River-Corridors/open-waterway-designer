//! The weirs, the locks and the pools of a design, for the map (MAP-4): a typed list made on
//! demand from the water level along the design line. Nothing is stored in the design.
//!
//! The design is a straight river 3000 m long and 600 m wide in UTM 30N, heading east, with three
//! pools: 8 m from 0 m, 10 m from 1000 m and 12 m from 2000 m. A weir with a lock sits at 1000 m
//! and at 2000 m, each with a lift of 2 m.
//!
//! Label: property

use owd_format::design::crs::Crs;
use owd_format::design::structures::{
    CHAMBER_LENGTH_FACTOR, CHAMBER_SIDE_ROOM_M, DEFAULT_WEIR_LENGTH_M, PLACEHOLDER_NOTE,
    StructuresIn, WALL_FREEBOARD_M, WALL_THICKNESS_M, structures,
};
use owd_format::design::water_stage;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::json;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const POOLS: [[f64; 2]; 3] = [[0.0, 8.0], [1000.0, 10.0], [2000.0, 12.0]];

/// The straight river, sized and aligned, with no water. `polygon`: with its river polygon.
fn design(polygon: bool) -> Session {
    let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let line = format!(
        r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
        json!(centre)
    );
    let poly = format!(
        r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
        json!([ring])
    );
    let mut s = Session::new("Weirs", "2026-11-20");
    if polygon {
        s.import(&[(Role::Centreline, &line), (Role::RiverPolygon, &poly)])
            .unwrap();
    } else {
        s.import(&[(Role::Centreline, &line)]).unwrap();
    }
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::Centreline, &json!({}), None).unwrap();
    s
}

fn utm(lon_lat: [f64; 2]) -> [f64; 2] {
    let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
        unreachable!()
    };
    u.forward(lon_lat[0], lon_lat[1]).unwrap()
}

#[test]
fn three_pools_give_two_weirs_with_their_levels_lifts_and_places_on_the_line() {
    let mut s = design(true);
    s.set_water_pools(&POOLS).unwrap();
    let before = s.to_text();
    let m = s.map_structures().unwrap();

    assert_eq!(m.pools.len(), 3);
    for (i, (p, want)) in m.pools.iter().zip(POOLS).enumerate() {
        assert_eq!(p.number, i + 1, "pools are counted from the mouth");
        assert_eq!(p.level_m, want[1]);
        assert!((p.from_m - want[0]).abs() < 0.01, "{p:?}");
        assert!((p.to_m - (want[0] + 1000.0)).abs() < 0.01, "{p:?}");
        // Its reach of the design line, and the middle of it for the label.
        let (first, last) = (utm(p.line[0]), utm(p.line[p.line.len() - 1]));
        assert!((first[0] - (X0 + want[0])).abs() < 0.01 && (first[1] - Y0).abs() < 0.01);
        assert!((last[0] - (X0 + want[0] + 1000.0)).abs() < 0.01);
        assert!((utm(p.mid)[0] - (X0 + want[0] + 500.0)).abs() < 0.01);
    }

    assert_eq!(m.weirs.len(), 2);
    for (i, w) in m.weirs.iter().enumerate() {
        let at = 1000.0 * (i + 1) as f64;
        assert_eq!(w.number, i + 1);
        assert!((w.chainage_m - at).abs() < 0.01, "{w:?}");
        let here = utm(w.position);
        assert!(
            (here[0] - (X0 + at)).abs() < 0.01 && (here[1] - Y0).abs() < 0.01,
            "{here:?}"
        );
        // Upstream is the higher chainage: the pool this weir holds.
        assert_eq!(w.upstream_level_m, POOLS[i + 1][1]);
        assert_eq!(w.downstream_level_m, POOLS[i][1]);
        assert_eq!(w.lift_m, 2.0);
        assert_eq!(
            (w.downstream_pool, w.upstream_pool),
            (Some(i + 1), Some(i + 2))
        );
        // The line heads east on the central meridian of the zone.
        assert!((w.bearing_deg - 90.0).abs() < 0.01, "{}", w.bearing_deg);
        // The weir is clipped to the river polygon: 600 m from bank to bank.
        assert!(w.clipped_to_river);
        assert!((w.weir_length_m - 600.0).abs() < 0.01);
    }

    // The placeholder model: sizes from the vessel, stated in one place.
    let size = s.channel_size().unwrap();
    assert_eq!(m.model.note, PLACEHOLDER_NOTE);
    assert_eq!(m.model.note, "placeholder model, not a design");
    assert_eq!(m.model.vessel_length_m, Some(size.loa_m));
    assert_eq!(
        m.model.chamber_length_m,
        Some(CHAMBER_LENGTH_FACTOR * size.loa_m)
    );
    assert_eq!(
        m.model.chamber_width_m,
        Some(size.beam_m + CHAMBER_SIDE_ROOM_M)
    );
    assert_eq!(
        (m.model.wall_thickness_m, m.model.wall_freeboard_m),
        (3.0, 1.5)
    );
    let (len, wid) = (1.2 * size.loa_m, size.beam_m + 2.0);
    let w = &m.weirs[0];
    let kinds: Vec<&str> = w.parts.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            "lock_chamber",
            "lock_wall",
            "lock_wall",
            "lock_gate",
            "lock_gate",
            "weir",
            "weir"
        ]
    );
    let extent = |ring: &[[f64; 2]]| {
        let pts: Vec<[f64; 2]> = ring.iter().map(|p| utm(*p)).collect();
        let fold =
            |i: usize, f: fn(f64, f64) -> f64, init: f64| pts.iter().fold(init, |a, p| f(a, p[i]));
        (
            fold(0, f64::min, f64::INFINITY) - (X0 + 1000.0),
            fold(0, f64::max, f64::NEG_INFINITY) - (X0 + 1000.0),
            fold(1, f64::min, f64::INFINITY) - Y0,
            fold(1, f64::max, f64::NEG_INFINITY) - Y0,
        )
    };
    let near = |got: (f64, f64, f64, f64), want: (f64, f64, f64, f64)| {
        assert!(
            (got.0 - want.0).abs() < 0.02
                && (got.1 - want.1).abs() < 0.02
                && (got.2 - want.2).abs() < 0.02
                && (got.3 - want.3).abs() < 0.02,
            "{got:?} against {want:?}"
        );
    };
    let (a, b, t) = (len / 2.0, wid / 2.0, WALL_THICKNESS_M);
    // The water in the chamber, between the gates, at the lower pool level.
    near(extent(&w.parts[0].ring), (-a + t, a - t, -b, b));
    assert_eq!(w.parts[0].top_m, 8.0);
    // Two walls along the chamber and a gate block at each end, 1.5 m above the upstream pool.
    near(extent(&w.parts[1].ring), (-a, a, b, b + t));
    near(extent(&w.parts[2].ring), (-a, a, -b - t, -b));
    near(extent(&w.parts[3].ring), (-a, -a + t, -b, b));
    near(extent(&w.parts[4].ring), (a - t, a, -b, b));
    for p in &w.parts[1..5] {
        assert_eq!(p.top_m, 10.0 + WALL_FREEBOARD_M);
    }
    // The weir: a wall from each lock wall to its bank, its crest at the upstream pool level.
    near(extent(&w.parts[5].ring), (-t / 2.0, t / 2.0, b + t, 300.0));
    near(
        extent(&w.parts[6].ring),
        (-t / 2.0, t / 2.0, -300.0, -b - t),
    );
    assert_eq!((w.parts[5].top_m, w.parts[6].top_m), (10.0, 10.0));
    for p in &w.parts {
        assert_eq!(p.ring.first(), p.ring.last(), "a closed ring");
    }

    // Derived on demand: the design is the same text, and holds no word of it.
    assert_eq!(s.to_text(), before);
    assert!(!before.contains("weir_length") && !before.contains("placeholder"));
}

#[test]
fn without_a_river_polygon_the_weir_has_the_default_length() {
    let mut s = design(false);
    s.set_water_pools(&POOLS).unwrap();
    let m = s.map_structures().unwrap();
    assert_eq!(m.weirs.len(), 2);
    assert_eq!(
        m.level_steps, 2,
        "the check the map shows: a weir for each step"
    );
    for w in &m.weirs {
        assert!(!w.clipped_to_river);
        assert_eq!(w.weir_length_m, DEFAULT_WEIR_LENGTH_M);
    }
}

#[test]
fn a_design_with_no_pools_gives_an_empty_list() {
    // No water at all.
    let mut s = design(true);
    let m = s.map_structures().unwrap();
    assert!(m.pools.is_empty() && m.weirs.is_empty());
    assert_eq!(s.pool_water_input().unwrap(), None);
    // One level, and a straight fall: water, but no step and so no pool.
    s.set_water_profile(&[[0.0, 9.0]]).unwrap();
    let m = s.map_structures().unwrap();
    assert!(m.pools.is_empty() && m.weirs.is_empty(), "{m:?}");
    s.set_water_fall(12.0, 0.001).unwrap();
    let m = s.map_structures().unwrap();
    assert!(m.pools.is_empty() && m.weirs.is_empty(), "{m:?}");
    assert_eq!(s.pool_water_input().unwrap(), None);
    // A new design, with nothing in it.
    let blank = Session::new("Blank", "2026-11-20");
    assert!(blank.map_structures().unwrap().weirs.is_empty());
}

#[test]
fn before_a_vessel_is_chosen_the_weirs_are_listed_with_no_lock() {
    let mut s = design(true);
    s.set_water_pools(&POOLS).unwrap();
    let with = s.map_structures().unwrap();
    assert!(with.weirs[0].parts.iter().any(|p| p.kind == "lock_wall"));
    // The same line and water, with no vessel: one weir wall from bank to bank, and no lock.
    let input = s.water_input().unwrap();
    let water = water_stage::water(&input).unwrap();
    let bare = structures(
        &StructuresIn {
            line: &input.design_line.0,
            water: &water,
            polygons: &[],
            vessel: None,
        },
        &|pts| Ok(pts.to_vec()),
    )
    .unwrap();
    assert_eq!(bare.model.chamber_length_m, None);
    assert_eq!(bare.weirs.len(), 2);
    for w in &bare.weirs {
        assert_eq!(w.parts.len(), 1);
        assert_eq!(
            (w.parts[0].kind, w.parts[0].top_m),
            ("weir", w.upstream_level_m)
        );
    }
}

#[test]
fn the_list_is_json_for_the_app_and_the_pool_water_reads_the_same_pools() {
    let mut s = design(true);
    s.set_water_pools(&POOLS).unwrap();
    let v = serde_json::to_value(s.map_structures().unwrap()).unwrap();
    assert_eq!(v["weirs"][1]["upstream_level_m"], 12.0);
    assert_eq!(v["weirs"][1]["parts"][0]["kind"], "lock_chamber");
    assert_eq!(v["pools"][2]["number"], 3);
    assert_eq!(v["model"]["note"], PLACEHOLDER_NOTE);
    let input = s.pool_water_input().unwrap().unwrap();
    assert_eq!(input.pools.len(), 3);
    let (number, from, to, level) = input.pools[1];
    assert_eq!((number, level), (2, 10.0));
    assert!((from - 1000.0).abs() < 0.01 && (to - 2000.0).abs() < 0.01);
    assert_eq!(input.line.len(), input.chainage_m.len());
    assert!((input.chainage_m[input.chainage_m.len() - 1] - 3000.0).abs() < 0.01);
    assert_eq!(input.polygon.len(), 1);
}
