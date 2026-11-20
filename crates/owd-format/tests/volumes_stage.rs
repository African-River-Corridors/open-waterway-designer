//! The volumes stage (spec 0017, ticket 9): it stores its stations, its volumes by reach and its
//! totals, and an island in the river polygon is dry land. Tested alone through the pure function
//! `volumes_stage(&VolumesIn, terrain)`; the earlier outputs it reads are made by the sizing and
//! water stages (both pure). No session is built for these.
//!
//! The river is straight, 2 km long, 400 m wide, in a flat terrain. The section of a station whose
//! design invert is `c` metres under the terrain is closed form at the default settings (samples
//! 1 m apart over 200 m each side, a wet slope of 3): the cut is `c` on the bottom and `c - r / 3`
//! at `r` metres up a slope.
//!
//! The rest of the file puts the stage through the session: what the volumes stage stores is what
//! the long section and the cross sections read from the walk today.
//!
//! Label: golden (the prism), property (the rest)

use owd_core::rules::engine;
use owd_format::design::sizing::{ChannelSize, SizingIn, size};
use owd_format::design::volumes_stage::{PROFILE_EVERY_M, Volumes, VolumesIn, volumes_stage};
use owd_format::design::water_stage::{Pool, WaterIn};
use owd_format::design::{AlignMethod, Role, Session, Stage, VolumesSettings};
use owd_format::geotiff::Dem;
use owd_format::types::{Chainage, Polyline, RiverPolygon, WaterProfile};
use serde_json::{Value, json};

const TERRAIN_M: f64 = 7.5;
/// Low enough that the design invert is under the terrain: there is a cut to take.
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
        if set.hash.is_none() {
            set.hash = engine::built_in()
                .find(&set.id, &set.version)
                .map(|s| s.hash.clone());
        }
    }
    size(&input, engine::built_in()).unwrap()
}

fn line() -> Polyline {
    Polyline((0..=20).map(|i| [100.0 * i as f64, 0.0]).collect())
}

fn ring(x0: f64, x1: f64, y0: f64, y1: f64) -> Polyline {
    Polyline(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]])
}

fn river(holes: Vec<Polyline>) -> RiverPolygon {
    RiverPolygon {
        shell: ring(-100.0, 2100.0, -200.0, 200.0),
        holes,
    }
}

fn water(pools: Vec<Pool>) -> WaterIn {
    WaterIn {
        centreline: line(),
        design_line: line(),
        given: Some(WaterProfile::from_pairs(&[[0.0, WATER_M]])),
        pools,
        beyond_pools: Default::default(),
        step_m: 20.0,
    }
}

fn input(holes: Vec<Polyline>, pools: Vec<Pool>) -> VolumesIn {
    VolumesIn {
        design_line: line(),
        size: channel(),
        water: water(pools),
        river_polygon: Some(river(holes)),
        settings: VolumesSettings::default(),
        profile_at: vec![],
    }
}

