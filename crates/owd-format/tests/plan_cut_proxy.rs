//! The proxy volume: with no terrain, a volume is the plan cut area times a stated height. It is
//! labelled "proxy", it carries its height, and it is never added to a terrain volume.
//!
//! The expected volume is closed-form: (W - w) x L x h, with W worked out here from the sizing in
//! the document: W = bottom width + 2 x side slope x design depth.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::volumes::sections::DEFAULT_WET_SLOPE;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const L: f64 = 2000.0;

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> Vec<Pt> {
    vec![
        [X0 + x0, Y0 + y0],
        [X0 + x1, Y0 + y0],
        [X0 + x1, Y0 + y1],
        [X0 + x0, Y0 + y1],
        [X0 + x0, Y0 + y0],
    ]
}

/// The `.owd` text of a straight river `w` wide with the given islands: a centreline and a river
/// polygon, sized for CEMT IV, with one design line option on the centreline.
fn design(w: f64, islands: &[Vec<Pt>]) -> String {
    let centre: Vec<Pt> = (0..=40).map(|i| [X0 + 50.0 * i as f64, Y0]).collect();
    let mut rings = vec![rect(-100.0, L + 100.0, -w / 2.0, w / 2.0)];
    rings.extend(islands.iter().cloned());
    let mut s = Session::new("Straight river", "2026-11-10");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(centre)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!(rings)
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
    let d = s.document();
    assert!(d["inputs"]["dem"].is_null() && d["inputs"]["water_level"].is_null());
    let line = d["options"][0]["line"]["coordinates"].as_array().unwrap();
    assert!(line.iter().all(|p| p[1].as_f64() == Some(Y0)));
    assert_eq!(line[line.len() - 1][0].as_f64(), Some(X0 + L));
    s.to_text()
}

fn close(got: &Value, want: f64) -> bool {
    let got = got.as_f64().unwrap_or(f64::NAN);
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

fn row(s: &Session) -> Value {
    s.plan_cut_result().expect("the plan cut has run")["options"][0].clone()
}

#[test]
fn the_proxy_volume_is_the_plan_area_times_the_stated_height() {
    let (w, h) = (30.0, 4.0);
    let mut s = Session::open(&design(w, &[])).unwrap();
    s.run_plan_cut(&json!({"proxy_height_m": h})).unwrap();
    let sizing = &s.document()["stages"]["sizing"]["results"];
    let wide = sizing["bottom_width_straight_m"].as_f64().unwrap()
        + 2.0 * DEFAULT_WET_SLOPE * sizing["design_depth_m"].as_f64().unwrap();
    let area = (wide - w) * L;
    let r = row(&s);
    assert!(close(&r["area_m2"], area));
    let p = &r["proxy_volume"];
    assert!(
        close(&p["volume_m3"], area * h),
        "{} != {}",
        p["volume_m3"],
        area * h
    );
    assert!(close(&p["by_bank"]["left_m3"], area * h / 2.0));
    assert!(close(&p["by_bank"]["right_m3"], area * h / 2.0));
    assert_eq!(p["height_m"], h);
    assert_eq!(
        s.document()["stages"]["plan_cut"]["settings"]["proxy_height_m"],
        h
    );
    assert!(
        p["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("proxy")
    );
}

#[test]
fn with_no_height_there_is_no_proxy_volume() {
    let mut s = Session::open(&design(30.0, &[])).unwrap();
    s.run_plan_cut(&json!({})).unwrap();
    assert!(row(&s).get("proxy_volume").is_none());
    assert!(s.document()["stages"]["plan_cut"]["settings"]["proxy_height_m"].is_null());

    // A height is kept by a later run, and null takes it away.
    s.run_plan_cut(&json!({"proxy_height_m": 2.5})).unwrap();
    s.run_plan_cut(&json!({})).unwrap();
    assert_eq!(row(&s)["proxy_volume"]["height_m"], 2.5);
    s.run_plan_cut(&json!({"proxy_height_m": null})).unwrap();
    assert!(row(&s).get("proxy_volume").is_none());
}

#[test]
fn a_height_that_is_not_a_positive_number_is_refused() {
    let mut s = Session::open(&design(30.0, &[])).unwrap();
    for bad in [json!(0.0), json!(-2.0), json!("high")] {
        let e = s
            .run_plan_cut(&json!({"proxy_height_m": bad}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("proxy_height_m"), "{bad}: {e}");
    }
    assert!(s.plan_cut_result().is_none());
}

#[test]
fn a_proxy_volume_is_never_added_to_a_terrain_volume() {
    // The web example holds terrain volumes. The plan cut with a height leaves them as they are.
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web/river-example.owd");
    let mut s = Session::open(&std::fs::read_to_string(example).unwrap()).unwrap();
    let volumes = s.document()["stages"]["volumes"].clone();
    assert!(volumes["results"]["options"][0]["total_m3"].is_number());
    s.run_plan_cut(&json!({"proxy_height_m": 4.0})).unwrap();
    assert_eq!(s.document()["stages"]["volumes"], volumes);

    // The proxy has its own member and its own names: no key of a terrain volume is used for it.
    let r = row(&s);
    assert!(r["proxy_volume"]["volume_m3"].as_f64().unwrap() > 0.0);
    for key in ["total_m3", "dredge_m3", "excav_m3"] {
        assert!(r.get(key).is_none(), "`{key}` is a terrain volume's name");
        assert!(r["proxy_volume"].get(key).is_none());
    }
}
