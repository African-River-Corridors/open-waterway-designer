//! The water level as a table of pools: each row is a level that holds from its chainage to the
//! next row, with a step at each row after the first. Nothing is read between two levels.
//!
//! The design is a straight river 3000 m long in UTM 30N with three pools: 12 m from 0 m, 10 m
//! from 1000 m and 8 m from 2000 m. The terrain is flat at 7.5 m, under every pool, so the cut of
//! a section is closed form: with the design invert `c` metres under the terrain, the cut is `c`
//! on the bottom and `c - r / 3` at `r` metres up a slope, at samples 1 m apart.
//!
//! Label: property

use owd_format::design::{AlignMethod, DesignError, Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const TERRAIN_M: f64 = 7.5;
const POOLS: [[f64; 2]; 3] = [[0.0, 12.0], [1000.0, 10.0], [2000.0, 8.0]];

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![TERRAIN_M; xs.len()]
}

fn centre() -> Vec<[f64; 2]> {
    (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect()
}

/// A straight river 600 m wide, sized and aligned, with a stand-in terrain record and no water.
fn design() -> Session {
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let mut s = Session::new("Pools", "2026-11-10");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre())
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    let dem = Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    s.set_dem("dem.tif", b"a stand-in terrain file", &dem);
    s
}

/// The level of the pool a chainage lies in.
fn pool_level(chainage_m: f64) -> f64 {
    POOLS
        .iter()
        .rev()
        .find(|p| p[0] <= chainage_m)
        .map_or(POOLS[0][1], |p| p[1])
}

fn refused(r: Result<(), DesignError>, word: &str) {
    match r {
        Err(DesignError::Input(m)) => assert!(m.contains(word), "the message lacks `{word}`: {m}"),
        other => panic!("expected a refusal with `{word}`, got {other:?}"),
    }
}

#[test]
fn the_level_at_a_chainage_is_the_level_of_the_pool_it_lies_in() {
    let mut s = design();
    s.set_water_pools(&POOLS).unwrap();
    let at = [
        0.0, 500.0, 999.0, 1001.0, 1500.0, 1999.0, 2001.0, 2500.0, 3000.0,
    ];
    let got = s.water_levels_along(&centre(), &at).unwrap();
    let want = [12.0, 12.0, 12.0, 10.0, 10.0, 10.0, 8.0, 8.0, 8.0];
    assert_eq!(got, want, "no level is read between two pools");

    // The first pool also holds upstream of its own chainage.
    s.set_water_pools(&[[400.0, 12.0], [1000.0, 10.0]]).unwrap();
    let got = s
        .water_levels_along(&centre(), &[100.0, 700.0, 1300.0])
        .unwrap();
    assert_eq!(got, [12.0, 12.0, 10.0]);
}

#[test]
fn a_pool_table_is_stored_as_the_stepped_profile_the_document_already_has() {
    let mut pools = design();
    pools.set_water_pools(&POOLS).unwrap();
    assert_eq!(
        pools.document()["inputs"]["water_level"],
        json!({"points": [[0.0, 12.0], [1000.0, 12.0], [1000.0, 10.0], [2000.0, 10.0], [2000.0, 8.0]]}),
        "each pool is flat to the next row, where the level steps"
    );
    let mut doubled = design();
    doubled
        .set_water_profile(&[
            [0.0, 12.0],
            [1000.0, 12.0],
            [1000.0, 10.0],
            [2000.0, 10.0],
            [2000.0, 8.0],
        ])
        .unwrap();
    assert_eq!(pools.to_text(), doubled.to_text());
    assert!(pools.validate().is_empty(), "{:?}", pools.validate());

    // One pool is a constant level.
    pools.set_water_pools(&[[0.0, 12.0]]).unwrap();
    assert_eq!(
        pools.document()["inputs"]["water_level"],
        json!({"points": [[0.0, 12.0]]})
    );
}

#[test]
fn a_pool_table_as_text_gives_the_same_water_level() {
    let mut numbers = design();
    numbers.set_water_pools(&POOLS).unwrap();
    let mut text = design();
    text.set_water_pools_text("chainage_m,level_m\n# three pools\n0,12\n1000, 10\n\n2000,8\n")
        .unwrap();
    assert_eq!(text.to_text(), numbers.to_text());

    let before = text.to_text();
    refused(
        text.set_water_pools_text("chainage_m,level_m\n0,12\n1000;10\n"),
        "line 3",
    );
    assert_eq!(text.to_text(), before, "a refused table changes nothing");
}

#[test]
fn a_pool_table_that_is_not_one_is_refused_and_nothing_changes() {
    let mut s = design();
    s.set_water_pools(&POOLS).unwrap();
    let before = s.to_text();
    refused(s.set_water_pools(&[]), "pool");
    // A pool needs a length: two rows at one chainage are not a pool table.
    refused(s.set_water_pools(&[[0.0, 12.0], [0.0, 10.0]]), "chainage");
    refused(
        s.set_water_pools(&[[1000.0, 12.0], [0.0, 10.0]]),
        "chainage",
    );
    refused(s.set_water_pools(&[[0.0, f64::NAN]]), "finite");
    assert_eq!(s.to_text(), before);
}

#[test]
fn the_volumes_stage_reads_each_station_in_its_own_pool() {
    let mut s = design();
    s.set_water_pools(&POOLS).unwrap();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    let sizing = s.document()["stages"]["sizing"]["results"].clone();
    let hb = sizing["bottom_width_straight_m"].as_f64().unwrap() / 2.0;
    let d = sizing["design_depth_m"].as_f64().unwrap();
    // The cut of one section whose design invert is `c` metres under the flat terrain.
    let section_m2 = |c: f64| -> f64 {
        (-200..=200)
            .map(|o| {
                let r = (f64::from(o).abs() - hb).max(0.0);
                (c - r / 3.0).max(0.0)
            })
            .sum()
    };

    let walk = s.option_section_walk("rolling-circle", &flat).unwrap();
    let long = s.long_section("rolling-circle", &walk).unwrap();
    let col = |key: &str| -> Vec<f64> {
        long[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    let (chainage, level, invert) = (
        col("chainage_m"),
        col("water_level_m"),
        col("design_invert_m"),
    );
    let (dredge, excav) = (col("dredge_area_m2"), col("excav_area_m2"));
    assert_eq!(chainage.len(), 121);
    let mut seen = [0usize; 3];
    for i in 0..chainage.len() {
        // A station on a step may fall either side of it by rounding: leave the two out.
        if POOLS[1..].iter().any(|p| (chainage[i] - p[0]).abs() < 1.0) {
            continue;
        }
        let want = pool_level(chainage[i]);
        assert_eq!(level[i], want, "the water level at {} m", chainage[i]);
        assert!((invert[i] - (want - d)).abs() < 1e-9);
        let cut = section_m2(TERRAIN_M - (want - d));
        assert!(
            (dredge[i] + excav[i] - cut).abs() <= 1e-6 * cut.max(1.0),
            "the cut at {} m: got {}, want {cut}",
            chainage[i],
            dredge[i] + excav[i]
        );
        seen[POOLS.iter().position(|p| p[1] == want).unwrap()] += 1;
    }
    assert_eq!(seen, [40, 39, 40], "stations read in each pool");
    // The cut steps with the pools: none under the top pool, the most under the bottom one.
    assert_eq!(section_m2(TERRAIN_M - (12.0 - d)), 0.0);
    assert!(section_m2(TERRAIN_M - (8.0 - d)) > section_m2(TERRAIN_M - (10.0 - d)));
    assert!(section_m2(TERRAIN_M - (10.0 - d)) > 0.0);
}
