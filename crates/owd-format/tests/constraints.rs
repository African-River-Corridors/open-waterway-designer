//! Constraints (spec 0017, ticket 13): no-go areas and the cut-rate multiplier.
//!
//! - The input: typed, read from GeoJSON, set through one session function, each refusal by code.
//! - What the line touches: each no-go area the line or its channel footprint enters, with the
//!   chainage from and to and the area touched, on synthetic boxes worked by hand.
//! - Weighted volumes: the cut weighted by the multiplier beside the plain cut, which does not move.
//! - A document with no constraints opens, saves and designs as before.
//!
//! Every polygon and raster here is synthetic, built in code.
//!
//! Label: golden (the hand-worked areas and volumes), property (the rest)

use owd_core::rules::engine;
use owd_format::design::constraints::{
    self, Area, Constraints, ConstraintsEdit, MultiplierFile, NoGoArea, constraints_touched,
    no_go_from_geojson, read_constraints,
};
use owd_format::design::cost::{CutVolumes, RatesTable, cost};
use owd_format::design::dem_source::write_geotiff;
use owd_format::design::sizing::{ChannelSize, SizingIn, size};
use owd_format::design::terrain::dem_sampler;
use owd_format::design::volumes_stage::{VolumesIn, volumes_stage, volumes_stage_with};
use owd_format::design::water_stage::WaterIn;
use owd_format::design::{AlignMethod, Session, Stage, StageStatus, VolumesSettings};
use owd_format::geotiff::{self, Dem};
use owd_format::types::{Chainage, Polyline, RiverPolygon, TerrainSampler, WaterProfile};
use serde_json::{Value, json};
use std::path::PathBuf;

fn web() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/web")
}

fn example() -> (String, Session) {
    let text = std::fs::read_to_string(web().join("river-example.owd")).unwrap();
    let s = Session::open(&text).unwrap();
    (text, s)
}

fn dem() -> Dem {
    geotiff::read_bytes(&std::fs::read(web().join("river-dem.tif")).unwrap()).unwrap()
}

/// A raster on the example DEM's grid whose every cell is `f(x, y)` at the cell centre.
fn raster(f: &dyn Fn(f64, f64) -> f64, epsg: u32) -> Vec<u8> {
    let mut d = dem();
    let [a, _, c, _, e, ff] = d.geotransform;
    for r in 0..d.height {
        for col in 0..d.width {
            let (x, y) = (c + a * (col as f64 + 0.5), ff + e * (r as f64 + 0.5));
            d.data[r * d.width + col] = f(x, y);
        }
    }
    d.nodata = None;
    d.epsg = Some(epsg);
    write_geotiff(&d).unwrap()
}

fn ring(x0: f64, x1: f64, y0: f64, y1: f64) -> Polyline {
    Polyline(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]])
}

fn area(name: &str, shell: Polyline, holes: Vec<Polyline>) -> NoGoArea {
    NoGoArea {
        name: name.into(),
        kind: "protected area".into(),
        polygon: Area { shell, holes },
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn code_of(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// --- what the line touches ---------------------------------------------------------------------

fn straight() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [1000.0, 0.0]]
}

#[test]
fn a_box_across_the_line_is_touched_from_its_start_to_its_end_with_the_area_by_hand() {
    // The footprint is 20 m each side of the line; the box is x 400..600, y -10..50: it holds the
    // footprint from y -10 to 20, 200 m long: 200 × 30 = 6000 m².
    let no_go = [area("box", ring(400.0, 600.0, -10.0, 50.0), vec![])];
    let t = constraints_touched(&straight(), 20.0, &no_go);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].name, "box");
    assert_eq!(t[0].kind, "protected area");
    assert_eq!(t[0].from, Chainage(400.0));
    assert_eq!(t[0].to, Chainage(600.0));
    assert!(close(t[0].area_m2, 6000.0), "{}", t[0].area_m2);
}

