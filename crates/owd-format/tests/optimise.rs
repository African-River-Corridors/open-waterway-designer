//! The optimise stage on the `optimise_pools` example (a straight 3 km design line across the
//! synthetic river, given in UTM 30N and shifted into the zone): the weirs, pools and lattice option
//! equal the cascade search and `optimise_pools` called directly, and equal what the frozen Python
//! printed for the example. The lattice option is kept with its pools and checks, and the volumes
//! stage measures it like any other option, under the pool levels.
//!
//! Label: snapshot (the Python reference for the example); the direct-call checks are properties.

use owd_core::alignment::radius_check::{min_radius_on_chord, radius_ok};
use owd_core::geom::{Polygon, Pt};
use owd_core::optimise::cascade::{
    CascadeConfig, SectionOptions, build_level_table, cascade_levels, earthworks_reach_cost,
    search_cascade,
};
use owd_core::optimise::lattice::{
    HalfWidthSpec, LatticeConfig, PoolOptions, SectionSpec, optimise_pools,
};
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_core::volumes::sections::HalfWidth;
use owd_format::design::{AlignMethod, Role, Session, Stage, StageStatus};
use owd_format::geotiff::Dem;
use serde_json::{Value, json};

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];

fn shift(p: &Pt) -> Pt {
    [p[0] + SHIFT[0], p[1] + SHIFT[1]]
}

fn params() -> RiverParams {
    RiverParams {
        length_m: 3000.0,
        wavelength_m: 2400.0,
        amplitude_m: 120.0,
        hill_at: [1500.0, -60.0],
        hill_height_m: 10.0,
        hill_radius_m: 80.0,
        ..RiverParams::default()
    }
}

fn terrain(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let r = synthetic_river(params()).unwrap();
    xs.iter()
        .zip(ys)
        .map(|(&x, &y)| r.terrain(x - SHIFT[0], y - SHIFT[1]))
        .collect()
}

fn dem() -> Dem {
    Dem {
        width: 2,
        height: 2,
        data: vec![0.0; 4],
        geotransform: [1.0, 0.0, 0.0, 0.0, -1.0, 2.0],
        nodata: None,
        epsg: Some(32630),
    }
}

