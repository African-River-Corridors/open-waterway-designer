//! The water level never changes on the map without a weir drawn at that place (map fault 2).
//!
//! The design: a river in UTM 30N that runs east, turns north into a meander loop 500 m deep and
//! comes back, and a given design line that runs straight across the neck of the loop: a
//! cut-through, outside the river polygon. The pools are a water-level table in river chainage,
//! with the weir on the loop (river chainage 1800 m). The line reads the level of the nearest
//! point of the river, so the level steps on the line where its nearest point jumps from one leg
//! of the loop to the other: at line chainage 1300 m, outside the river polygon.
//!
//! Label: property

use owd_format::design::crs::Crs;
use owd_format::design::pool_water::PoolWater;
use owd_format::design::terrain_tile::TileMaker;
use owd_format::design::{Role, Session, Stage};
use owd_format::geotiff::Dem;
use owd_format::sectioned::TerrainRef;
use serde_json::json;
use std::sync::Arc;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

fn geo(kind: &str, coords: serde_json::Value) -> String {
    format!(r#"{{"type":"{kind}"{UTM30},"coordinates":{coords}}}"#)
}

fn off(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    pts.iter().map(|p| [X0 + p[0], Y0 + p[1]]).collect()
}

fn design() -> Session {
    let centre = off(&[
        [0.0, 0.0],
        [1000.0, 0.0],
        [1000.0, 500.0],
        [1600.0, 500.0],
        [1600.0, 0.0],
        [3000.0, 0.0],
    ]);
    let mut ring = off(&[
        [-100.0, 40.0],
        [960.0, 40.0],
        [960.0, 540.0],
        [1640.0, 540.0],
        [1640.0, 40.0],
        [3100.0, 40.0],
        [3100.0, -40.0],
        [1560.0, -40.0],
        [1560.0, 460.0],
        [1040.0, 460.0],
        [1040.0, -40.0],
        [-100.0, -40.0],
    ]);
    ring.push(ring[0]);
    let mut s = Session::new("Cut-through", "2026-11-26");
    s.import(&[
        (Role::Centreline, &geo("LineString", json!(centre))),
        (Role::RiverPolygon, &geo("Polygon", json!([ring]))),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    let line: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * f64::from(i), Y0]).collect();
    s.import_design_line(&geo("LineString", json!(line)), "Cut-through")
        .unwrap();
    // The pool table, in river chainage: the weir is on the loop.
    s.set_water_pools(&[[0.0, 8.0], [1800.0, 10.0]]).unwrap();
    s
}

#[test]
fn every_step_of_the_level_has_its_weir_with_a_lock_on_the_line_where_the_line_leaves_the_river() {
    let s = design();
    let m = s.map_structures().unwrap();
    assert_eq!(m.level_steps, 1, "one step of the level along the line");
    assert_eq!(m.weirs.len(), m.level_steps, "a weir for each step");
    let w = &m.weirs[0];
    assert!((w.chainage_m - 1300.0).abs() < 1.0, "{}", w.chainage_m);
    assert_eq!((w.downstream_level_m, w.upstream_level_m), (8.0, 10.0));
    // The line is outside the river polygon there: the weir takes the default length across the
    // line, and the lock still stands on the line.
    assert!(!w.clipped_to_river);
    let kinds: Vec<&str> = w.parts.iter().map(|p| p.kind).collect();
    assert!(kinds.contains(&"weir"), "{kinds:?}");
    assert!(
        kinds.contains(&"lock_chamber") && kinds.contains(&"lock_gate"),
        "{kinds:?}"
    );
}

#[test]
fn the_level_steps_only_at_weirs_along_the_water_profile() {
    // The steps counted straight from the water profile: every change of level between two
    // points of the profile is at one place (a step), never spread over a stretch.
    let s = design();
    let m = s.map_structures().unwrap();
    let input = s.pool_water_input().unwrap().unwrap();
    let changes = input.pools.windows(2).filter(|p| p[0].3 != p[1].3).count();
    assert_eq!(changes, m.weirs.len());
    for (p, w) in input.pools.windows(2).zip(&m.weirs) {
        assert!((p[0].2 - w.chainage_m).abs() < 1e-6 && (p[1].1 - w.chainage_m).abs() < 1e-6);
    }
}

#[test]
fn on_a_coarse_flood_mask_the_water_steps_on_the_weirs_cross_line() {
    let s = design();
    // A valley along the line: 5 m on it, up 0.02 m a metre to each side (both pools flood).
    let cell = 5.0;
    let (w, h) = (680usize, 400usize);
    let (left, top) = (X0 - 200.0, Y0 + 1000.0);
    let data = (0..w * h)
        .map(|i| {
            let y = top - ((i / w) as f64 + 0.5) * cell;
            5.0 + 0.02 * (y - Y0).abs()
        })
        .collect();
    let dem: TerrainRef = Arc::new(Dem {
        width: w,
        height: h,
        data,
        geotransform: [cell, 0.0, left, 0.0, -cell, top],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    });
    let maker = TileMaker::new(dem).unwrap();
    let input = s.pool_water_input().unwrap().unwrap();
    // Mask cells of tens of metres, as on a DEM of a billion cells.
    let water = PoolWater::with_budget(&maker, &input, 1500).unwrap();
    assert!(water.stats().cell_m >= 40.0, "{:?}", water.stats());
    let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
        unreachable!()
    };
    let at = |x: f64, y: f64| {
        let ll = u.inverse(X0 + x, Y0 + y).unwrap();
        water.at(&maker, ll[0], ll[1])
    };
    for y in [-60.0, -20.0, 0.0, 20.0, 60.0] {
        for (x, level) in [(1297.0, 8.0), (1303.0, 10.0), (1260.0, 8.0), (1340.0, 10.0)] {
            let wet = at(x, y).unwrap_or_else(|| panic!("water at {x}, {y}"));
            assert_eq!(wet.water_level_m, level, "at {x}, {y}");
        }
    }
}
