//! A pool lattice line that crosses itself is never a passing design (issue #569).
//!
//! The case is synthetic: a sine-generated meander, heading = 110 deg x sin(2 pi s / 2,000 m), 8 km
//! long. The river runs backwards at each apex and its lobes nest, but the river line does not
//! cross itself. Water is a strip of 65 m round it at -8 m, ground is +2 m, on a 5 m grid that reaches 1,200 m
//! beyond the river. R_min is 340 m
//! (PIANC WG 141, CEMT class IV, quality A). One weir at 3,500 m with a lift of 0.5 m. The lattice is
//! coarse (pitch 40 m, cell 20 m, corridor 200 m) so the test runs in a debug build; the first pool's
//! search loops there as it does at pitch 12 m and cell 5 m.
//!
//! Label: property

use owd_core::alignment::loops::{first_self_crossing_m, is_self_intersecting};
use owd_core::geom::offset::{Cap, buffer_line};
use owd_core::geom::{Geometry, Pt};
use owd_format::design::export::failed_checks;
use owd_format::design::{Role, Session, Stage};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM31: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32631"}}"#;
const ORIGIN: Pt = [500_000.0, 1_000_000.0];
const WATER_HALF_M: f64 = 65.0;

/// The meander's centreline, in 5 m steps.
fn centreline() -> Vec<Pt> {
    let th0 = 110f64.to_radians();
    let (lam, len, ds) = (2000.0, 8000.0, 5.0);
    let n = (len / ds) as usize;
    let mut out = vec![ORIGIN];
    for i in 0..n {
        let th = th0 * libm::sin(2.0 * core::f64::consts::PI * (i as f64 * ds) / lam);
        let [x, y] = out[out.len() - 1];
        out.push([x + ds * libm::cos(th), y + ds * libm::sin(th)]);
    }
    out
}

/// The water strip on a 5 m grid, as a DEM would hold it: -8 m on a cell whose centre is within
/// `WATER_HALF_M` of the centreline, +2 m elsewhere; each point takes its nearest cell.
fn terrain(cl: Vec<Pt>) -> impl Fn(&[f64], &[f64]) -> Vec<f64> {
    const CELL: f64 = 5.0;
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &cl {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let pad = 1200.0;
    let (x0, y0) = (x0 - pad, y0 - pad);
    let cols = ((x1 + pad - x0) / CELL).ceil() as usize;
    let rows = ((y1 + pad - y0) / CELL).ceil() as usize;
    let mut wet = vec![false; rows * cols];
    let w2 = WATER_HALF_M * WATER_HALF_M;
    for s in cl.windows(2) {
        let ([ax, ay], [bx, by]) = (s[0], s[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let c0 = ((ax.min(bx) - WATER_HALF_M - x0) / CELL).floor().max(0.0) as usize;
        let c1 = (((ax.max(bx) + WATER_HALF_M - x0) / CELL).ceil() as usize).min(cols - 1);
        let r0 = ((ay.min(by) - WATER_HALF_M - y0) / CELL).floor().max(0.0) as usize;
        let r1 = (((ay.max(by) + WATER_HALF_M - y0) / CELL).ceil() as usize).min(rows - 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (x, y) = (x0 + (c as f64 + 0.5) * CELL, y0 + (r as f64 + 0.5) * CELL);
                let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let (px, py) = (ax + t * dx - x, ay + t * dy - y);
                if px * px + py * py <= w2 {
                    wet[r * cols + c] = true;
                }
            }
        }
    }
    move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| {
                let c = ((x - x0) / CELL).floor();
                let r = ((y - y0) / CELL).floor();
                if c < 0.0 || r < 0.0 || c >= cols as f64 || r >= rows as f64 {
                    return f64::NAN;
                }
                if wet[r as usize * cols + c as usize] {
                    -8.0
                } else {
                    2.0
                }
            })
            .collect()
    }
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32631),
    }
}

fn design(cl: &[Pt]) -> Session {
    let ring = match buffer_line(cl, 60.0, 16, Cap::Round).unwrap() {
        Geometry::Polygon(p) => p.exterior,
        other => panic!("the river came out as {other:?}"),
    };
    let mut s = Session::new("Meander", "2026-01-01");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM31},"coordinates":{}}}"#,
                json!(cl)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM31},"coordinates":{}}}"#,
                json!([ring])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "A")])
        .unwrap();
    s.set_vessel("CEMT_IV", None).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.set_dem("dem.tif", b"dem", &dem());
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s
}

fn lattice_option(s: &Session) -> Value {
    s.document()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "lattice")
        .cloned()
        .unwrap()
}

fn line_of(o: &Value) -> Vec<Pt> {
    o["line"]["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn run(weir_m: f64) -> Value {
    let cl = centreline();
    let mut s = design(&cl);
    let t = terrain(cl);
    s.run_optimise(
        &json!({
            "candidate_sites_m": [weir_m],
            "lifts_m": [0.5],
            "base_level_m": 0.0,
            "terminal_min_level_m": 0.5,
            "pitch_m": 40.0,
            "corridor_half_m": 200.0,
            "cell_m": 20.0,
        }),
        Some(&t),
    )
    .unwrap();
    let o = lattice_option(&s);
    let line = line_of(&o);
    println!(
        "weir {weir_m}: checks {} failed {:?} first crossing {:?}",
        o["checks"],
        failed_checks(&o),
        first_self_crossing_m(&line)
    );
    for p in o["details"]["pools"].as_array().unwrap() {
        println!(
            "  pool {:.0}..{:.0}: used {} closed {} because {}",
            p["start_m"].as_f64().unwrap(),
            p["end_m"].as_f64().unwrap(),
            p["used"],
            p["goal_closed"],
            p["kept_baseline_because"]
        );
    }
    o
}

/// The fault: the first pool's search line crosses itself, and the option used to pass with a looped
/// line. A line that crosses itself must never pass: here the guard moves the pool's search end and
/// the line is simple, and the check says so.
#[test]
fn a_self_crossing_lattice_line_never_passes() {
    let o = run(3500.0);
    let line = line_of(&o);
    if is_self_intersecting(&line) {
        assert!(
            failed_checks(&o).contains(&"no_self_crossing"),
            "the line crosses itself at {:?} m and the option passes",
            first_self_crossing_m(&line)
        );
    }
    assert!(!is_self_intersecting(&line), "the line crosses itself");
    assert_eq!(o["checks"]["no_self_crossing"]["ok"], json!(true));
    assert_eq!(
        o["checks"]["no_self_crossing"]["first_crossing_m"],
        Value::Null
    );
    assert!(failed_checks(&o).is_empty(), "{:?}", failed_checks(&o));
    let warnings = o["sanity"]["warnings"].to_string();
    assert!(
        warnings.contains("so its search end moved"),
        "the moved end is named: {warnings}"
    );
}