/// The example's settings, as the example sets them.
fn example_settings() -> Value {
    json!({
        "candidate_sites_m": [500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        "lifts_m": [0.5, 1.0],
        "terminal_min_level_m": 0.5,
        "min_spacing_m": 1000.0,
        "dry_weight": 1.5,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "cut_model": "b",
        "maxw_m": 120.0,
        "rake_half_m": 80.0,
        "rake_step_m": 5.0,
        "ds_step_m": 10.0,
        "pitch_m": 20.0,
        "n_heading": 24,
        "radius_multiples": [1.0, 2.0, 4.0],
        "corridor_half_m": 180.0,
        "cell_m": 5.0,
    })
}

fn design() -> Session {
    design_with(true)
}

fn design_with(water: bool) -> Session {
    let r = synthetic_river(params()).unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let line = [shift(&[0.0, 0.0]), shift(&[3000.0, 0.0])];
    let mut s = Session::new("Pools", "2026-10-16");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(line)
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
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    s.set_dem("dem.tif", b"dem", &dem());
    if water {
        s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    }
    s
}

fn pts(v: &Value) -> Vec<Pt> {
    v["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

#[test]
fn the_sizing_gives_the_examples_depth_and_r_min_and_a_half_width_near_its_15_m() {
    let s = design();
    let r = &s.document()["stages"]["sizing"]["results"];
    assert_eq!(r["design_depth_m"], 3.25);
    // The example hard-codes 15 m; the sizing gives 15.2 m. The stage uses the sizing's.
    assert!((r["bottom_width_straight_m"].as_f64().unwrap() / 2.0 - 15.2).abs() < 1e-9);
    assert_eq!(r["min_bend_radius_m"], 255.0);
}

#[test]
fn weirs_pools_and_the_lattice_option_equal_the_core_called_directly() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    let res = s.document()["stages"]["optimise"]["results"].clone();
    let hb = s.document()["stages"]["sizing"]["results"]["bottom_width_straight_m"]
        .as_f64()
        .unwrap()
        / 2.0;
    let baseline = pts(&res["baseline"]);
    assert!((baseline[0][0] - SHIFT[0]).abs() < 1e-9 && baseline.len() >= 2);

    // The core, called directly, as the example calls it.
    let r = synthetic_river(params()).unwrap();
    let poly = [Polygon {
        exterior: r.polygon.exterior.iter().map(shift).collect(),
        holes: vec![],
    }];
    let mut cfg = CascadeConfig::new(
        vec![500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        vec![0.5, 1.0],
        owd_core::geom::linear::length(&baseline),
        0.5,
    );
    cfg.min_spacing_m = 1000.0;
    let cfg = cfg.validated().unwrap();
    let table = build_level_table(
        &baseline,
        &terrain,
        &poly,
        &cascade_levels(&cfg),
        3.25,
        &HalfWidth::Fixed(hb),
        Some("b"),
        Some(25.0),
        Some(120.0),
        None,
        SectionOptions::default(),
    )
    .unwrap();
    let reach = earthworks_reach_cost(&table, 1.0, 1.5);
    let structure = |lift: f64| 1.0e5 + 2.0e5 * lift;
    let search = search_cascade(&cfg, &reach, &structure).unwrap();
    let best = search.best().unwrap();
    let pools = best.pools();
    assert_eq!(
        res["pools"],
        json!(
            pools
                .iter()
                .map(|&(a, b, l)| json!([a, b, l]))
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(
        res["cascade"]["total_cost"].as_f64().unwrap(),
        best.total_cost
    );

    let mut spec = SectionSpec::new(HalfWidthSpec::Fixed(hb));
    spec.side_slope_h_per_v = 3.0;
    spec.dry_weight = 1.5;
    spec.rake_half_m = 80.0;
    spec.rake_step_m = 5.0;
    spec.ds_step_m = 10.0;
    let mut lat = LatticeConfig::new(255.0);
    lat.pitch_m = 20.0;
    lat.n_heading = 24;
    lat.radius_multiples = vec![1.0, 2.0, 4.0];
    let lat = lat.validated().unwrap();
    let direct = optimise_pools(
        &baseline,
        &terrain,
        &poly,
        &pools,
        3.25,
        &spec,
        &lat,
        None,
        PoolOptions {
            corridor_half_m: 180.0,
            cell_m: 5.0,
        },
        true,
    )
    .unwrap();
    let opt = owd_format::design::design_lines_of(s.document())
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(pts(&opt["line"]), direct.line);
    assert_eq!(opt["details"]["n_replaced"], direct.n_replaced());
    let measured = min_radius_on_chord(&direct.line, 255.0 / 3.0).unwrap();
    assert_eq!(
        opt["checks"]["measured_min_radius_m"].as_f64().unwrap(),
        measured
    );
    assert_eq!(opt["checks"]["radius_check_chord_m"], 255.0 / 3.0);
    // Spec 0005 Q11 (#95): the pools result and every converged pool carry the chord reading.
    assert_eq!(direct.measured_min_radius_m, measured);
    assert_eq!(direct.radius_check_chord_m, 255.0 / 3.0);
    assert_eq!(direct.radius_ok, radius_ok(measured, 255.0));
    for p in &direct.pools {
        assert_eq!(p.result.radius_check_chord_m, 255.0 / 3.0);
        if let Some(l) = &p.result.line {
            let m = min_radius_on_chord(l, 255.0 / 3.0).unwrap();
            assert_eq!(p.result.measured_min_radius_m, m);
            assert_eq!(p.result.radius_ok, radius_ok(m, 255.0));
        } else {
            assert!(p.result.measured_min_radius_m.is_nan() && !p.result.radius_ok);
        }
    }
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
}

#[test]
fn volumes_measure_the_lattice_line_under_the_pool_levels() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::NotRun);
    s.run_volumes(&json!({"cut_model": "b", "maxw_m": 120.0}), Some(&terrain))
        .unwrap();
    let rows = owd_format::design::volume_rows_of(s.document());
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    // One design line per run (spec 0017): the lattice's refined line is the one measured.
    assert_eq!(ids, ["lattice"]);
    assert!(rows.iter().all(|r| r["water_source"] == "optimise"));
    // The profile is the pool levels: a step of the weir's lift where the weir stands.
    let (profile, source) = s.water_profile().unwrap();
    assert_eq!(source, "optimise");
    let weir = &s.document()["stages"]["optimise"]["results"]["cascade"]["weirs"][0];
    let lift = weir["lift_m"].as_f64().unwrap();
    let profile = profile.to_pairs();
    let steps: Vec<&[[f64; 2]]> = profile
        .windows(2)
        .filter(|w| w[0][0] == w[1][0] && w[0][1] != w[1][1])
        .collect();
    assert!(!steps.is_empty(), "{profile:?}");
    assert!((steps[0][1][1] - steps[0][0][1] - lift).abs() < 1e-9);
    assert!(rows.iter().all(|r| r["dredge_m3"].as_f64().unwrap() > 0.0));
    // A new optimise run leaves the volumes stale.
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Fresh);
    s.run_optimise(&json!({"dry_weight": 3.0}), Some(&terrain))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Volumes), StageStatus::Stale);
}

#[test]
fn the_lattice_option_carries_a_sanity_entry_like_every_other_option() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    let o = owd_format::design::design_lines_of(s.document())
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(o["sanity"]["checked"], json!(true), "{}", o["sanity"]);
    assert!(o["sanity"]["warnings"].is_array());
}

#[test]
fn the_stage_says_what_it_needs() {
    let mut s = design_with(false);
    let e = s
        .run_optimise(&Value::Null, Some(&terrain))
        .unwrap_err()
        .to_string();
    assert!(e.contains("needs a water level"), "{e}");
    let mut s = design();
    let e = s
        .run_optimise(&example_settings(), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("dem.tif") && e.contains("missing"), "{e}");
    let mut bare = Session::new("x", "2026-10-16");
    bare.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    bare.set_vessel("CEMT_IV", None).unwrap();
    bare.run_stage(Stage::Sizing).unwrap();
    // With no DEM the stage runs on plan area, so a bare design is refused for what that needs:
    // a river to follow.
    let e = bare
        .run_optimise(&example_settings(), Some(&terrain))
        .unwrap_err()
        .to_string();
    println!("{e}");
    assert!(!e.contains("needs a DEM"), "{e}");
}