#[test]
fn a_box_beside_the_footprint_is_not_touched_and_one_the_footprint_only_grazes_is() {
    let clear = [area("clear", ring(400.0, 600.0, 25.0, 80.0), vec![])];
    assert!(constraints_touched(&straight(), 20.0, &clear).is_empty());
    // The line itself does not enter it; its footprint does (y 15..20): 100 × 5 = 500 m².
    let graze = [area("graze", ring(100.0, 200.0, 15.0, 80.0), vec![])];
    let t = constraints_touched(&straight(), 20.0, &graze);
    assert_eq!(t.len(), 1);
    assert!(close(t[0].area_m2, 500.0), "{}", t[0].area_m2);
}

#[test]
fn a_hole_in_a_no_go_area_is_not_no_go_so_the_line_enters_it_twice() {
    let no_go = [area(
        "ring",
        ring(300.0, 700.0, -50.0, 50.0),
        vec![ring(400.0, 600.0, -40.0, 40.0)],
    )];
    let t = constraints_touched(&straight(), 20.0, &no_go);
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!((t[0].from, t[0].to), (Chainage(300.0), Chainage(400.0)));
    assert_eq!((t[1].from, t[1].to), (Chainage(600.0), Chainage(700.0)));
    for x in &t {
        assert!(close(x.area_m2, 100.0 * 40.0), "{}", x.area_m2);
    }
}

// --- the input ---------------------------------------------------------------------------------

fn geojson() -> Value {
    json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"name": "north field", "kind": "farmland"},
         "geometry": {"type": "Polygon", "coordinates": [
            [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0], [0.0, 0.0]],
            [[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 10.0]]]}},
        {"type": "Feature", "properties": {"name": "site b", "kind": "heritage"},
         "geometry": {"type": "Polygon", "coordinates": [
            [[200.0, 0.0], [300.0, 0.0], [300.0, 50.0], [200.0, 0.0]]]}}
    ]})
}

#[test]
fn no_go_areas_are_read_from_geojson_with_a_name_a_kind_and_their_holes() {
    let a = no_go_from_geojson(&geojson()).unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].name, "north field");
    assert_eq!(a[0].kind, "farmland");
    assert_eq!(a[0].polygon.holes.len(), 1);
    assert_eq!(a[1].kind, "heritage");
}

#[test]
fn each_bad_input_is_refused_by_its_code_and_changes_nothing() {
    let (text, mut s) = example();
    let try_set = |s: &mut Session, no_go: Vec<NoGoArea>| {
        code_of(
            s.set_constraints(ConstraintsEdit {
                no_go: Some(no_go),
                multiplier: None,
            })
            .unwrap_err(),
        )
    };
    let ok = || area("a", ring(0.0, 10.0, 0.0, 10.0), vec![]);
    let mut nameless = ok();
    nameless.name = "  ".into();
    assert!(try_set(&mut s, vec![nameless]).contains("CONSTRAINTS_NAME_MISSING"));
    let mut kindless = ok();
    kindless.kind = String::new();
    assert!(try_set(&mut s, vec![kindless]).contains("CONSTRAINTS_KIND_MISSING"));
    assert!(try_set(&mut s, vec![ok(), ok()]).contains("CONSTRAINTS_NAME_DUPLICATE"));
    let line = area(
        "flat",
        Polyline(vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]]),
        vec![],
    );
    assert!(try_set(&mut s, vec![line]).contains("CONSTRAINTS_POLYGON_INVALID"));
    let nan = area(
        "nan",
        Polyline(vec![[0.0, 0.0], [f64::NAN, 0.0], [5.0, 5.0]]),
        vec![],
    );
    assert!(try_set(&mut s, vec![nan]).contains("CONSTRAINTS_POLYGON_INVALID"));
    assert_eq!(s.to_text(), text, "a refusal changes nothing");

    // A key the input does not know is refused, so a setting is never lost in silence.
    let e = read_constraints(&json!({"no_go": [], "multipler": null})).unwrap_err();
    assert_eq!(e.code, "CONSTRAINTS_INVALID");
    // A feature with no name, or one that is not a polygon.
    let mut g = geojson();
    g["features"][0]["properties"] = json!({"kind": "farmland"});
    assert_eq!(
        no_go_from_geojson(&g).unwrap_err().code,
        "CONSTRAINTS_NAME_MISSING"
    );
    let mut g = geojson();
    g["features"][1]["geometry"] = json!({"type": "LineString", "coordinates": [[0, 0], [1, 1]]});
    assert_eq!(
        no_go_from_geojson(&g).unwrap_err().code,
        "CONSTRAINTS_POLYGON_INVALID"
    );
}