/// The cut of one section whose design invert is `c` metres under the flat terrain.
fn section_m2(c: f64, hb: f64) -> f64 {
    (-200..=200)
        .map(|o| {
            let r = (f64::from(o).abs() - hb).max(0.0);
            (c - r / 3.0).max(0.0)
        })
        .sum()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn a_prism_in_a_flat_raster_has_the_volume_worked_by_hand() {
    let size = channel();
    let out = volumes_stage(&input(vec![], vec![]), &flat).unwrap();
    let c = TERRAIN_M - (WATER_M - size.design_depth_m);
    let area = section_m2(c, size.bottom_width_straight_m / 2.0);
    assert!(c > 0.5 && area > 10.0, "a real cut: c = {c}, area = {area}");
    assert!(!out.stations.is_empty());
    for st in &out.stations {
        assert!(
            close(st.dredge_area_m2, area),
            "{} m: {} against {area}",
            st.chainage_m,
            st.dredge_area_m2
        );
        assert_eq!(st.excav_area_m2, 0.0, "all under water");
        assert_eq!(st.water_level_m, WATER_M);
    }
    let n = out.stations.len() as f64;
    assert!(close(out.totals.dredge_m3, n * area * out.station_m));
    assert_eq!(out.totals.excav_m3, 0.0);
    assert_eq!(out.totals.stations, out.stations.len());
}

#[test]
fn an_island_is_dry_land_not_dredge() {
    // An island 400 m long across the line, from 800 m to 1200 m, 120 m wide.
    let open = volumes_stage(&input(vec![], vec![]), &flat).unwrap();
    let isl = volumes_stage(
        &input(vec![ring(800.0, 1200.0, -60.0, 60.0)], vec![]),
        &flat,
    )
    .unwrap();
    assert!(
        isl.totals.dredge_m3 < open.totals.dredge_m3,
        "{:?} {:?}",
        isl.totals,
        open.totals
    );
    assert!(isl.totals.excav_m3 > 0.0, "the island's cut is dry cut");
    assert!(close(
        isl.totals.dredge_m3 + isl.totals.excav_m3,
        open.totals.dredge_m3
    ));
    // A station on the island has dry samples at the line; one off it has none.
    let on = isl
        .stations
        .iter()
        .find(|s| (s.chainage_m - 1000.0).abs() < 1e-6)
        .unwrap();
    assert!(on.excav_area_m2 > 0.0);
    let off = isl
        .stations
        .iter()
        .find(|s| (s.chainage_m - 200.0).abs() < 1e-6)
        .unwrap();
    assert_eq!(off.excav_area_m2, 0.0);
}

#[test]
fn the_volumes_by_reach_add_up_to_the_totals() {
    let pools = vec![
        Pool {
            from: Chainage(0.0),
            to: Chainage(900.0),
            level_m: 8.0,
            min_level_m: None,
            max_level_m: None,
        },
        Pool {
            from: Chainage(900.0),
            to: Chainage(2000.0),
            level_m: 9.0,
            min_level_m: None,
            max_level_m: None,
        },
    ];
    let out = volumes_stage(&input(vec![], pools), &flat).unwrap();
    assert_eq!(out.by_reach.len(), 2, "{:?}", out.by_reach);
    assert_eq!(out.by_reach[0].to, out.by_reach[1].from);
    assert_eq!(out.by_reach[0].to.0, 900.0);
    let stations: usize = out.by_reach.iter().map(|r| r.stations).sum();
    assert_eq!(stations, out.stations.len(), "each station in one reach");
    let dredge: f64 = out.by_reach.iter().map(|r| r.dredge_m3).sum();
    let excav: f64 = out.by_reach.iter().map(|r| r.excav_m3).sum();
    assert!(close(dredge, out.totals.dredge_m3));
    assert!(close(excav, out.totals.excav_m3));
    // The upper pool is deeper water over the same ground: less cut per metre.
    let per_m = |i: usize| out.by_reach[i].dredge_m3 / out.by_reach[i].stations as f64;
    assert!(per_m(1) < per_m(0), "{:?}", out.by_reach);
}

#[test]
fn a_section_profile_is_kept_every_500_m_and_where_asked() {
    let mut inp = input(vec![], vec![]);
    inp.profile_at = vec![745.0];
    let out = volumes_stage(&inp, &flat).unwrap();
    assert_eq!(PROFILE_EVERY_M, 500.0);
    let kept: Vec<f64> = out
        .stations
        .iter()
        .filter(|s| s.profile.is_some())
        .map(|s| s.chainage_m)
        .collect();
    // The stations nearest 0, 500, 1000, 1500 and 2000 m, and the one nearest 745 m (stations are 25 m apart).
    assert_eq!(kept, vec![0.0, 500.0, 750.0, 1000.0, 1500.0, 2000.0]);
    let p = out.stations[0].profile.as_ref().unwrap();
    assert_eq!(p.offsets_m.len(), 401);
    assert_eq!(p.terrain_m.len(), 401);
    assert_eq!(p.cut_face_m.len(), 401);
    assert_eq!(p.wet.len(), 401);
}

#[test]
fn the_stage_refuses_by_code() {
    let mut inp = input(vec![], vec![]);
    inp.water.given = None;
    let e = volumes_stage(&inp, &flat).err().unwrap();
    assert_eq!(
        (e.stage.as_str(), e.code.as_str()),
        ("volumes", "VOLUMES_NO_WATER")
    );
    let mut inp = input(vec![], vec![]);
    inp.design_line = Polyline(vec![[0.0, 0.0]]);
    let e = volumes_stage(&inp, &flat).err().unwrap();
    assert_eq!(e.code, "VOLUMES_NO_LINE");
}

/// No terrain anywhere along the line: the raster does not reach it.
fn nothing(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![f64::NAN; xs.len()]
}

/// The flat terrain, ending 50 m past chainage 1500 m (x = 1550): beyond, a whole section has none.
fn ends_at_1550(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .map(|&x| if x <= 1550.0 { TERRAIN_M } else { f64::NAN })
        .collect()
}

/// A sampler that answers with the wrong number of heights.
fn short(xs: &[f64], _ys: &[f64]) -> Vec<f64> {
    vec![TERRAIN_M; xs.len() / 2]
}

#[test]
fn no_terrain_on_the_line_is_refused_with_the_chainage_of_the_first_station() {
    let e = volumes_stage(&input(vec![], vec![]), &nothing)
        .err()
        .unwrap();
    assert_eq!(e.code, "VOLUMES_NO_ELEVATION", "{e}");
    assert_eq!(e.at, Some(Chainage(0.0)));
}

#[test]
fn a_station_off_the_raster_is_refused_with_its_chainage() {
    let e = volumes_stage(&input(vec![], vec![]), &ends_at_1550)
        .err()
        .unwrap();
    assert_eq!(e.code, "VOLUMES_OUTSIDE_RASTER", "{e}");
    // Samples reach 200 m either side of a station across the line, not along it: the first
    // station past x = 1550 has no terrain at all. Stations are 25 m apart.
    assert_eq!(e.at, Some(Chainage(1575.0)));
}

/// Terrain that rises across the section, 0.005 m per metre from the line each way, and that ends
/// 150 m either side of the line: the outer 50 m of each section is off the raster.
fn rising_across_and_narrow(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    xs.iter()
        .zip(ys)
        .map(|(_, &y)| {
            if y.abs() <= 150.0 {
                TERRAIN_M + 0.005 * y.abs()
            } else {
                f64::NAN
            }
        })
        .collect()
}

#[test]
fn a_station_with_part_of_its_section_off_the_raster_counts_and_is_not_refused() {
    // QA on PR 564: only a station with no height at any sample is refused. A station whose section
    // reaches past the raster on both sides counts the part on the raster, by hand: at offset `o`
    // the ground is `c + 0.005 |o|` over the invert, and the face rises 1 in 3 beyond the bottom.
    let size = channel();
    let out = volumes_stage(&input(vec![], vec![]), &rising_across_and_narrow).unwrap();
    let c = TERRAIN_M - (WATER_M - size.design_depth_m);
    let hb = size.bottom_width_straight_m / 2.0;
    let area: f64 = (-150..=150)
        .map(|o| {
            let o = f64::from(o);
            let r = (o.abs() - hb).max(0.0);
            (c + 0.005 * o.abs() - r / 3.0).max(0.0)
        })
        .sum();
    assert!(area > 10.0, "a real cut: {area}");
    // Every station of the 2 km line is there, and each counts the same, worked area.
    assert_eq!(out.stations.len(), 81);
    for st in &out.stations {
        assert!(
            close(st.dredge_area_m2, area),
            "{} m: {} against {area}",
            st.chainage_m,
            st.dredge_area_m2
        );
    }
    assert!(close(out.totals.dredge_m3, 81.0 * area * out.station_m));
}

#[test]
fn an_unknown_cut_model_and_a_broken_sampler_are_refused_by_code() {
    let mut inp = input(vec![], vec![]);
    inp.settings.cut_model = "z".into();
    let e = volumes_stage(&inp, &flat).err().unwrap();
    assert_eq!(e.code, "VOLUMES_CUT_MODEL", "{e}");
    // What is left of the catch-all: a terrain sampler that breaks its contract.
    let e = volumes_stage(&input(vec![], vec![]), &short).err().unwrap();
    assert_eq!(e.code, "VOLUMES_WALK", "{e}");
}

#[test]
fn a_fixed_half_width_does_not_need_the_channel_shape() {
    // A size read back from JSON has no Rule engine shape.
    let read: ChannelSize =
        serde_json::from_value(serde_json::to_value(channel()).unwrap()).unwrap();
    assert!(read.shape().is_err(), "the read size has no shape");
    let mut inp = input(vec![], vec![]);
    inp.size = read.clone();
    let e = volumes_stage(&inp, &flat).err().unwrap();
    assert_eq!(e.code, "SIZING_NO_SHAPE", "{e}");
    inp.settings.half_width_m = Some(20.0);
    let out = volumes_stage(&inp, &flat).unwrap();
    assert!(out.stations.iter().all(|s| s.half_width_m == 20.0));
}

#[test]
fn the_stage_has_a_schema_file() {
    assert!(
        owd_format::stage::contracts()
            .iter()
            .any(|c| c.stage == "volumes")
    );
}

// --- through the session --------------------------------------------------------------------

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;

fn design() -> Session {
    let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
    let ring = [
        [X0 - 100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 - 300.0],
        [X0 + 3100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 + 300.0],
        [X0 - 100.0, Y0 - 300.0],
    ];
    let mut s = Session::new("Volumes", "2026-11-20");
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
    s.set_water_pools(&[[0.0, 12.0], [1000.0, 10.0], [2000.0, 8.0]])
        .unwrap();
    s
}

#[test]
fn the_stored_stations_are_what_the_long_section_and_cross_sections_read() {
    let mut s = design();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    let res = &s.document()["stages"]["volumes"]["results"];
    let stored: Volumes = serde_json::from_value(res["sections"].clone())
        .unwrap_or_else(|e| panic!("the stored sections read as Volumes: {e}"));
    // The totals are the row's numbers, exactly.
    assert_eq!(json!(stored.totals.dredge_m3), res["dredge_m3"]);
    assert_eq!(json!(stored.totals.excav_m3), res["excav_m3"]);
    assert_eq!(json!(stored.totals.total_m3), res["total_m3"]);
    assert_eq!(json!(stored.totals.in_river_m3), res["in_river_m3"]);
    assert_eq!(json!(stored.totals.widening_m3), res["widening_m3"]);
    assert_eq!(json!(stored.totals.cut_through_m3), res["cut_through_m3"]);
    assert_eq!(json!(stored.totals.max_face_h_m), res["max_face_h_m"]);
    assert_eq!(json!(stored.stations.len()), res["stations"]);
    // The reaches are the water stage's stretches: the given pool table, then beyond its last
    // point (held flat).
    let water = s.design_water().unwrap();
    assert_eq!(
        stored.by_reach.len(),
        water.stretches.len(),
        "{:?}",
        stored.by_reach
    );

    let id = s.final_line().unwrap().id.clone();
    let walk = s.option_section_walk(&id, &flat).unwrap();
    let long = s.long_section(&id, &walk).unwrap();
    let col = |k: &str| -> Vec<Value> { long[k].as_array().unwrap().clone() };
    let mine =
        |f: &dyn Fn(&owd_format::design::volumes_stage::VolumeStation) -> f64| -> Vec<Value> {
            stored.stations.iter().map(|st| json!(f(st))).collect()
        };
    assert_eq!(mine(&|st| st.chainage_m), col("chainage_m"));
    assert_eq!(mine(&|st| st.water_level_m), col("water_level_m"));
    assert_eq!(mine(&|st| st.invert_m), col("design_invert_m"));
    assert_eq!(mine(&|st| st.dredge_area_m2), col("dredge_area_m2"));
    assert_eq!(mine(&|st| st.excav_area_m2), col("excav_area_m2"));
    assert_eq!(mine(&|st| st.max_face_h_m), col("max_face_h_m"));
    assert_eq!(
        stored
            .stations
            .iter()
            .map(|st| json!(st.bed_m))
            .collect::<Vec<_>>(),
        col("bed_m")
    );
    // Every profiled station's section is the walk's (more than one is kept).
    let kept: Vec<_> = stored
        .stations
        .iter()
        .filter(|st| st.profile.is_some())
        .collect();
    assert!(kept.len() > 1, "{} profiles kept", kept.len());
    for st in kept {
        let cs = s.cross_section(&walk, st.chainage_m).unwrap();
        assert_eq!(json!(st.chainage_m), cs["chainage_m"]);
        let p = st.profile.as_ref().unwrap();
        assert_eq!(json!(p.offsets_m), cs["offsets_m"]);
        assert_eq!(json!(p.terrain_m), cs["terrain_m"]);
        assert_eq!(json!(p.cut_face_m), cs["cut_face_m"]);
        assert_eq!(json!(p.wet), cs["wet"]);
    }
    // It reopens and validates.
    let back = Session::open(&s.to_text()).unwrap();
    assert!(back.validate().is_empty(), "{:?}", back.validate());
}

#[test]
fn a_refused_volumes_run_leaves_the_document_as_it_was() {
    let mut s = design();
    s.run_volumes(&Value::Null, Some(&flat)).unwrap();
    let before = s.to_text();
    let e = s.run_volumes(&json!({"cut_model": "z"}), Some(&flat));
    assert!(e.is_err());
    assert_eq!(s.to_text(), before, "nothing is half written");
    let e = s.run_volumes(&Value::Null, Some(&nothing));
    assert!(e.is_err());
    assert_eq!(s.to_text(), before, "nothing is half written");
}

/// The document size on the web example, with the stations stored: before (no `sections`), after,
/// and what a profile at every station would have weighed. Run by hand:
/// `cargo test -p owd-format --test volumes_stage document_size -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement for the PR, not a check"]
fn document_size_on_the_web_example() {
    use owd_format::design::terrain::dem_sampler;
    use owd_format::types::TerrainSampler;
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web");
    let text = std::fs::read_to_string(web.join("river-example.owd")).unwrap();
    let dem = owd_format::geotiff::read_bytes(&std::fs::read(web.join("river-dem.tif")).unwrap())
        .unwrap();
    let mut s = Session::open(&text).unwrap();
    let t = dem_sampler(&dem, Some(32630)).unwrap();
    let ter: TerrainSampler = &t;
    s.run_stale_with(Some(ter)).unwrap();
    s.run_stage_with(Stage::Volumes, Some(ter)).unwrap();
    let after = s.to_text();
    let mut doc = s.document().clone();
    let sections = doc["stages"]["volumes"]["results"]["sections"].take();
    doc["stages"]["volumes"]["results"]
        .as_object_mut()
        .unwrap()
        .remove("sections");
    let before = serde_json::to_string_pretty(&doc).unwrap();
    let v: Volumes = serde_json::from_value(sections.clone()).unwrap();
    let profiled = v.stations.iter().filter(|st| st.profile.is_some()).count();
    let one = v
        .stations
        .iter()
        .find_map(|st| st.profile.as_ref())
        .map(|p| serde_json::to_string_pretty(p).unwrap().len())
        .unwrap_or(0);
    println!(
        "stations {}, profiled {profiled}; document before {} bytes, after {} bytes; one profile \
         about {one} bytes, so a profile at every station would add about {} bytes",
        v.stations.len(),
        before.len(),
        after.len(),
        one * (v.stations.len() - profiled)
    );
}