/// The length weight (a cost per metre of design line) is a setting of the stage, off (0) by
/// default. Off, it is not written into the document; on, it is, through the same reading as `owd
/// optimise --length-weight`. A negative value is refused by name and changes nothing.
///
/// Label: property
#[test]
fn the_length_weight_is_a_setting_off_by_default() {
    use owd_format::design::OptimiseSettings;
    let d = OptimiseSettings::default();
    assert_eq!(d.length_weight, 0.0);
    assert!(d.to_value().get("length_weight").is_none());
    let mut s = design();
    s.set_optimise_settings(&json!({"length_weight": 25.0}))
        .unwrap();
    let st = &s.document()["stages"]["optimise"]["settings"];
    assert_eq!(st["length_weight"], json!(25.0));
    // The rest are the defaults.
    let mut want = d.to_value();
    want["length_weight"] = json!(25.0);
    assert_eq!(st, &want);
    for bad in [json!(-1.0), json!("ten")] {
        let before = s.to_text();
        let e = s
            .set_optimise_settings(&json!({ "length_weight": bad }))
            .unwrap_err()
            .to_string();
        assert!(e.contains("length_weight"), "{e}");
        assert_eq!(s.to_text(), before);
    }
    // Back to 0: the key goes, and the settings are the defaults again.
    s.set_optimise_settings(&json!({"length_weight": 0.0}))
        .unwrap();
    assert_eq!(s.document()["stages"]["optimise"]["settings"], d.to_value());
}

/// At weight 0 the stage writes the same document, byte for byte, as a run that never names the
/// weight. A positive weight makes a fresh stage stale, is kept in the settings and shown in the
/// lattice's details with each pool's line length and length cost; the cascade does not read it
/// (every cascade covers the whole line, so a cost per metre would add the same to each), so its
/// weirs and costs are unchanged.
///
/// Label: property
#[test]
fn a_zero_length_weight_changes_nothing_and_a_positive_one_is_hashed_and_shown() {
    let mut plain = design();
    plain
        .run_optimise(&example_settings(), Some(&terrain))
        .unwrap();
    let mut zero = design();
    let mut with_zero = example_settings();
    with_zero["length_weight"] = json!(0.0);
    zero.run_optimise(&with_zero, Some(&terrain)).unwrap();
    assert_eq!(zero.to_text(), plain.to_text());

    // On: the fresh stage goes stale, then runs with it.
    assert_eq!(zero.stage_status(Stage::Optimise), StageStatus::Fresh);
    zero.set_optimise_settings(&json!({"length_weight": 50.0}))
        .unwrap();
    assert_eq!(zero.stage_status(Stage::Optimise), StageStatus::Stale);
    zero.run_optimise(&Value::Null, Some(&terrain)).unwrap();
    assert_eq!(zero.stage_status(Stage::Optimise), StageStatus::Fresh);
    let doc = zero.document();
    assert_eq!(
        doc["stages"]["optimise"]["settings"]["length_weight"],
        json!(50.0)
    );
    let base = plain.document();
    assert_eq!(
        doc["stages"]["optimise"]["results"]["cascade"],
        base["stages"]["optimise"]["results"]["cascade"]
    );
    let opt = owd_format::design::design_lines_of(doc)
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(opt["details"]["length_weight"], json!(50.0));
    for p in opt["details"]["pools"].as_array().unwrap() {
        if p["converged"] == json!(true) {
            let len = p["line_length_m"].as_f64().unwrap();
            assert!(len > 0.0);
            assert_eq!(p["length_cost"].as_f64().unwrap(), 50.0 * len);
            assert!(p["cost"].as_f64().unwrap() >= 50.0 * len);
        }
    }
    let off = owd_format::design::design_lines_of(base)
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert!(off["details"].get("length_weight").is_none());
    assert!(
        off["details"]["pools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("line_length_m").is_none())
    );
}

/// FNV-1a, 64 bit, of a text: a fingerprint with no dependency.
fn fingerprint(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// With every new setting at its default (`land_weight` 0, `corridor_limit` false) the optimise
/// stage gives the bytes it gave before the settings existed. The fingerprints are of the stage's
/// settings and results on the `optimise_pools` example, as text, taken on the branch
/// `local/2026-11-20` at cabe3ee, before the land cost was written. Naming the defaults changes
/// nothing either. A change of the optimise results on purpose must move these numbers.
///
/// Label: snapshot (the base branch's optimise results on the example).
#[test]
fn with_the_new_settings_at_their_defaults_the_results_are_the_base_branchs_bytes() {
    let mut plain = design();
    plain
        .run_optimise(&example_settings(), Some(&terrain))
        .unwrap();
    let st = &plain.document()["stages"]["optimise"];
    let (settings, results) = (st["settings"].to_string(), st["results"].to_string());
    println!(
        "settings {} bytes {:#018x}, results {} bytes {:#018x}",
        settings.len(),
        fingerprint(&settings),
        results.len(),
        fingerprint(&results)
    );
    assert_eq!((settings.len(), fingerprint(&settings)), BASE_SETTINGS);
    assert_eq!((results.len(), fingerprint(&results)), BASE_RESULTS);
    // The defaults, named: the same bytes.
    let mut named = design();
    let mut with_defaults = example_settings();
    with_defaults["land_weight"] = json!(0.0);
    with_defaults["corridor_limit"] = json!(false);
    named.run_optimise(&with_defaults, Some(&terrain)).unwrap();
    assert_eq!(named.to_text(), plain.to_text());
}

/// The land weight (a cost per m2 of channel footprint outside the river polygon) is a setting of
/// the stage, off (0) by default. Off, it is not written into the document; on, it is, through the
/// same reading as `owd optimise --land-weight`. A negative value, or text, is refused by name in
/// plain words and changes nothing.
///
/// Label: property
#[test]
fn the_land_weight_is_a_setting_off_by_default() {
    use owd_format::design::OptimiseSettings;
    let d = OptimiseSettings::default();
    assert_eq!(d.land_weight, 0.0);
    assert!(d.to_value().get("land_weight").is_none());
    let mut s = design();
    s.set_optimise_settings(&json!({"land_weight": 3.25}))
        .unwrap();
    let st = &s.document()["stages"]["optimise"]["settings"];
    let mut want = d.to_value();
    want["land_weight"] = json!(3.25);
    assert_eq!(st, &want);
    // It reads back from the saved text.
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(
        again.document()["stages"]["optimise"]["settings"]["land_weight"],
        json!(3.25)
    );
    let before = s.to_text();
    let e = s
        .set_optimise_settings(&json!({"land_weight": -1.0}))
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "the setting `land_weight` must be a number >= 0 (a cost per m2 of channel footprint \
         outside the river polygon, in m3 of cut per m2; 0 is off)"
    );
    assert_eq!(s.to_text(), before);
    let e = s
        .set_optimise_settings(&json!({"land_weight": "a lot"}))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("land_weight") && e.contains("must be a number"),
        "{e}"
    );
    assert_eq!(s.to_text(), before);
    s.set_optimise_settings(&json!({"land_weight": 0.0}))
        .unwrap();
    assert_eq!(s.document()["stages"]["optimise"]["settings"], d.to_value());
}

