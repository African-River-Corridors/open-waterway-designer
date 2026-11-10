//! Ticket C3: the channel limits of each design line option as GeoJSON. The limits are the two
//! edges of the channel bottom: the design line moved left and right by the bottom half-width,
//! with the bend widening of the sized channel.
//!
//! Label: property
//!
//! Closed form. The test takes the widths from the sizing results of the document
//! (`bottom_width_straight_m`, `bottom_width_at_rmin_m`), not from the export. On a straight line
//! each limit is half the straight width from the line. On an arc of the minimum radius each
//! limit is half the width at the minimum radius from the line.

use owd_core::geom::Pt;
use owd_format::design::export::SCREENING_LABEL;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

/// A sized design (PIANC WG 141 quality B, CEMT IV) with one option whose line is `line`.
fn design(line: &[Pt]) -> Session {
    let mut s = Session::new("Limits test", "2026-11-10");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!([[X0, Y0], [X0 + 1500.0, Y0]])
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    // A synthetic option: the test puts its own line in the place of the option's line.
    let mut doc: Value = serde_json::from_str(&s.to_text()).unwrap();
    assert_eq!(doc["options"].as_array().unwrap().len(), 1);
    doc["options"][0]["line"] = json!({"type": "LineString", "coordinates": line});
    Session::open(&doc.to_string()).unwrap()
}

fn sizing(s: &Session, key: &str) -> f64 {
    s.document()["stages"]["sizing"]["results"][key]
        .as_f64()
        .unwrap()
}

/// The limit of `side` ("left" or "right") of the one option, from the export.
fn limit(fc: &Value, side: &str) -> Vec<Pt> {
    let found: Vec<&Value> = fc["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["properties"]["side"] == side)
        .collect();
    assert_eq!(found.len(), 1, "one {side} limit for the one option");
    let f = found[0];
    assert_eq!(f["properties"]["id"], "rolling-circle");
    assert_eq!(f["properties"]["screening"], SCREENING_LABEL);
    assert_eq!(f["geometry"]["type"], "LineString");
    f["geometry"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn dist(a: Pt, b: Pt) -> f64 {
    libm::hypot(a[0] - b[0], a[1] - b[1])
}

/// A vertex at least every 30 m, so the limit follows the line.
fn well_sampled(l: &[Pt]) {
    assert!(l.len() >= 2);
    for w in l.windows(2) {
        assert!(dist(w[0], w[1]) <= 30.0, "{:?} to {:?}", w[0], w[1]);
    }
}

#[test]
fn on_a_straight_line_the_limits_are_half_the_straight_width_away() {
    let s = design(&[[X0, Y0], [X0 + 1500.0, Y0]]);
    let half = sizing(&s, "bottom_width_straight_m") / 2.0;
    let fc: Value = serde_json::from_str(&s.export_limits_geojson(false).unwrap()).unwrap();
    assert_eq!(fc["type"], "FeatureCollection");
    assert_eq!(fc["screening"], SCREENING_LABEL);
    assert_eq!(
        fc["crs"]["properties"]["name"],
        "urn:ogc:def:crs:EPSG::32630"
    );
    assert_eq!(fc["features"].as_array().unwrap().len(), 2);
    // The line runs east: left is north.
    for (side, y) in [("left", Y0 + half), ("right", Y0 - half)] {
        let l = limit(&fc, side);
        well_sampled(&l);
        for p in &l {
            assert!((p[1] - y).abs() < 1e-6, "{side}: {p:?}, want y = {y}");
        }
        assert!((l[0][0] - X0).abs() < 1e-6, "{side} starts at {:?}", l[0]);
        let end = l[l.len() - 1];
        assert!(
            (end[0] - (X0 + 1500.0)).abs() < 1e-6,
            "{side} ends at {end:?}"
        );
    }
}

#[test]
fn on_an_arc_of_the_minimum_radius_the_limits_are_half_the_widened_width_away() {
    let probe = design(&[[X0, Y0], [X0 + 1500.0, Y0]]);
    let r = sizing(&probe, "min_bend_radius_m");
    let widened = sizing(&probe, "bottom_width_at_rmin_m");
    let straight = sizing(&probe, "bottom_width_straight_m");
    assert!(
        (widened - straight - sizing(&probe, "bend_widening_at_rmin_m")).abs() < 1e-9
            && widened > straight,
        "the width at the minimum radius is the straight width plus the widening there"
    );
    let half = widened / 2.0;

    // An arc of radius `r` through 120 degrees, turning left from east; a point about every 2 m.
    let centre = [X0, Y0 + r];
    let sweep = 120.0_f64.to_radians();
    let n = (r * sweep / 2.0).ceil();
    let at = |t: f64, radius: f64| {
        [
            centre[0] + radius * libm::sin(t),
            centre[1] - radius * libm::cos(t),
        ]
    };
    let line: Vec<Pt> = (0..=(n as u32))
        .map(|i| at(sweep * f64::from(i) / n, r))
        .collect();
    let s = design(&line);
    let fc: Value = serde_json::from_str(&s.export_limits_geojson(false).unwrap()).unwrap();

    // The arc turns left: the left limit is inside it, the right limit outside.
    for (side, radius) in [("left", r - half), ("right", r + half)] {
        let l = limit(&fc, side);
        well_sampled(&l);
        for p in &l {
            let d = dist(*p, centre);
            assert!(
                (d - radius).abs() < 0.05,
                "{side}: {p:?} is {d} m from the centre of the arc, want {radius}"
            );
        }
        // It starts beside the start of the line and ends beside its end.
        assert!(
            dist(l[0], at(0.0, radius)) < 2.0,
            "{side} starts at {:?}",
            l[0]
        );
        let end = l[l.len() - 1];
        assert!(dist(end, at(sweep, radius)) < 2.0, "{side} ends at {end:?}");
    }
}

#[test]
fn with_wgs84_the_limits_are_in_longitude_and_latitude() {
    let s = design(&[[X0, Y0], [X0 + 1500.0, Y0]]);
    let fc: Value = serde_json::from_str(&s.export_limits_geojson(true).unwrap()).unwrap();
    assert!(fc.get("crs").is_none(), "a WGS84 file names no CRS");
    for side in ["left", "right"] {
        for p in limit(&fc, side) {
            // UTM 30N: near 3 degrees west, 5.4 degrees north.
            assert!(
                (-3.1..-2.9).contains(&p[0]) && (5.3..5.5).contains(&p[1]),
                "{p:?}"
            );
        }
    }
}

#[test]
fn with_no_option_the_export_says_so() {
    let s = Session::new("Limits test", "2026-11-10");
    let e = s.export_limits_geojson(false).unwrap_err().to_string();
    assert!(e.contains("run the alignment stage"), "{e}");
}