#[test]
fn setting_no_go_areas_stores_them_marks_the_stages_that_read_them_stale_and_clearing_restores_the_bytes()
 {
    let (text, mut s) = example();
    assert_eq!(s.constraints().unwrap(), None);
    let c = s
        .set_constraints(ConstraintsEdit {
            no_go: Some(no_go_from_geojson(&geojson()).unwrap()),
            multiplier: None,
        })
        .unwrap();
    assert_eq!(c.no_go.len(), 2);
    assert_eq!(s.constraints().unwrap(), Some(c.clone()));
    assert_eq!(
        read_constraints(&s.document()["inputs"][constraints::INPUT]).unwrap(),
        c
    );
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Stale);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
    assert_eq!(s.stage_status(Stage::Sizing), StageStatus::Fresh);
    // Opened again from its text, it reads the same.
    let back = Session::open(&s.to_text()).unwrap();
    assert_eq!(back.constraints().unwrap(), Some(c));
    // Clearing every part removes the input: the document is what it was, byte for byte.
    s.set_constraints(ConstraintsEdit {
        no_go: Some(vec![]),
        multiplier: Some(None),
    })
    .unwrap();
    assert_eq!(s.constraints().unwrap(), None);
    assert_eq!(s.to_text(), text);
}

#[test]
fn the_multiplier_raster_is_recorded_by_hash_like_the_dem_and_checked_cell_by_cell() {
    let (_, mut s) = example();
    let bytes = raster(&|_, _| 2.0, 32630);
    let c = s
        .set_constraints(ConstraintsEdit {
            no_go: None,
            multiplier: Some(Some(MultiplierFile {
                path: "cost-weights.tif",
                bytes: &bytes,
            })),
        })
        .unwrap();
    let m = c.cut_rate_multiplier.unwrap();
    assert_eq!(m.path, "cost-weights.tif");
    assert_eq!(m.sha256, owd_format::design::hash::sha256_hex(&bytes));
    assert_eq!(m.crs, "EPSG:32630");
    assert_eq!(s.multiplier_path(), Some("cost-weights.tif"));

    // Another CRS than WGS84 or the project's is refused, as for the DEM.
    let other = raster(&|_, _| 2.0, 32631);
    let e = s
        .set_constraints(ConstraintsEdit {
            no_go: None,
            multiplier: Some(Some(MultiplierFile {
                path: "x.tif",
                bytes: &other,
            })),
        })
        .unwrap_err();
    assert!(code_of(e).contains("CONSTRAINTS_RASTER_CRS"));
    // A negative cell is refused: a multiplier is zero or more.
    let negative = raster(&|x, _| if x > 0.0 { -1.0 } else { 1.0 }, 32630);
    let e = s
        .set_constraints(ConstraintsEdit {
            no_go: None,
            multiplier: Some(Some(MultiplierFile {
                path: "x.tif",
                bytes: &negative,
            })),
        })
        .unwrap_err();
    assert!(code_of(e).contains("CONSTRAINTS_RASTER_VALUE"));
    let e = s
        .set_constraints(ConstraintsEdit {
            no_go: None,
            multiplier: Some(Some(MultiplierFile {
                path: "x.tif",
                bytes: b"not a tiff",
            })),
        })
        .unwrap_err();
    assert!(code_of(e).contains("CONSTRAINTS_RASTER_UNREADABLE"));
    // The recorded file is still the one set first.
    assert_eq!(s.multiplier_path(), Some("cost-weights.tif"));

    // Opened again, the raster must be attached by the file whose hash was recorded.
    let mut back = Session::open(&s.to_text()).unwrap();
    let e = back.attach_multiplier(&negative).unwrap_err();
    assert!(code_of(e).contains("CONSTRAINTS_RASTER_CHANGED"));
    back.attach_multiplier(&bytes).unwrap();
}

// --- weighted volumes --------------------------------------------------------------------------