/// A positive land weight makes a fresh stage stale, is kept in the settings and shown in the
/// lattice's details: the weight, the width of the footprint it is charged on, and each pool's land
/// area and land cost. The cascade does not read it, so the weirs and their costs are unchanged.
/// The volumes stage reports true volumes: it does not read the search cost, so on a line it
/// measures the same m3 whatever land weight is set.
///
/// Label: property
#[test]
fn a_positive_land_weight_is_hashed_and_shown_and_never_reaches_the_volumes() {
    let mut plain = design();
    plain
        .run_optimise(&example_settings(), Some(&terrain))
        .unwrap();
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    s.set_optimise_settings(&json!({"land_weight": 3.25}))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Stale);
    s.run_optimise(&Value::Null, Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    let (doc, base) = (s.document(), plain.document());
    assert_eq!(
        doc["stages"]["optimise"]["results"]["cascade"],
        base["stages"]["optimise"]["results"]["cascade"]
    );
    let lattice = |d: &Value| {
        owd_format::design::design_lines_of(d)
            .into_iter()
            .find(|o| o["method"] == "lattice")
            .unwrap()
            .clone()
    };
    let opt = lattice(doc);
    assert_eq!(opt["details"]["land_weight"], json!(3.25));
    // Bottom width 30.4 m, side slope 3 through a depth of 3.25 m each side.
    let width = opt["details"]["land_footprint_width_m"].as_f64().unwrap();
    assert!((width - (30.4 + 2.0 * 3.0 * 3.25)).abs() < 1e-9, "{width}");
    let mut seen = 0;
    for p in opt["details"]["pools"].as_array().unwrap() {
        if p["converged"] == json!(true) {
            let area = p["land_area_m2"].as_f64().unwrap();
            assert!(area >= 0.0);
            assert_eq!(p["land_cost"].as_f64().unwrap(), 3.25 * area);
            assert!(p["cost"].as_f64().unwrap() >= 3.25 * area - 1e-6);
            assert!(p["baseline_land_area_m2"].as_f64().unwrap() >= 0.0);
            seen += 1;
        }
    }
    assert!(seen > 0);
    let off = lattice(base);
    assert!(off["details"].get("land_weight").is_none());
    assert!(off["details"].get("land_footprint_width_m").is_none());
    assert!(
        off["details"]["pools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("land_area_m2").is_none() && p.get("land_cost").is_none())
    );

    // The volumes stage measures the line it is given, with its own section method: its record
    // holds m3 only, the same fields as on a run with no land weight, and no land cost.
    let rows = |s: &mut Session| -> Vec<Value> {
        s.run_volumes(&json!({"cut_model": "b", "maxw_m": 120.0}), Some(&terrain))
            .unwrap();
        owd_format::design::volume_rows_of(s.document())
    };
    let (with_land, without) = (rows(&mut s), rows(&mut plain));
    let keys = |r: &Value| -> Vec<String> { r.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&with_land[0]), keys(&without[0]));
    assert!(keys(&with_land[0]).iter().all(|k| !k.contains("land")));
    assert!(with_land[0]["dredge_m3"].as_f64().unwrap() > 0.0);
}

