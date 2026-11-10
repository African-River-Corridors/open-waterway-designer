//! The volumes stage reports the cut of each option by kind: in the river, widening of the river,
//! cut-through. The three add up to the total, and no number the stage reports today moves.
//!
//! The design is a straight river 3000 m long in UTM 30N, on flat terrain at the water level. The
//! design line is the straight centreline. The test works the closed form out from the sizing in
//! its own document: half the bottom width `hb`, the design depth `d`, and a wet slope of 3. The
//! samples are 1 m apart, at whole metres from the line, and the cut is `d` on the bottom and
//! `d - r / 3` at `r` metres up a slope.
//!
//! Label: property

use owd_format::design::{AlignMethod, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const LEVEL_M: f64 = 10.0;
const STATION_M: f64 = 25.0;
const STATIONS: f64 = 121.0;

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![LEVEL_M; xs.len()]
}

/// A straight river with its polygon from `y0` to `y1` metres across the centreline, sized,
/// aligned, with a stand-in terrain record and a constant water level.
fn design(y0: f64, y1: f64) -> Session {
    let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 + y0],
        [X0 + 3100.0, Y0 + y0],
        [X0 + 3100.0, Y0 + y1],
        [X0 - 100.0, Y0 + y1],
        [X0 - 100.0, Y0 + y0],
    ];
    let mut s = Session::new("Volume by kind", "2026-11-10");
    s.import(&[
        (
            owd_format::design::Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            owd_format::design::Role::RiverPolygon,
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
    s.set_water_profile(&[[0.0, LEVEL_M]]).unwrap();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    s
}

fn row(s: &Session) -> Value {
    s.document()["stages"]["volumes"]["results"]["options"][0].clone()
}

fn num(v: &Value, key: &str) -> f64 {
    v[key]
        .as_f64()
        .unwrap_or_else(|| panic!("the volumes row has no number `{key}`: {v}"))
}

fn close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

/// The cut of one section, in m2: all of it, and the part within `river_half_m` of the line.
fn section_m2(s: &Session, river_half_m: f64) -> (f64, f64) {
    let sizing = &s.document()["stages"]["sizing"]["results"];
    let hb = sizing["bottom_width_straight_m"].as_f64().unwrap() / 2.0;
    let d = sizing["design_depth_m"].as_f64().unwrap();
    let (mut all, mut inside) = (0.0, 0.0);
    for o in -200..=200 {
        let o = f64::from(o);
        let r = (o.abs() - hb).max(0.0);
        let cut = (d - r / 3.0).max(0.0);
        all += cut;
        if o.abs() < river_half_m {
            inside += cut;
        }
    }
    (all, inside)
}

/// The numbers the stage reports today, against the closed form: they must not move.
fn todays_numbers_hold(s: &Session, river_half_m: f64) {
    let r = row(s);
    let (all, inside) = section_m2(s, river_half_m);
    let to_m3 = STATION_M * STATIONS;
    assert_eq!(r["stations"], json!(121));
    close(num(&r, "dredge_m3"), inside * to_m3, "dredge_m3");
    close(num(&r, "excav_m3"), (all - inside) * to_m3, "excav_m3");
    close(num(&r, "total_m3"), all * to_m3, "total_m3");
}

#[test]
fn a_river_narrower_than_the_channel_is_in_the_river_and_widening_in_the_known_ratio() {
    // The river is 21 m wide; the bottom of the channel is wider.
    let s = design(-10.5, 10.5);
    todays_numbers_hold(&s, 10.5);
    let r = row(&s);
    let (all, inside) = section_m2(&s, 10.5);
    assert!(inside > 0.0 && inside < all);
    close(num(&r, "in_river_m3"), num(&r, "dredge_m3"), "in_river_m3");
    close(num(&r, "widening_m3"), num(&r, "excav_m3"), "widening_m3");
    assert_eq!(num(&r, "cut_through_m3"), 0.0);
    close(
        num(&r, "in_river_m3") / num(&r, "total_m3"),
        inside / all,
        "the share in the river",
    );
    close(
        num(&r, "in_river_m3") + num(&r, "widening_m3") + num(&r, "cut_through_m3"),
        num(&r, "total_m3"),
        "the three kinds add up to total_m3",
    );
}

#[test]
fn a_line_inside_a_wide_river_is_all_in_the_river() {
    let s = design(-300.0, 300.0);
    todays_numbers_hold(&s, 300.0);
    let r = row(&s);
    close(num(&r, "in_river_m3"), num(&r, "total_m3"), "in_river_m3");
    assert_eq!(num(&r, "widening_m3"), 0.0);
    assert_eq!(num(&r, "cut_through_m3"), 0.0);
}

#[test]
fn a_line_across_dry_land_is_all_cut_through() {
    // The design has a river polygon, but it lies 5 km from the line.
    let s = design(5000.0, 5100.0);
    todays_numbers_hold(&s, 0.0);
    let r = row(&s);
    close(
        num(&r, "cut_through_m3"),
        num(&r, "total_m3"),
        "cut_through_m3",
    );
    assert_eq!(num(&r, "in_river_m3"), 0.0);
    assert_eq!(num(&r, "widening_m3"), 0.0);
}

#[test]
fn the_long_section_and_the_cross_section_give_the_kinds_at_each_station() {
    let s = design(-10.5, 10.5);
    let (all, inside) = section_m2(&s, 10.5);
    let walk = s.option_section_walk("rolling-circle", &flat).unwrap();
    let long = s.long_section("rolling-circle", &walk).unwrap();
    let col = |key: &str| -> Vec<f64> {
        long[key]
            .as_array()
            .unwrap_or_else(|| panic!("the long section has no series `{key}`"))
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    let (river, widening, through) = (
        col("in_river_area_m2"),
        col("widening_area_m2"),
        col("cut_through_area_m2"),
    );
    let (dredge, excav) = (col("dredge_area_m2"), col("excav_area_m2"));
    assert_eq!(river.len(), 121);
    for i in 0..121 {
        close(river[i], inside, "in the river, one station");
        close(widening[i], all - inside, "widening, one station");
        assert_eq!(through[i], 0.0);
        close(
            river[i] + widening[i] + through[i],
            dredge[i] + excav[i],
            "the kinds of one station add up",
        );
    }
    let cross = s.cross_section(&walk, 1500.0).unwrap();
    close(num(&cross, "in_river_area_m2"), inside, "the cross-section");
    close(
        num(&cross, "widening_area_m2"),
        all - inside,
        "the cross-section",
    );
    assert_eq!(num(&cross, "cut_through_area_m2"), 0.0);
}
