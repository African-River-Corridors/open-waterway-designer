//! With two banks and no centreline, import cannot tell which way the river flows. The user may
//! give a downstream point: the banks then run towards it, and the sides are named looking that way.
//!
//! Label: property

use owd_format::design::{DesignError, Role, Session};
use serde_json::json;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A straight line 3 km long, in UTM 30N, running east, `y` metres north of the river axis.
fn east(y: f64) -> Vec<Pt> {
    (0..=30)
        .map(|i| [500_000.0 + 100.0 * f64::from(i), 600_000.0 + y])
        .collect()
}

fn west(y: f64) -> Vec<Pt> {
    east(y).into_iter().rev().collect()
}

fn text(pts: &[Pt]) -> String {
    format!(
        r#"{{"type":"LineString",{UTM30},"coordinates":{}}}"#,
        json!(pts)
    )
}

fn line(s: &Session, role: Role) -> Vec<Pt> {
    s.document()["inputs"][role.name()]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

// Longitude and latitude. UTM 30N has its central meridian at 3° W: easting 500 000 m is 3° W, and
// 3 km further east is near 2.973° W. Northing 600 000 m is near 5.43° N.
const WEST_END: [f64; 2] = [-3.0, 5.43];
const EAST_END: [f64; 2] = [-2.973, 5.43];

#[test]
fn a_downstream_point_gives_the_direction_and_the_sides() {
    let banks = [
        (Role::LeftBank, text(&east(100.0))),
        (Role::RightBank, text(&west(-100.0))),
    ];
    let items: Vec<(Role, &str)> = banks.iter().map(|(r, t)| (*r, t.as_str())).collect();

    // Downstream is east: the north bank is the left one, and both banks run east.
    let mut s = Session::new("Banks", "2026-11-10");
    s.import_downstream(&items, EAST_END).unwrap();
    assert_eq!(line(&s, Role::LeftBank), east(100.0));
    assert_eq!(line(&s, Role::RightBank), east(-100.0));

    // Downstream is west: the south bank is the left one, and both banks run west.
    let mut s = Session::new("Banks", "2026-11-10");
    s.import_downstream(&items, WEST_END).unwrap();
    assert_eq!(line(&s, Role::LeftBank), west(-100.0));
    assert_eq!(line(&s, Role::RightBank), west(100.0));

    // A point that is no longitude and latitude is refused, and changes nothing.
    let mut s = Session::new("Banks", "2026-11-10");
    let before = s.clone();
    let e = s
        .import_downstream(&items, [500_000.0, 600_000.0])
        .unwrap_err();
    assert!(matches!(e, DesignError::Input(_)), "{e:?}");
    assert_eq!(s, before);
}
