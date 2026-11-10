//! The plan cut in the document: the cut in plan of every design line option, with no terrain and
//! no water level, on a straight synthetic river given in UTM 30N.
//!
//! Every expected number is closed-form. The fairway width at the water surface, W, is worked out
//! here from the sizing the test set up, by the stated rule:
//! W = bottom width + 2 x side slope x design depth (a straight line has no bend widening).
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::volumes::sections::DEFAULT_WET_SLOPE;
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
/// The length of the river centreline, and so of the design line.
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

/// A design with a straight centreline along +x and no river polygon yet, sized for CEMT IV.
fn sized() -> Session {
    let centre: Vec<Pt> = (0..=40).map(|i| [X0 + 50.0 * i as f64, Y0]).collect();
    let mut s = Session::new("Straight river", "2026-11-10");
    s.import(&[(
        Role::Centreline,
        &format!(
            r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
            json!(centre)
        ),
    )])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s
}

fn with_polygon(mut s: Session, w: f64, islands: &[Vec<Pt>]) -> Session {
    // The river is longer than the line at both ends, so the fairway ends inside it.
    let mut rings = vec![rect(-100.0, L + 100.0, -w / 2.0, w / 2.0)];
    rings.extend(islands.iter().cloned());
    s.import(&[(
        Role::RiverPolygon,
        &format!(
            r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
            json!(rings)
        ),
    )])
    .unwrap();
    s
}

/// A straight river `w` wide with the given islands, sized and aligned. No terrain, no water level.
fn design(w: f64, islands: &[Vec<Pt>]) -> Session {
    let mut s = with_polygon(sized(), w, islands);
    s.align(
        AlignMethod::RollingCircle,
        &json!({"use_water": false}),
        None,
    )
    .unwrap();
    let d = s.document();
    assert!(d["inputs"]["dem"].is_null() && d["inputs"]["water_level"].is_null());
    // The design line is the straight centreline, so its length is L and it has no bend.
    let line = d["options"][0]["line"]["coordinates"].as_array().unwrap();
    assert!(line.iter().all(|p| p[1].as_f64() == Some(Y0)));
    assert_eq!(line[0][0].as_f64(), Some(X0));
    assert_eq!(line[line.len() - 1][0].as_f64(), Some(X0 + L));
    s
}

/// W, from the sizing in the document.
fn surface_width(s: &Session) -> f64 {
    let r = &s.document()["stages"]["sizing"]["results"];
    r["bottom_width_straight_m"].as_f64().unwrap()
        + 2.0 * DEFAULT_WET_SLOPE * r["design_depth_m"].as_f64().unwrap()
}

