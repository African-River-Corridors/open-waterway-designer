//! The stages on a sectioned DEM (COG-1, part 2): every reader of the ground asks through one
//! interface, so a sectioned file and the same ground in memory give the same volumes and
//! sections bit for bit (they read full resolution), and the readers that may read an overview (the
//! optimise stage's search grid and flood field, the map's tiles at a far zoom) say so and stay
//! within a stated bound of the full-resolution answer.
//!
//! Label: property

use owd_core::geom::Pt;
use owd_core::synthetic::{RiverParams, synthetic_river};
use owd_format::design::terrain::dem_sampler;
use owd_format::design::terrain_tile::TileMaker;
use owd_format::design::{AlignMethod, Role, Session, Stage};
use owd_format::geotiff::Dem;
use owd_format::sectioned::layout::SampleType;
use owd_format::sectioned::write::{Compression, Spec, write_to_vec};
use owd_format::sectioned::{SectionedDem, Terrain, TerrainRef};
use serde_json::{Value, json};
use std::sync::Arc;

const UTM30: &str = r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
const SHIFT: Pt = [500_000.0, 600_000.0];
/// The DEM: 2 m cells over the river, from x -150 m to 3150 m and y -600 m to 600 m.
const CELL: f64 = 2.0;
const W: usize = 1650;
const H: usize = 600;
const X0: f64 = SHIFT[0] - 150.0;
const Y1: f64 = SHIFT[1] + 600.0;

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

fn grid() -> Vec<f64> {
    let river = synthetic_river(params()).unwrap();
    (0..H * W)
        .map(|i| {
            let (r, c) = (i / W, i % W);
            river.terrain(
                X0 + (c as f64 + 0.5) * CELL - SHIFT[0],
                Y1 - (r as f64 + 0.5) * CELL - SHIFT[1],
            )
        })
        .collect()
}

fn gt() -> [f64; 6] {
    [CELL, 0.0, X0, 0.0, -CELL, Y1]
}