const BASE_SETTINGS: (usize, u64) = (551, 0xd587_925a_739f_6b8a);
const BASE_RESULTS: (usize, u64) = (48188, 0x5036_5f19_a431_3451);

/// The corridor limit is a setting of the stage, off (false) by default and then not written. On,
/// it is written, read back from the saved text, and makes a fresh stage stale. A value that is not
/// true or false is refused by name in plain words and changes nothing.
///
/// Label: property
#[test]
fn the_corridor_limit_is_a_setting_off_by_default() {
    use owd_format::design::OptimiseSettings;
    let d = OptimiseSettings::default();
    assert!(!d.corridor_limit);
    assert!(d.to_value().get("corridor_limit").is_none());
    let mut s = design();
    s.set_optimise_settings(&json!({"corridor_limit": true}))
        .unwrap();
    let mut want = d.to_value();
    want["corridor_limit"] = json!(true);
    assert_eq!(s.document()["stages"]["optimise"]["settings"], want);
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(
        again.document()["stages"]["optimise"]["settings"]["corridor_limit"],
        json!(true)
    );
    let before = s.to_text();
    for bad in [json!(1.0), json!("yes")] {
        let e = s
            .set_optimise_settings(&json!({ "corridor_limit": bad }))
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "the setting `corridor_limit` must be true or false (true holds the pool search \
             inside the corridor)"
        );
        assert_eq!(s.to_text(), before);
    }
    s.set_optimise_settings(&json!({"corridor_limit": false}))
        .unwrap();
    assert_eq!(s.document()["stages"]["optimise"]["settings"], d.to_value());
}

/// With the limit on, the lattice's line stays within `corridor_half_m` of the baseline (to within
/// a grid cell), the details say the limit was on and how wide, and every pool is either used or
/// keeps its baseline with a reason. The example's corridor is 180 m.
///
/// Label: property
#[test]
fn with_the_corridor_limit_the_stages_line_stays_inside_the_corridor() {
    let mut s = design();
    s.run_optimise(&example_settings(), Some(&terrain)).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    s.set_optimise_settings(&json!({"corridor_limit": true}))
        .unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Stale);
    s.run_optimise(&Value::Null, Some(&terrain)).unwrap();
    let doc = s.document();
    let res = &doc["stages"]["optimise"]["results"];
    let opt = owd_format::design::design_lines_of(doc)
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(opt["details"]["corridor_limit"], json!(true));
    assert_eq!(opt["details"]["corridor_half_m"], json!(180.0));
    let (line, baseline) = (pts(&opt["line"]), pts(&res["baseline"]));
    let far = line
        .iter()
        .map(|p| {
            let q = owd_core::geom::linear::interpolate(
                &baseline,
                owd_core::geom::linear::project(&baseline, *p),
            )
            .unwrap();
            libm::hypot(p[0] - q[0], p[1] - q[1])
        })
        .fold(0.0, f64::max);
    assert!(far <= 180.0 + 5.0, "{far}");
    for p in opt["details"]["pools"].as_array().unwrap() {
        assert!(
            p["used"] == json!(true) || p["kept_baseline_because"].is_string(),
            "{p}"
        );
    }
}

/// A design like `design()` on a river with tight bends (wavelength 900 m, amplitude 120 m: bends of
/// about 170 m against R_min 255 m), with NO DEM.
fn design_without_a_dem() -> (Session, Vec<Polygon>) {
    let r = synthetic_river(RiverParams {
        length_m: 3000.0,
        wavelength_m: 900.0,
        amplitude_m: 120.0,
        ..RiverParams::default()
    })
    .unwrap();
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let line = [shift(&[0.0, 0.0]), shift(&[3000.0, 0.0])];
    let mut s = Session::new("Plan area", "2026-11-20");
    s.import(&[
        (
            Role::Centreline,
            &format!(
                r#"{{"type":"LineString"{UTM30},"coordinates":{}}}"#,
                json!(line)
            ),
        ),
        (
            Role::RiverPolygon,
            &format!(
                r#"{{"type":"Polygon"{UTM30},"coordinates":{}}}"#,
                json!([ring.clone()])
            ),
        ),
    ])
    .unwrap();
    s.set_rule_sets(&["pianc-wg141"], &[("se_quality", "B")])
        .unwrap();
    s.set_vessel("CEMT_IV", Some(2.5)).unwrap();
    s.run_stage(Stage::Sizing).unwrap();
    s.align(AlignMethod::RollingCircle, &Value::Null, None)
        .unwrap();
    (
        s,
        vec![Polygon {
            exterior: ring,
            holes: vec![],
        }],
    )
}