fn row(s: &Session) -> Value {
    let options = s.plan_cut_result().expect("the plan cut has run")["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(options.len(), 1);
    options[0].clone()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

/// The area of a GeoJSON polygon: its shell less its holes.
fn polygon_area(rings: &Value) -> f64 {
    let ring_area = |ring: &Value| -> f64 {
        let p: Vec<Pt> = ring
            .as_array()
            .unwrap()
            .iter()
            .map(|q| [num(&q[0]) - X0, num(&q[1]) - Y0])
            .collect();
        (p.windows(2)
            .map(|s| s[0][0] * s[1][1] - s[1][0] * s[0][1])
            .sum::<f64>()
            / 2.0)
            .abs()
    };
    let rings = rings.as_array().unwrap();
    ring_area(&rings[0]) - rings[1..].iter().map(ring_area).sum::<f64>()
}

#[test]
fn a_river_narrower_than_the_fairway_gives_a_cut_with_no_terrain_and_no_water_level() {
    let w = 30.0;
    let mut s = design(w, &[]);
    let wide = surface_width(&s);
    assert!(
        wide > w + 10.0,
        "the test needs a fairway wider than the river: {wide}"
    );
    s.run_plan_cut(&json!({})).unwrap();

    // The fairway ends square at the two ends of the line, inside the river: the length is L.
    let want = (wide - w) * L;
    let r = row(&s);
    assert_eq!(r["id"], s.document()["options"][0]["id"]);
    assert!(
        close(num(&r["area_m2"]), want),
        "{} != {want}",
        r["area_m2"]
    );
    assert!(close(num(&r["by_bank"]["left_m2"]), want / 2.0));
    assert!(close(num(&r["by_bank"]["right_m2"]), want / 2.0));
    assert_eq!(r["pieces"], 2);
    assert!(close(num(&r["largest_piece_m2"]), want / 2.0));

    // The cut itself, for export: a MultiPolygon in the project CRS, one polygon per piece.
    assert_eq!(r["cut"]["type"], "MultiPolygon");
    let polys = r["cut"]["coordinates"].as_array().unwrap();
    assert_eq!(polys.len(), 2);
    let drawn: f64 = polys.iter().map(polygon_area).sum();
    assert!(close(drawn, want), "{drawn} != {want}");

    // What the result was made with is stored beside it.
    let st = &s.document()["stages"]["plan_cut"];
    assert_eq!(st["settings"]["min_piece_m2"], 50.0);
    assert_eq!(st["settings"]["wet_slope"], DEFAULT_WET_SLOPE);
    assert!(st["depends_on"].is_string());

    // Plan only, said on the result and on each option's row.
    for label in [&st["results"]["label"], &r["label"]] {
        let text = label.as_str().expect("a plain label").to_lowercase();
        assert!(text.contains("plan only"), "{text}");
        assert!(text.contains("no terrain"), "{text}");
    }
}

#[test]
fn a_river_wider_than_the_fairway_everywhere_has_no_cut() {
    let mut s = design(200.0, &[]);
    assert!(surface_width(&s) < 190.0);
    s.run_plan_cut(&json!({})).unwrap();
    let r = row(&s);
    assert_eq!(num(&r["area_m2"]), 0.0);
    assert_eq!(num(&r["by_bank"]["left_m2"]), 0.0);
    assert_eq!(num(&r["by_bank"]["right_m2"]), 0.0);
    assert_eq!(r["pieces"], 0);
    assert_eq!(num(&r["largest_piece_m2"]), 0.0);
    assert_eq!(r["cut"]["type"], "MultiPolygon");
    assert_eq!(r["cut"]["coordinates"], json!([]));
}

#[test]
fn an_island_inside_the_fairway_is_cut() {
    // A 60 m by 15 m island, 5 m to 20 m left of the line. The fairway is wider than that.
    let mut s = design(200.0, &[rect(400.0, 460.0, 5.0, 20.0)]);
    assert!(surface_width(&s) / 2.0 > 21.0);
    s.run_plan_cut(&json!({})).unwrap();
    let r = row(&s);
    assert!(close(num(&r["area_m2"]), 60.0 * 15.0), "{}", r["area_m2"]);
    assert!(close(num(&r["by_bank"]["left_m2"]), 60.0 * 15.0));
    assert_eq!(num(&r["by_bank"]["right_m2"]), 0.0);
    assert_eq!(r["pieces"], 1);
}

#[test]
fn a_piece_under_min_piece_m2_is_not_a_cut_and_the_setting_is_kept() {
    // A 6 m by 6 m island: 36 m2, under the default of 50 m2.
    let island = rect(400.0, 406.0, 5.0, 11.0);
    let mut s = design(200.0, &[island]);
    s.run_plan_cut(&json!({})).unwrap();
    let r = row(&s);
    assert_eq!(r["pieces"], 0);
    assert_eq!(num(&r["area_m2"]), 0.0);
    assert_eq!(r["dropped"]["pieces"], 1);
    assert!(close(num(&r["dropped"]["area_m2"]), 36.0));

    s.run_plan_cut(&json!({"min_piece_m2": 10.0})).unwrap();
    let r = row(&s);
    assert_eq!(r["pieces"], 1);
    assert!(close(num(&r["area_m2"]), 36.0));
    assert_eq!(
        s.document()["stages"]["plan_cut"]["settings"]["min_piece_m2"],
        10.0
    );

    // A later run with no setting keeps the one in the document.
    s.run_plan_cut(&json!({})).unwrap();
    assert_eq!(row(&s)["pieces"], 1);
}

#[test]
fn a_setting_that_is_not_known_or_not_usable_is_refused() {
    let mut s = design(30.0, &[]);
    for (bad, word) in [
        (json!({"min_piece": 10.0}), "min_piece"),
        (json!({"min_piece_m2": -1.0}), "min_piece_m2"),
        (json!({"min_piece_m2": "ten"}), "min_piece_m2"),
        (json!({"station_m": 0.0}), "station_m"),
    ] {
        let e = s.run_plan_cut(&bad).unwrap_err().to_string();
        assert!(e.contains(word), "{bad}: {e}");
    }
    assert!(
        s.plan_cut_result().is_none(),
        "a refused run stores nothing"
    );
}

#[test]
fn the_plan_cut_says_what_it_needs() {
    // No river polygon.
    let mut s = sized();
    let e = s.run_plan_cut(&json!({})).unwrap_err().to_string();
    assert!(e.contains("river polygon"), "{e}");

    // No design line option.
    let mut s = with_polygon(sized(), 30.0, &[]);
    let e = s.run_plan_cut(&json!({})).unwrap_err().to_string();
    assert!(e.contains("design line"), "{e}");

    // A sizing that is not fresh.
    let mut s = design(30.0, &[]);
    s.set_vessel("CEMT_Va", Some(2.8)).unwrap();
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Stale);
    let e = s.run_plan_cut(&json!({})).unwrap_err().to_string();
    assert!(e.contains("sizing"), "{e}");
}

#[test]
fn the_plan_cut_has_its_own_status_and_is_not_a_stage_of_the_chain() {
    let mut s = design(30.0, &[]);
    assert_eq!(s.plan_cut_status(), StageStatus::NotRun);
    let chain_before = s.status();
    s.run_plan_cut(&json!({})).unwrap();
    assert_eq!(s.plan_cut_status(), StageStatus::Fresh);
    // The four stages stand as they did, and the document still opens.
    assert_eq!(s.status(), chain_before);
    assert_eq!(Stage::ALL.len(), 4);
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(again.plan_cut_status(), StageStatus::Fresh);

    // A new river polygon makes it stale. The loop that runs what is due leaves it alone.
    let mut s = with_polygon(s, 40.0, &[]);
    assert_eq!(s.plan_cut_status(), StageStatus::Stale);
    let kept = s.plan_cut_result().cloned();
    s.run_stale_with(None).unwrap();
    assert_eq!(s.plan_cut_status(), StageStatus::Stale);
    assert_eq!(s.plan_cut_result().cloned(), kept);

    // A new sizing makes it stale too.
    let mut s = design(30.0, &[]);
    s.run_plan_cut(&json!({})).unwrap();
    s.set_vessel("CEMT_Va", Some(2.8)).unwrap();
    assert_eq!(s.plan_cut_status(), StageStatus::Stale);
}

#[test]
fn the_cut_exports_as_geojson_one_feature_per_piece() {
    let w = 30.0;
    let mut s = design(w, &[]);
    assert!(
        s.export_plan_cut_geojson(false).is_err(),
        "nothing to export before a run"
    );
    s.run_plan_cut(&json!({})).unwrap();
    let want = (surface_width(&s) - w) * L;

    let fc: Value = serde_json::from_str(&s.export_plan_cut_geojson(false).unwrap()).unwrap();
    assert_eq!(fc["type"], "FeatureCollection");
    assert!(
        fc["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("plan only")
    );
    assert!(
        fc["crs"]["properties"]["name"]
            .as_str()
            .unwrap()
            .ends_with("32630")
    );
    let features = fc["features"].as_array().unwrap();
    assert_eq!(features.len(), 2);
    let mut banks = Vec::new();
    for f in features {
        assert_eq!(f["geometry"]["type"], "Polygon");
        let p = &f["properties"];
        assert_eq!(p["option"], s.document()["options"][0]["id"]);
        assert!(
            p["label"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("plan only")
        );
        assert!(close(num(&p["area_m2"]), want / 2.0));
        assert!(close(
            polygon_area(&f["geometry"]["coordinates"]),
            want / 2.0
        ));
        banks.push(p["bank"].as_str().unwrap().to_string());
        // Left of a line that runs east is north.
        let north = num(&f["geometry"]["coordinates"][0][0][1]) > Y0;
        assert_eq!(p["bank"] == "left", north);
    }
    banks.sort();
    assert_eq!(banks, ["left", "right"]);

    // In longitude and latitude there is no `crs` member (RFC 7946).
    let fc: Value = serde_json::from_str(&s.export_plan_cut_geojson(true).unwrap()).unwrap();
    assert!(fc.get("crs").is_none());
    let lon = num(&fc["features"][0]["geometry"]["coordinates"][0][0][0]);
    assert!((-6.0..=0.0).contains(&lon), "UTM zone 30N: {lon}");
}