fn dem_terrain(d: &Dem) -> impl Fn(&[f64], &[f64]) -> Vec<f64> + '_ {
    dem_sampler(d, Some(32630)).unwrap()
}

#[test]
fn a_document_with_no_constraints_opens_saves_and_designs_as_before() {
    for name in [
        "river-example.owd",
        "river-reroute.owd",
        "river-rolling-circle.owd",
    ] {
        let text = std::fs::read_to_string(web().join(name)).unwrap();
        let s = Session::open(&text).unwrap();
        assert_eq!(s.to_text(), text, "{name} saves byte for byte");
        assert_eq!(s.constraints().unwrap(), None);
    }
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/synthetic-river.owd"),
    )
    .unwrap();
    assert_eq!(Session::open(&text).unwrap().to_text(), text);

    // The design runs as it did: the stored volumes come back to the last digit, with no weighted
    // cut, and the design line touches nothing.
    let (_, mut s) = example();
    let before = s.document()["stages"]["volumes"]["results"].clone();
    let d = dem();
    let t = dem_terrain(&d);
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    let after = &s.document()["stages"]["volumes"]["results"];
    for k in ["dredge_m3", "excav_m3", "total_m3", "max_face_h_m"] {
        assert_eq!(after[k], before[k], "{k}");
    }
    let sections = after["sections"].to_string();
    assert!(
        !sections.contains("weighted"),
        "no weighted cut without a multiplier"
    );
    assert_eq!(
        s.document()["stages"]["alignment"]["results"]["constraints_touched"],
        json!([])
    );
}

#[test]
fn a_multiplier_of_one_gives_weighted_volumes_equal_to_the_plain_ones_and_moves_no_plain_number() {
    let (_, mut plain) = example();
    let d = dem();
    let t = dem_terrain(&d);
    plain
        .run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();

    let (_, mut s) = example();
    let ones = raster(&|_, _| 1.0, 32630);
    s.set_constraints(ConstraintsEdit {
        no_go: None,
        multiplier: Some(Some(MultiplierFile {
            path: "ones.tif",
            bytes: &ones,
        })),
    })
    .unwrap();
    // The alignment is stale (its inputs changed): run it again, then the volumes.
    s.run_stage_with(Stage::Alignment, None).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    let p = &plain.document()["stages"]["volumes"]["results"]["sections"];
    let w = &s.document()["stages"]["volumes"]["results"]["sections"];
    for (a, b) in p["stations"]
        .as_array()
        .unwrap()
        .iter()
        .zip(w["stations"].as_array().unwrap())
    {
        assert_eq!(a["dredge_area_m2"], b["dredge_area_m2"]);
        assert_eq!(a["excav_area_m2"], b["excav_area_m2"]);
        assert_eq!(b["weighted_dredge_area_m2"], b["dredge_area_m2"]);
        assert_eq!(b["weighted_excav_area_m2"], b["excav_area_m2"]);
    }
    for (a, b) in p["by_reach"]
        .as_array()
        .unwrap()
        .iter()
        .zip(w["by_reach"].as_array().unwrap())
    {
        assert_eq!(a["dredge_m3"], b["dredge_m3"]);
        assert_eq!(b["weighted_dredge_m3"], b["dredge_m3"]);
        assert_eq!(b["weighted_excav_m3"], b["excav_m3"]);
        assert_eq!(b["weighted_total_m3"], b["total_m3"]);
    }
    assert_eq!(w["totals"]["weighted_total_m3"], w["totals"]["total_m3"]);

    // The cost says it used the weighted volumes; with a multiplier of one they are the plain ones.
    let rates: RatesTable = serde_json::from_value(json!({
        "dredge_per_m3": {"amount": 10.0, "currency": "USD"},
        "dry_cut_per_m3": {"amount": 5.0, "currency": "USD"},
        "disposal_per_m3": {"amount": 2.0, "currency": "USD"},
        "haul_per_m3_km": {"amount": 1.0, "currency": "USD"},
        "haul_km": 3.0
    }))
    .unwrap();
    plain.set_rates(Some(rates.clone())).unwrap();
    s.set_rates(Some(rates)).unwrap();
    let cp = cost(&plain.cost_input().unwrap()).unwrap();
    let cw = cost(&s.cost_input().unwrap()).unwrap();
    assert_eq!(cp.cut_volumes, CutVolumes::Plain);
    assert_eq!(cw.cut_volumes, CutVolumes::Weighted);
    assert!(close(cw.totals.total.amount, cp.totals.total.amount));
}