/// With no DEM the stage runs on plan area ("optimize for volume, or m2 if we don't have a DEM"):
/// the cost is the m2 of channel footprint outside the river polygon, at a land weight of 1 unless
/// one is set, plus the length cost. No water level is needed, no weirs or pool levels are chosen,
/// and the result says plainly, in its results, its details and its warnings, that it was optimised
/// on plan area and not on volume. The line takes less land than the baseline it started from. The
/// volumes stage still refuses to run without a DEM.
///
/// Label: property
#[test]
fn with_no_dem_the_stage_optimises_on_plan_area_and_says_so() {
    let (mut s, river) = design_without_a_dem();
    let settings = json!({
        "rake_half_m": 80.0, "rake_step_m": 5.0, "ds_step_m": 10.0, "pitch_m": 20.0,
        "n_heading": 24, "radius_multiples": [1.0, 2.0, 4.0], "corridor_half_m": 180.0,
        "cell_m": 5.0, "length_weight": 1.0,
    });
    s.run_optimise(&settings, None).unwrap();
    assert_eq!(s.stage_status(Stage::Optimise), StageStatus::Fresh);
    let doc = s.document().clone();
    let res = &doc["stages"]["optimise"]["results"];
    assert_eq!(res["optimised_on"], json!("plan_area"));
    assert_eq!(res["pools"], json!([]));
    assert_eq!(res["profile"], json!([]));
    assert_eq!(res["cascade"]["weirs"], json!([]));
    assert!(res["base_level_m"].is_null());
    // The land weight was not set: 1 is used and said, and it is not written into the settings.
    assert!(
        doc["stages"]["optimise"]["settings"]
            .get("land_weight")
            .is_none()
    );
    let opt = owd_format::design::design_lines_of(&doc)
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    let d = &opt["details"];
    assert_eq!(d["optimised_on"], json!("plan_area"));
    assert_eq!(d["land_weight"], json!(1.0));
    assert_eq!(d["land_weight_defaulted"], json!(true));
    let note = d["optimised_on_note"].as_str().unwrap();
    assert!(
        note.contains("optimised on plan area") && note.contains("not on volume"),
        "{note}"
    );
    assert!(
        opt["sanity"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str() == Some(note)),
        "{}",
        opt["sanity"]
    );
    // Every piece is one stretch of the whole line, with no water level; the line takes less land
    // than the baseline, and its cost is its land (weight 1) and its length (weight 1), no cut.
    let (mut land, mut base_land) = (0.0, 0.0);
    for p in d["pools"].as_array().unwrap() {
        assert!(p["water_level_m"].is_null(), "{p}");
        assert_eq!(p["converged"], json!(true), "{p}");
        let (a, l) = (
            p["land_area_m2"].as_f64().unwrap(),
            p["line_length_m"].as_f64().unwrap(),
        );
        let cost = p["cost"].as_f64().unwrap();
        assert!((cost - a - l).abs() < 1e-3 * cost, "{p}");
        land += a;
        base_land += p["baseline_land_area_m2"].as_f64().unwrap();
    }
    println!("land: line {land:.0} m2, baseline {base_land:.0} m2");
    assert!(land < 0.8 * base_land, "{land} against {base_land}");
    // Read apart from the search's own measure: less of the line's centre is outside the polygon
    // than of the baseline's.
    let outside = |line: &[Pt]| {
        let n = (owd_core::geom::linear::length(line) / 5.0) as usize;
        (0..=n)
            .filter(|&i| {
                let p = owd_core::geom::linear::interpolate(line, i as f64 * 5.0).unwrap();
                !owd_core::geom::overlay::contains_xy(&river, p)
            })
            .count() as f64
            / (n + 1) as f64
    };
    let (line_out, base_out) = (outside(&pts(&opt["line"])), outside(&pts(&res["baseline"])));
    println!("outside the polygon: line {line_out:.3}, baseline {base_out:.3}");
    assert!(line_out < base_out, "{line_out} against {base_out}");
    assert_eq!(opt["checks"]["radius_ok"], json!(true));

    // The run needed no water level. With one given, the water the later stages read is still the
    // inputs': the stage set no pool level.
    assert!(s.water_profile().is_none());
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s.run_optimise(&settings, None).unwrap();
    assert_eq!(s.water_profile().unwrap().1, "input");
    // The volumes stage refuses without a DEM, as before.
    let e = s.run_volumes(&Value::Null, None).unwrap_err().to_string();
    assert!(e.contains("the volumes stage needs a DEM"), "{e}");
    assert!(s.document()["stages"]["volumes"]["results"].is_null());

    // A land weight that is set is the one used.
    let mut with = settings.clone();
    with["land_weight"] = json!(2.0);
    s.run_optimise(&with, None).unwrap();
    let opt = owd_format::design::design_lines_of(s.document())
        .into_iter()
        .find(|o| o["method"] == "lattice")
        .unwrap()
        .clone();
    assert_eq!(opt["details"]["land_weight"], json!(2.0));
    assert_eq!(opt["details"]["land_weight_defaulted"], json!(false));
    assert_eq!(opt["details"]["optimised_on"], json!("plan_area"));
}

/// A design that names a DEM whose file is missing is still refused: plan area is for a design with
/// no DEM at all, never a quiet fallback when the DEM cannot be read.
///
/// Label: property
#[test]
fn a_named_dem_that_is_missing_is_still_refused() {
    let mut s = design();
    let e = s
        .run_optimise(&example_settings(), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("dem.tif") && e.contains("missing"), "{e}");
    assert!(s.document()["stages"]["optimise"]["results"].is_null());
}

