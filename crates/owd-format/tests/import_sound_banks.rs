//! Import makes a sound pair of banks. Both banks run the same way: downstream, when a centreline
//! says which way that is. Left and right are named looking downstream. Two banks that cross are
//! refused. Two banks alone give the river polygon, so "what fits" runs on them.
//!
//! Label: property

use owd_format::design::{DesignError, Role, Session};
use serde_json::json;

type Pt = [f64; 2];

const UTM30: &str = r#""crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;

/// A gently winding line 3 km long, in UTM 30N, running east, `y` metres north of the river axis.
fn east(y: f64) -> Vec<Pt> {
    (0..=30)
        .map(|i| {
            let x = 100.0 * f64::from(i);
            [500_000.0 + x, 600_000.0 + y + 40.0 * libm::sin(x / 500.0)]
        })
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

/// A new design with `items` imported in one call.
fn imported(items: &[(Role, Vec<Pt>)]) -> Session {
    let texts: Vec<(Role, String)> = items.iter().map(|(r, p)| (*r, text(p))).collect();
    let list: Vec<(Role, &str)> = texts.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let mut s = Session::new("Banks", "2026-11-10");
    s.import(&list).unwrap();
    s
}

fn line(s: &Session, role: Role) -> Vec<Pt> {
    s.document()["inputs"][role.name()]["coordinates"]
        .as_array()
        .unwrap_or_else(|| panic!("the document has no {}", role.name()))
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

/// The exterior ring of the river polygon; empty when the document has none.
fn ring(s: &Session) -> Vec<Pt> {
    s.document()["inputs"]["river_polygon"]["coordinates"][0]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

// The north bank is the left bank looking east, the way the centreline runs.
const NORTH: f64 = 100.0;
const SOUTH: f64 = -100.0;

#[test]
fn banks_drawn_in_either_direction_import_to_the_same_document() {
    let centre = (Role::Centreline, east(0.0));
    let same_way = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, east(SOUTH)),
        centre.clone(),
    ]);
    // Drawn downstream, the banks are kept as drawn.
    assert_eq!(line(&same_way, Role::LeftBank), east(NORTH));
    assert_eq!(line(&same_way, Role::RightBank), east(SOUTH));
    for (left, right, drawn) in [
        (east(NORTH), west(SOUTH), "the right bank drawn upstream"),
        (west(NORTH), east(SOUTH), "the left bank drawn upstream"),
        (west(NORTH), west(SOUTH), "both banks drawn upstream"),
    ] {
        let s = imported(&[
            (Role::LeftBank, left),
            (Role::RightBank, right),
            centre.clone(),
        ]);
        assert_eq!(s.document(), same_way.document(), "{drawn}");
    }
}

#[test]
fn with_no_centreline_the_right_bank_is_turned_to_run_the_way_of_the_left_bank() {
    let same_way = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, east(SOUTH)),
    ]);
    let opposite = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, west(SOUTH)),
    ]);
    assert_eq!(line(&opposite, Role::RightBank), east(SOUTH));
    assert_eq!(opposite.document(), same_way.document());
    // The same holds when the right bank comes in a later import.
    let mut later = imported(&[(Role::LeftBank, east(NORTH))]);
    later
        .import(&[(Role::RightBank, &text(&west(SOUTH)))])
        .unwrap();
    assert_eq!(line(&later, Role::RightBank), east(SOUTH));
}

#[test]
fn the_sides_are_named_looking_downstream() {
    let named_right = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, east(SOUTH)),
        (Role::Centreline, east(0.0)),
    ]);
    // The same lines with the two names the wrong way round.
    let named_wrong = imported(&[
        (Role::LeftBank, east(SOUTH)),
        (Role::RightBank, east(NORTH)),
        (Role::Centreline, east(0.0)),
    ]);
    assert_eq!(line(&named_wrong, Role::LeftBank), east(NORTH));
    assert_eq!(line(&named_wrong, Role::RightBank), east(SOUTH));
    assert_eq!(named_wrong.document(), named_right.document());
    // A centreline drawn the other way: downstream is west, so the south bank is the left one.
    let flows_west = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, east(SOUTH)),
        (Role::Centreline, west(0.0)),
    ]);
    assert_eq!(line(&flows_west, Role::LeftBank), west(SOUTH));
    assert_eq!(line(&flows_west, Role::RightBank), west(NORTH));
}

#[test]
fn banks_that_cross_each_other_are_refused() {
    // The left bank dips across the right bank between 1.4 km and 1.6 km.
    let mut crossing = east(NORTH);
    crossing[15][1] -= 300.0;
    let refused = |e: DesignError| match e {
        DesignError::Input(m) => {
            assert!(m.contains("cross"), "{m}");
            m
        }
        other => panic!("not an input error: {other:?}"),
    };

    let mut s = Session::new("Banks", "2026-11-10");
    let before = s.clone();
    let e = s
        .import(&[
            (Role::LeftBank, &text(&crossing)),
            (Role::RightBank, &text(&east(SOUTH))),
        ])
        .expect_err("two banks that cross must be refused");
    let m = refused(e);
    // The message says where: a point between the two crossings, in the project CRS.
    assert!(m.contains("5014") || m.contains("5015"), "{m}");
    assert_eq!(s, before, "a refused import changes nothing");

    // A bank that crosses the bank already in the document is refused too.
    let mut s = imported(&[(Role::RightBank, east(SOUTH))]);
    let before = s.clone();
    let e = s
        .import(&[(Role::LeftBank, &text(&crossing))])
        .expect_err("a bank that crosses the other bank must be refused");
    refused(e);
    assert_eq!(s, before, "a refused import changes nothing");
}

#[test]
fn two_banks_alone_give_the_river_polygon_and_what_fits_runs() {
    let mut s = imported(&[
        (Role::LeftBank, east(NORTH)),
        (Role::RightBank, west(SOUTH)),
    ]);
    s.run_fits(None).expect("what fits runs on two banks alone");
    assert!(s.fits_result().is_some());
    // The polygon is the left bank, then the right bank backwards, closed.
    let mut built = east(NORTH);
    built.extend(west(SOUTH));
    built.push(east(NORTH)[0]);
    assert_eq!(ring(&s), built);
    assert!(s.validate().is_empty(), "{:?}", s.validate());

    // A polygon that import built follows a bank that is imported again.
    s.import(&[(Role::LeftBank, &text(&east(NORTH + 20.0)))])
        .unwrap();
    assert_eq!(ring(&s)[..31], east(NORTH + 20.0)[..]);

    // A polygon that was given is kept.
    let given: Vec<Pt> = vec![
        [500_000.0, 599_800.0],
        [503_000.0, 599_800.0],
        [503_000.0, 600_200.0],
        [500_000.0, 600_200.0],
        [500_000.0, 599_800.0],
    ];
    let polygon = format!(
        r#"{{"type":"Polygon",{UTM30},"coordinates":{}}}"#,
        json!([given])
    );
    let mut s = Session::new("Banks", "2026-11-10");
    s.import(&[
        (Role::RiverPolygon, &polygon),
        (Role::LeftBank, &text(&east(NORTH))),
        (Role::RightBank, &text(&east(SOUTH))),
    ])
    .unwrap();
    assert_eq!(ring(&s), given);
    s.import(&[(Role::LeftBank, &text(&east(NORTH + 20.0)))])
        .unwrap();
    assert_eq!(ring(&s), given);
}