#[test]
fn a_named_multiplier_that_is_not_attached_is_refused_by_the_volumes_never_read_as_one() {
    let (_, mut s) = example();
    let ones = raster(&|_, _| 1.0, 32630);
    s.set_constraints(ConstraintsEdit {
        no_go: None,
        multiplier: Some(Some(MultiplierFile {
            path: "ones.tif",
            bytes: &ones,
        })),
    })
    .unwrap();
    s.run_stage_with(Stage::Alignment, None).unwrap();
    let mut reopened = Session::open(&s.to_text()).unwrap();
    let d = dem();
    let t = dem_terrain(&d);
    let e = reopened
        .run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap_err();
    assert!(code_of(&e).contains("VOLUMES_NO_MULTIPLIER"), "{e}");
    assert!(code_of(&e).contains("ones.tif"), "{e}");
}

#[test]
fn the_rolling_circle_does_not_steer_but_its_line_reports_the_no_go_area_it_enters() {
    let (_, mut s) = example();
    let line = s.document()["stages"]["alignment"]["results"]["line"]["coordinates"].clone();
    let pts: Vec<[f64; 2]> = serde_json::from_value(line).unwrap();
    let mid = pts[pts.len() / 2];
    let no_go = vec![area(
        "middle",
        ring(mid[0] - 30.0, mid[0] + 30.0, mid[1] - 30.0, mid[1] + 30.0),
        vec![],
    )];
    s.set_constraints(ConstraintsEdit {
        no_go: Some(no_go),
        multiplier: None,
    })
    .unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    let touched = &s.document()["stages"]["alignment"]["results"]["constraints_touched"];
    assert_eq!(touched.as_array().unwrap().len(), 1, "{touched}");
    assert_eq!(touched[0]["name"], "middle");
    assert_eq!(touched[0]["kind"], "protected area");
    assert!(touched[0]["area_m2"].as_f64().unwrap() > 0.0);
    assert!(touched[0]["from"].as_f64().unwrap() < touched[0]["to"].as_f64().unwrap());
}

// --- the volumes stage alone: a ×2 patch over half the stations ---------------------------------

const TERRAIN_M: f64 = 7.5;
const WATER_M: f64 = 9.0;

fn flat(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![TERRAIN_M; xs.len()]
}

fn channel() -> ChannelSize {
    let mut input: SizingIn = serde_json::from_value(json!({
        "rule_sets": [{"id": "pianc-wg141", "version": "0.3.0"}, {"id": "owd", "version": "0.3.0"}],
        "set_inputs": [{"name": "se_quality", "value": "B"}],
        "vessel_standard_id": "CEMT_IV",
        "draught_m": 2.5,
    }))
    .unwrap();
    for set in &mut input.rule_sets {
        set.hash = engine::built_in()
            .find(&set.id, &set.version)
            .map(|s| s.hash.clone());
    }
    size(&input, engine::built_in()).unwrap()
}

fn volumes_input() -> VolumesIn {
    let line = Polyline((0..=20).map(|i| [100.0 * i as f64, 0.0]).collect());
    VolumesIn {
        design_line: line.clone(),
        size: channel(),
        water: WaterIn {
            centreline: line.clone(),
            design_line: line,
            given: Some(WaterProfile::from_pairs(&[[0.0, WATER_M]])),
            pools: vec![],
            beyond_pools: Default::default(),
            step_m: 20.0,
        },
        river_polygon: Some(RiverPolygon {
            shell: ring(-100.0, 2100.0, -200.0, 200.0),
            holes: vec![],
        }),
        settings: VolumesSettings::default(),
        profile_at: vec![],
    }
}