/// The ground in memory and the same ground as a sectioned file (f64, so every cell is the same
/// number), with overviews at 4 m and 8 m.
fn both() -> (Arc<Dem>, Arc<SectionedDem>) {
    let data = grid();
    let mem = Dem {
        width: W,
        height: H,
        data: data.clone(),
        geotransform: gt(),
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    let spec = Spec {
        width: W,
        height: H,
        tile: 256,
        sample: SampleType::F64,
        compression: Compression::Deflate,
        predictor: 3,
        big: None,
        overviews: Some(2),
        nodata: Some(-9999.0),
        geotransform: gt(),
        epsg: Some(32630),
        gap: 0,
    };
    let bytes = write_to_vec(&spec, |r, c| data[r * W + c]).unwrap();
    let sec = SectionedDem::open(Arc::new(bytes), 64 << 20).unwrap();
    assert_eq!(sec.levels().len(), 3);
    (Arc::new(mem), Arc::new(sec))
}

fn settings() -> Value {
    json!({
        "candidate_sites_m": [1000.0, 2000.0],
        "lifts_m": [0.5, 1.0],
        "terminal_min_level_m": 0.5,
        "min_spacing_m": 1000.0,
        "dry_weight": 1.5,
        "structure_fixed": 1.0e5,
        "structure_per_lift": 2.0e5,
        "cut_model": "b",
        "maxw_m": 120.0,
        "rake_half_m": 80.0,
        "rake_step_m": 10.0,
        "ds_step_m": 20.0,
        "pitch_m": 40.0,
        "n_heading": 16,
        "radius_multiples": [1.0, 2.0],
        "corridor_half_m": 150.0,
        "cell_m": 10.0,
    })
}

fn design() -> Session {
    let r = synthetic_river(params()).unwrap();
    let shift = |p: &Pt| [p[0] + SHIFT[0], p[1] + SHIFT[1]];
    let ring: Vec<Pt> = r.polygon.exterior.iter().map(shift).collect();
    let line = [shift(&[0.0, 0.0]), shift(&[3000.0, 0.0])];
    let mut s = Session::new("Pools", "2026-11-26");
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
    let (mem, _) = both();
    s.set_dem("dem.tif", b"dem", &*mem);
    s.set_water_profile(&[[0.0, 0.0]]).unwrap();
    s
}

#[test]
fn volumes_and_sections_are_the_same_bits_from_a_sectioned_file_with_overviews() {
    let (mem, sec) = both();
    let m = dem_sampler(&*mem, Some(32630)).unwrap();
    let k = dem_sampler(&*sec, Some(32630)).unwrap();
    let mut s = design();
    s.run_optimise(&settings(), Some(&m)).unwrap();
    let mut a = s.clone();
    let mut b = s;
    let vol = json!({"cut_model": "b", "maxw_m": 120.0});
    a.run_volumes(&vol, Some(&m)).unwrap();
    b.run_volumes(&vol, Some(&k)).unwrap();
    assert_eq!(
        a.document()["stages"]["volumes"]["results"],
        b.document()["stages"]["volumes"]["results"],
        "the volumes read full resolution, never an overview"
    );
}

#[test]
fn the_optimise_grid_reads_an_overview_says_so_and_stays_near_the_full_resolution_answer() {
    let (mem, sec) = both();
    let m = dem_sampler(&*mem, Some(32630)).unwrap();
    let k = dem_sampler(&*sec, Some(32630)).unwrap();
    let mut a = design();
    let mut b = a.clone();
    a.run_optimise(&settings(), Some(&m)).unwrap();
    b.run_optimise(&settings(), Some(&k)).unwrap();
    let ra = &a.document()["stages"]["optimise"]["results"];
    let rb = &b.document()["stages"]["optimise"]["results"];
    // The weirs are chosen on full-resolution section reads: the same either way.
    assert_eq!(ra["pools"], rb["pools"]);
    assert_eq!(ra["cascade"], rb["cascade"]);
    let lattice = |s: &Session| {
        owd_format::design::design_lines_of(s.document())
            .into_iter()
            .find(|o| o["method"] == "lattice")
            .unwrap()
            .clone()
    };
    let (la, lb) = (lattice(&a), lattice(&b));
    // In memory there is one level: the details are as they always were.
    assert!(la["details"].get("ground_read_cell_m").is_none());
    // The 10 m search grid reads the 8 m overview, and says so.
    assert_eq!(
        lb["details"]["ground_read_cell_m"], 8.0,
        "{}",
        lb["details"]
    );
    assert!(
        lb["details"]["ground_read_note"]
            .as_str()
            .unwrap()
            .contains("full resolution")
    );
    // The bound: each pool's lattice cost within 5 % of the full-resolution one.
    let costs = |l: &Value| -> Vec<f64> {
        l["details"]["pools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["cost"].as_f64().unwrap_or(0.0))
            .collect()
    };
    for (x, y) in costs(&la).iter().zip(costs(&lb)) {
        assert!(
            (x - y).abs() <= 0.05 * x.abs().max(1.0),
            "lattice cost {x} in memory, {y} from the overview"
        );
    }
}

#[test]
fn map_tiles_are_the_same_without_overviews_and_bounded_with_them() {
    let (mem, sec) = both();
    let mm = TileMaker::new(mem.clone() as TerrainRef).unwrap();
    let ms = TileMaker::new(sec.clone() as TerrainRef).unwrap();
    // A tile at the middle of the DEM, at zoom `z`.
    let info = mm.info().clone();
    let (cx, cy) = (
        (info.bounds[0] + info.bounds[2]) / 2.0,
        (info.bounds[1] + info.bounds[3]) / 2.0,
    );
    let tile_of = |z: u8| {
        let n = f64::from(1u32 << z);
        let lr = cy.to_radians();
        (
            ((cx + 180.0) / 360.0 * n) as u32,
            ((1.0 - libm::log(libm::tan(lr) + 1.0 / libm::cos(lr)) / core::f64::consts::PI) / 2.0
                * n) as u32,
        )
    };
    // A deep tile reads full resolution on both.
    let (tx, ty) = tile_of(16);
    assert_eq!(ms.level_at_zoom(16), 0);
    assert_eq!(
        mm.heights(16, tx, ty).unwrap(),
        ms.heights(16, tx, ty).unwrap()
    );
    // A far tile reads the coarsest overview no larger than its pixel, and stays near.
    let (tx, ty) = tile_of(12);
    assert!(ms.level_at_zoom(12) > 0);
    assert_eq!(mm.level_at_zoom(12), 0, "a DEM in memory has one level");
    let (a, b) = (
        mm.heights(12, tx, ty).unwrap(),
        ms.heights(12, tx, ty).unwrap(),
    );
    let diffs: Vec<f32> = a.iter().zip(&b).map(|(p, q)| (p - q).abs()).collect();
    let worst = diffs.iter().copied().fold(0f32, f32::max);
    let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
    eprintln!(
        "zoom 12, level {}: mean {mean} m, worst {worst} m",
        ms.level_at_zoom(12)
    );
    // The overview is a mean of 2 m cells: it rounds off the hill's top and the river's banks
    // (relief of 10 m within 80 m) by up to a few metres at a pixel, and is close on average.
    assert!(
        mean < 0.25,
        "a far tile from the overview is within 0.25 m on average: {mean}"
    );
    assert!(worst < 5.0, "and within 5 m at any pixel: {worst}");
}

#[test]
fn the_hillshade_reads_a_level_no_finer_than_its_pixels() {
    let (mem, sec) = both();
    let a = owd_format::design::hillshade::hillshade(&*mem).unwrap();
    let b = owd_format::design::hillshade::hillshade(&*sec).unwrap();
    assert_eq!(a.corners, b.corners);
    // The file without overviews draws the same picture as the DEM in memory.
    let spec_bytes = {
        let data = grid();
        let spec = Spec {
            width: W,
            height: H,
            tile: 256,
            sample: SampleType::F64,
            compression: Compression::Deflate,
            predictor: 3,
            big: None,
            overviews: Some(0),
            nodata: Some(-9999.0),
            geotransform: gt(),
            epsg: Some(32630),
            gap: 0,
        };
        write_to_vec(&spec, |r, c| data[r * W + c]).unwrap()
    };
    let flat = SectionedDem::open(Arc::new(spec_bytes), 64 << 20).unwrap();
    assert_eq!(
        owd_format::design::hillshade::hillshade(&flat).unwrap().png,
        a.png
    );
}
