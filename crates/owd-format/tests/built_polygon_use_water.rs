//! A river polygon that the import built from the two banks does not change the alignment's
//! default. With `use_water` unset, the rolling circle keeps to the water only when the user gave
//! the polygon. An explicit `use_water: true` still uses a built one.
//!
//! Label: property

use owd_format::design::{AlignMethod, Role, Session, Stage};
use serde_json::{Value, json};

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A bank of a winding river, 6 km long: a sine of 600 m amplitude and 1.2 km wavelength, `off`
/// metres to the north of the middle. Its bends (radius about 60 m) are far tighter than the 255 m
/// the vessel needs, so a line held in the water differs from one that is not.
fn bank(off: f64) -> Vec<Pt> {
    (0..=600)
        .map(|i| {
            let x = 10.0 * f64::from(i);
            let y = 600.0 * libm::sin(2.0 * std::f64::consts::PI * x / 1200.0) + off;
            [500_000.0 + x, 600_000.0 + y]
        })
        .collect()
}

fn line_text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

/// The two banks imported, with `polygon` given as well when it is `Some`, and the channel sized.
fn design(polygon: Option<Vec<Pt>>) -> Session {
    let (left, right) = (line_text(&bank(75.0)), line_text(&bank(-75.0)));
    let mut items = vec![
        (Role::LeftBank, left.as_str()),
        (Role::RightBank, right.as_str()),
    ];
    let poly_text = polygon.map(|ring| {
        format!(
            r#"{{"type":"Polygon",{UTM30},"coordinates":[{}]}}"#,
            json!(ring)
        )
    });
    if let Some(p) = &poly_text {
        items.push((Role::RiverPolygon, p.as_str()));
    }
    let mut s = Session::new("Built polygon", "2026-11-10");
    s.import(&items).unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

/// The rolling-circle line with `settings`.
fn rolled(mut s: Session, settings: Value) -> Value {
    s.align(AlignMethod::RollingCircle, &settings, None)
        .unwrap();
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "rolling_circle")
        .expect("a rolling-circle option")["line"]
        .clone()
}

/// The same design with its river polygon set to null.
fn without_polygon(s: &Session) -> Session {
    let mut doc: Value = serde_json::from_str(&s.to_text()).unwrap();
    doc["inputs"]["river_polygon"] = Value::Null;
    Session::open(&doc.to_string()).unwrap()
}

#[test]
fn a_built_polygon_leaves_the_default_line_as_it_was_without_one() {
    let built = design(None);
    assert!(
        built.document()["inputs"]["river_polygon"].is_object(),
        "the import builds the polygon from the two banks"
    );
    let plain = without_polygon(&built);
    let with_built = rolled(built.clone(), json!({}));
    assert_eq!(
        with_built,
        rolled(plain.clone(), json!({})),
        "the built polygon changed the default rolling-circle line"
    );
    // Asked for, the built polygon is used: the line held in the water is another line.
    assert_ne!(
        rolled(built, json!({"use_water": true})),
        rolled(plain, json!({})),
        "`use_water: true` did not use the built polygon"
    );
}

#[test]
fn a_polygon_the_user_gave_keeps_the_water_aware_default() {
    // The ring of the two banks with one more vertex on its closing side: not the built polygon.
    let (l, r) = (bank(75.0), bank(-75.0));
    let mut ring = l.clone();
    ring.extend(r.iter().rev());
    let (a, b) = (ring[ring.len() - 1], ring[0]);
    ring.push([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    ring.push(ring[0]);
    let given = design(Some(ring));
    assert_eq!(
        rolled(given.clone(), json!({})),
        rolled(given, json!({"use_water": true})),
        "a polygon the user gave no longer turns the water-aware default on"
    );
}