#[test]
fn a_patch_of_two_over_half_the_stations_weighs_their_cut_twice_by_hand() {
    let input = volumes_input();
    let plain = volumes_stage(&input, &flat).unwrap();
    // ×2 on the far half of the line (x >= 1000 m); elsewhere no data, which is 1.
    let patch = |xs: &[f64], _: &[f64]| -> Vec<f64> {
        xs.iter()
            .map(|&x| if x >= 1000.0 { 2.0 } else { f64::NAN })
            .collect()
    };
    let w = volumes_stage_with(&input, &flat, Some(&patch)).unwrap();
    assert_eq!(w.stations.len(), plain.stations.len());
    let (mut lo, mut hi) = (0usize, 0usize);
    for (p, s) in plain.stations.iter().zip(&w.stations) {
        assert_eq!(s.dredge_area_m2, p.dredge_area_m2, "plain does not move");
        assert_eq!(s.excav_area_m2, p.excav_area_m2, "plain does not move");
        let k = if s.chainage_m >= 1000.0 {
            hi += 1;
            2.0
        } else {
            lo += 1;
            1.0
        };
        assert!(close(
            s.weighted_dredge_area_m2.unwrap(),
            k * p.dredge_area_m2
        ));
        assert_eq!(p.weighted_dredge_area_m2, None);
    }
    assert!(lo > 0 && hi > 0);
    // Every section has the same cut (a prism): the weighted volume is (lo + 2 hi) / n of the plain.
    let n = (lo + hi) as f64;
    let want = plain.totals.dredge_m3 * (lo as f64 + 2.0 * hi as f64) / n;
    assert!(close(w.totals.weighted_dredge_m3.unwrap(), want));
    let by_reach: f64 = w
        .by_reach
        .iter()
        .map(|r| r.weighted_dredge_m3.unwrap())
        .sum();
    assert!(close(by_reach, want), "the reaches add up to the total");
    assert_eq!(w.totals.dredge_m3, plain.totals.dredge_m3);
    assert_eq!(plain.totals.weighted_dredge_m3, None);
}

#[test]
fn the_constraints_input_has_a_schema_and_a_document_without_one_has_none() {
    let c = Constraints::default();
    assert!(c.is_empty());
    assert_eq!(serde_json::to_value(&c).unwrap(), json!({}));
}

// --- the multiplier read from a real raster file ------------------------------------------------

/// A raster file in the project CRS over x -100..(-100 + 10 width), y -300..300, 10 m cells, each
/// cell `f(x, y)` at its centre, with `nodata` as its no-data value.
fn grid_raster(width: usize, f: &dyn Fn(f64, f64) -> f64, nodata: Option<f64>) -> Vec<u8> {
    let height = 60;
    let mut data = Vec::with_capacity(width * height);
    for r in 0..height {
        for c in 0..width {
            data.push(f(
                -100.0 + 10.0 * (c as f64 + 0.5),
                300.0 - 10.0 * (r as f64 + 0.5),
            ));
        }
    }
    let d = Dem {
        width,
        height,
        data,
        geotransform: [10.0, 0.0, -100.0, 0.0, -10.0, 300.0],
        nodata,
        epsg: Some(32630),
    };
    write_geotiff(&d).unwrap()
}

/// The weighted dredge area over the plain one at each station, from the multiplier in `bytes`.
fn station_factors(bytes: &[u8]) -> Vec<(f64, f64)> {
    let input = volumes_input();
    let plain = volumes_stage(&input, &flat).unwrap();
    let raster = constraints::read_multiplier(bytes, Some(32630)).unwrap();
    let m = constraints::multiplier_sampler(&raster, Some(32630)).unwrap();
    let w = volumes_stage_with(&input, &flat, Some(&m)).unwrap();
    plain
        .stations
        .iter()
        .zip(&w.stations)
        .map(|(p, s)| {
            assert_eq!(s.dredge_area_m2, p.dredge_area_m2, "plain does not move");
            (
                s.chainage_m,
                s.weighted_dredge_area_m2.unwrap() / p.dredge_area_m2,
            )
        })
        .collect()
}