/// The flood settings (ticket OPT-2) are settings of the stage, each off or at its default until
/// set, and then not written into the document. Set, each is written through the same reading as
/// the flags of `owd optimise`. A bad value is refused by the setting's name and changes nothing.
///
/// Label: property
#[test]
fn the_flood_settings_are_off_by_default_round_trip_and_are_validated() {
    use owd_format::design::OptimiseSettings;
    let d = OptimiseSettings::default();
    assert_eq!(d.flood_cost_per_ha, 0.0);
    assert_eq!(d.max_above_bank_m, None);
    assert_eq!(d.flood_distance_m, 1500.0);
    assert_eq!(d.flood_cell_m, 25.0);
    assert!(!d.flood_report);
    assert!(!d.flood_on());
    let keys = [
        "flood_cost_per_ha",
        "max_above_bank_m",
        "flood_distance_m",
        "flood_cell_m",
        "flood_report",
    ];
    for k in keys {
        assert!(
            d.to_value().get(k).is_none(),
            "{k} is written at its default"
        );
    }
    let mut s = design();
    let set = json!({
        "flood_cost_per_ha": 3000.0,
        "max_above_bank_m": 1.5,
        "flood_distance_m": 800.0,
        "flood_cell_m": 10.0,
        "flood_report": true,
    });
    s.set_optimise_settings(&set).unwrap();
    let mut want = d.to_value();
    for k in keys {
        want[k] = set[k].clone();
    }
    assert_eq!(s.document()["stages"]["optimise"]["settings"], want);
    // Saved and opened again, the settings are the same.
    let again = Session::open(&s.to_text()).unwrap();
    assert_eq!(again.document()["stages"]["optimise"]["settings"], want);

    for (k, bad) in [
        ("flood_cost_per_ha", json!(-1.0)),
        ("flood_cost_per_ha", json!("3000")),
        ("max_above_bank_m", json!(-0.1)),
        ("max_above_bank_m", json!("high")),
        ("flood_distance_m", json!(0.0)),
        ("flood_cell_m", json!(-5.0)),
        ("flood_report", json!(1)),
    ] {
        let before = s.to_text();
        let e = s
            .set_optimise_settings(&json!({ k: bad }))
            .unwrap_err()
            .to_string();
        assert!(e.contains(k), "{e}");
        assert_eq!(s.to_text(), before);
    }
    // Back to the defaults: the keys go, and the settings are the defaults again.
    s.set_optimise_settings(&json!({
        "flood_cost_per_ha": 0.0,
        "max_above_bank_m": null,
        "flood_distance_m": 1500.0,
        "flood_cell_m": 25.0,
        "flood_report": false,
    }))
    .unwrap();
    assert_eq!(s.document()["stages"]["optimise"]["settings"], d.to_value());
}

/// With no DEM the optimise stage runs on plan area and chooses no weirs, so flooded land cannot be
/// worked out: a flood cost above 0, a limit above bank-full or the flood report is refused in plain
/// words, and the design is not changed. A
/// named DEM whose file is missing keeps the stage's own refusal.
///
/// Label: property
#[test]
fn a_flood_cost_or_a_bank_limit_without_a_dem_is_refused() {
    for (setting, word) in [
        (
            json!({"flood_cost_per_ha": 3000.0}),
            "a flood cost (`flood_cost_per_ha`)",
        ),
        (
            json!({"max_above_bank_m": 1.0}),
            "a limit above bank-full (`max_above_bank_m`)",
        ),
        (
            json!({"flood_report": true}),
            "the flood report (`flood_report`)",
        ),
    ] {
        let (mut s, _) = design_without_a_dem();
        let before = s.to_text();
        let e = s.run_optimise(&setting, None).unwrap_err().to_string();
        assert!(
            e.contains(word) && e.contains("cannot be worked out without a DEM"),
            "{e}"
        );
        assert!(e.contains("runs on plan area"), "{e}");
        assert_eq!(s.to_text(), before);
        // The DEM named but not there to read: the stage's own message, before the flood one.
        let mut named = design();
        let e = named.run_optimise(&setting, None).unwrap_err().to_string();
        assert!(e.contains("dem.tif") && e.contains("missing"), "{e}");
    }
}