#[test]
fn a_no_data_cell_of_a_real_raster_file_multiplies_by_one() {
    // ×3 from x 1000 m on; before it, the file's own no-data value.
    let bytes = grid_raster(
        230,
        &|x, _| if x >= 1000.0 { 3.0 } else { -9999.0 },
        Some(-9999.0),
    );
    let f = station_factors(&bytes);
    assert!(f.iter().any(|&(c, _)| c < 1000.0) && f.iter().any(|&(c, _)| c >= 1000.0));
    for (c, k) in f {
        let want = if c >= 1000.0 { 3.0 } else { 1.0 };
        assert!(close(k, want), "at {c} m: ×{k}, not ×{want}");
    }
}

#[test]
fn a_point_off_the_raster_multiplies_by_one() {
    // ×2 on a raster that ends at x 1000 m: beyond it, the line is off the raster.
    let bytes = grid_raster(110, &|_, _| 2.0, None);
    let f = station_factors(&bytes);
    for (c, k) in f {
        let want = if c < 1000.0 { 2.0 } else { 1.0 };
        assert!(close(k, want), "at {c} m: ×{k}, not ×{want}");
    }
}

#[test]
fn a_patch_that_varies_across_the_section_is_read_sample_by_sample() {
    let input = volumes_input();
    let plain = volumes_stage(&input, &flat).unwrap();
    // ×3 left of the line, ×1 right of it, ×2 on it: the section is symmetric, so the weighted cut
    // is twice the plain cut, sample by sample.
    let sides = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(_, &y)| 2.0 + y.signum() * f64::from(u8::from(y != 0.0)))
            .collect()
    };
    let w = volumes_stage_with(&input, &flat, Some(&sides)).unwrap();
    // ×2 on the left only (no data, 1, on the right): more than the plain cut, less than twice it.
    let left = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(_, &y)| if y > 0.0 { 2.0 } else { f64::NAN })
            .collect()
    };
    let l = volumes_stage_with(&input, &flat, Some(&left)).unwrap();
    for ((p, s), h) in plain.stations.iter().zip(&w.stations).zip(&l.stations) {
        let a = p.dredge_area_m2;
        assert!(a > 0.0);
        assert!(
            (s.weighted_dredge_area_m2.unwrap() - 2.0 * a).abs() <= 1e-9 * a,
            "at {}: {} vs {}",
            p.chainage_m,
            s.weighted_dredge_area_m2.unwrap(),
            2.0 * a
        );
        let k = h.weighted_dredge_area_m2.unwrap() / a;
        assert!(k > 1.2 && k < 1.8, "at {}: ×{k}", p.chainage_m);
    }
}

#[test]
fn moving_the_multiplier_raster_file_leaves_the_stages_fresh() {
    let (_, mut s) = example();
    let ones = raster(&|_, _| 1.0, 32630);
    let set = |s: &mut Session, path: &str| {
        s.set_constraints(ConstraintsEdit {
            no_go: None,
            multiplier: Some(Some(MultiplierFile { path, bytes: &ones })),
        })
        .unwrap();
    };
    set(&mut s, "ones.tif");
    let d = dem();
    let t = dem_terrain(&d);
    s.run_stage_with(Stage::Alignment, None).unwrap();
    s.run_stage_with(Stage::Volumes, Some(&t as TerrainSampler))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    // The same file, moved to another folder: the record's path changes, its hash does not.
    set(&mut s, "rasters/ones.tif");
    assert_eq!(s.multiplier_path(), Some("rasters/ones.tif"));
    assert_eq!(s.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    // Opened again and attached from its new place, the stages are still fresh.
    let mut back = Session::open(&s.to_text()).unwrap();
    back.attach_multiplier(&ones).unwrap();
    assert_eq!(back.stage_status(Stage::Alignment), StageStatus::Fresh);
    assert_eq!(back.stage_status(Stage::Volumes), StageStatus::Fresh);
}

#[test]
fn a_document_with_no_constraints_designs_its_line_again_byte_for_byte() {
    // The alignment run again on a document with no constraints stores the line it stored: no
    // `no_go_clear` check is written, and nothing else moves.
    let (text, mut s) = example();
    s.run_stage_with(Stage::Alignment, None).unwrap();
    let before: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        s.document()["stages"]["alignment"],
        before["stages"]["alignment"]
    );
    assert!(
        s.document()["stages"]["alignment"]["results"]["checks"]
            .get("no_go_clear")
            .is_none()
    );
}