/// The flood terms in the stage, on the `optimise_pools` example. At flood cost 0 the stage writes
/// the same document, byte for byte, as a run that never names it. With the report on, the cascade
/// is the same and says what each pool floods and how it stands against the banks. A flood cost is
/// part of the cascade's total, and a high one never floods more. A limit that cannot be held
/// fails in plain words and changes nothing.
///
/// Label: property
#[test]
fn a_zero_flood_cost_changes_nothing_and_the_flood_terms_are_reported() {
    let mut plain = design();
    plain
        .run_optimise(&example_settings(), Some(&terrain))
        .unwrap();
    let mut zero = design();
    let mut with_zero = example_settings();
    with_zero["flood_cost_per_ha"] = json!(0.0);
    with_zero["max_above_bank_m"] = Value::Null;
    with_zero["flood_distance_m"] = json!(1500.0);
    zero.run_optimise(&with_zero, Some(&terrain)).unwrap();
    assert_eq!(zero.to_text(), plain.to_text());
    let base = plain.document()["stages"]["optimise"]["results"].clone();
    assert!(base["cascade"].get("flood").is_none());

    // Lifts of 2 and 4 m to a top pool of 4 m: the example's banks stand about 3 m over the
    // water, so these pools flood land. The report alone leaves the search as it is (the pools are
    // those of the cut cost alone), and gives a row for each pool.
    let mut high = example_settings();
    high["lifts_m"] = json!([2.0, 4.0]);
    high["terminal_min_level_m"] = json!(4.0);
    high["flood_cell_m"] = json!(10.0);
    let mut shown = design();
    let mut settings = high.clone();
    settings["flood_report"] = json!(true);
    shown.run_optimise(&settings, Some(&terrain)).unwrap();
    let res = shown.document()["stages"]["optimise"]["results"].clone();
    let flood = res["cascade"]["flood"].clone();
    assert!(
        res["cascade"]["weirs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["lift_m"] == json!(4.0))
    );
    let rows = flood["pools"].as_array().unwrap();
    let pools = res["pools"].as_array().unwrap();
    assert_eq!(rows.len(), pools.len());
    let mut sum = 0.0;
    for (i, (row, pool)) in rows.iter().zip(pools).enumerate() {
        assert_eq!(row["start_m"], pool[0]);
        assert_eq!(row["end_m"], pool[1]);
        assert_eq!(row["level_m"], pool[2]);
        if i == 0 {
            assert!(row["lift_m"].is_null());
        } else {
            assert_eq!(row["lift_m"], res["cascade"]["weirs"][i - 1]["lift_m"]);
        }
        let ha = row["flooded_ha"].as_f64().unwrap();
        assert!(ha >= 0.0);
        sum += ha;
        assert_eq!(row["flood_cost"], json!(0.0));
        assert_eq!(row["reaches_limit"], json!(false));
        for k in [
            "above_bank_lower_m",
            "above_bank_upper_m",
            "max_above_bank_m",
            "depth_over_river_lower_m",
            "depth_over_river_upper_m",
        ] {
            assert!(row[k].is_f64(), "{k}: {row}");
        }
        // The most a pool stands above the banks is at least what it does at either end.
        let most = row["max_above_bank_m"].as_f64().unwrap();
        assert!(most >= row["above_bank_lower_m"].as_f64().unwrap() - 1e-9);
        assert!(most >= row["above_bank_upper_m"].as_f64().unwrap() - 1e-9);
    }
    assert!((flood["flooded_ha"].as_f64().unwrap() - sum).abs() < 1e-9);
    assert!(sum > 1.0, "the 4 m pools flood land: {sum} ha");
    assert_eq!(flood["flood_cost"], json!(0.0));
    assert_eq!(flood["cut_cost"], res["cascade"]["reach_cost"]);
    assert_eq!(flood["distance_m"], json!(1500.0));
    assert_eq!(flood["cell_m"], json!(10.0));
    assert!(flood["bank_full"].as_str().unwrap().contains("estimate"));
    println!("report: {flood}");

    // A flood cost: a fresh stage goes stale, the cost is in the settings and in the total, and
    // the pools flood no more than before.
    assert_eq!(shown.stage_status(Stage::Optimise), StageStatus::Fresh);
    shown
        .set_optimise_settings(&json!({"flood_cost_per_ha": 1.0e7, "flood_report": false}))
        .unwrap();
    assert_eq!(shown.stage_status(Stage::Optimise), StageStatus::Stale);
    shown.run_optimise(&Value::Null, Some(&terrain)).unwrap();
    let doc = shown.document();
    assert_eq!(
        doc["stages"]["optimise"]["settings"]["flood_cost_per_ha"],
        json!(1.0e7)
    );
    let c = &doc["stages"]["optimise"]["results"]["cascade"];
    let f = &c["flood"];
    println!("priced: {f}");
    let ha = f["flooded_ha"].as_f64().unwrap();
    assert!(ha < sum, "{ha} against {sum}");
    assert!(
        doc["stages"]["optimise"]["settings"]
            .get("flood_report")
            .is_none()
    );
    assert!((f["flood_cost"].as_f64().unwrap() - 1.0e7 * ha).abs() < 1e-3);
    let total = c["total_cost"].as_f64().unwrap();
    let parts = f["cut_cost"].as_f64().unwrap()
        + f["flood_cost"].as_f64().unwrap()
        + c["structure_cost"].as_f64().unwrap();
    assert!(
        (total - parts).abs() < 1e-6 * total.max(1.0),
        "{total} against {parts}"
    );

    // A limit no cascade can hold: the top pool must stand at 4 m, about 1 m over the banks, and
    // the limit allows 0.25 m. The stage fails in plain words and writes nothing.
    let before = shown.to_text();
    let e = shown
        .run_optimise(
            &json!({"flood_cost_per_ha": 0.0, "max_above_bank_m": 0.25}),
            Some(&terrain),
        )
        .unwrap_err()
        .to_string();
    println!("{e}");
    assert!(e.contains("No cascade reaches the top level of 4.00 m with every pool at most 0.25 m above bank-full"), "{e}");
    assert!(
        e.contains("blocks it") && e.contains("To unblock it"),
        "{e}"
    );
    assert_eq!(shown.to_text(), before);
    // A top level the lifts cannot reach at all is the search's own failure, not the limit's.
    let e = shown
        .run_optimise(
            &json!({"max_above_bank_m": 0.25, "terminal_min_level_m": 50.0}),
            Some(&terrain),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("no cascade reaches the terminal level"), "{e}");
}
