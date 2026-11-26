//! An overlay of the map draws nothing where the model has no result (map fault 1): on a sectioned
//! DEM much wider than the design line's corridor, with pools at levels well under the high ground
//! to each side, every tile well outside the corridor is clear for the pool water and the cut, and
//! the terrain with the water on it equals the plain ground there.
//!
//! Label: property

use owd_format::design::crs::Crs;
use owd_format::design::pool_water::PoolWater;
use owd_format::design::structures::PoolWaterIn;
use owd_format::design::terrain_tile::{TILE, TileMaker, decode_png, pixel_lon_lat};
use owd_format::sectioned::layout::SampleType;
use owd_format::sectioned::write::{Compression, Spec, write_to_vec};
use owd_format::sectioned::{SectionedDem, SharedSource, Terrain, TerrainRef};
use std::sync::Arc;

const X0: f64 = 500_000.0;
const Y0: f64 = 600_000.0;
const CELL: f64 = 4.0;
/// The DEM: 8 km east to west, 24 km north to south. The line runs north along its middle.
const W: usize = 2000;
const H: usize = 6000;
const AXIS: f64 = X0 + 4000.0;

/// A valley floor at 108 m on the axis, rising 0.06 m a metre to each side: 125 m at about 280 m
/// out, 200 m at about 1.5 km.
fn ground(x: f64, _y: f64) -> f64 {
    (108.0 + 0.06 * (x - AXIS).abs()).min(230.0)
}

fn dem(nodata_ring: bool) -> TerrainRef {
    let spec = Spec {
        width: W,
        height: H,
        tile: 256,
        sample: SampleType::F32,
        compression: Compression::Deflate,
        predictor: 3,
        big: None,
        overviews: None,
        nodata: Some(-9999.0),
        geotransform: [CELL, 0.0, X0, 0.0, -CELL, Y0 + H as f64 * CELL],
        epsg: Some(32630),
        gap: 0,
    };
    let bytes = write_to_vec(&spec, |r, c| {
        let (x, y) = (
            X0 + (c as f64 + 0.5) * CELL,
            Y0 + H as f64 * CELL - (r as f64 + 0.5) * CELL,
        );
        if nodata_ring && !(50..W - 50).contains(&c) {
            return -9999.0;
        }
        ground(x, y)
    })
    .unwrap();
    Arc::new(SectionedDem::open(Arc::new(bytes) as SharedSource, 64 << 20).unwrap())
}

fn lon_lat(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
        unreachable!()
    };
    u.inverse_all(pts).unwrap()
}

/// Two pools along a line from 1 km to 23 km north, given as a water-level table: 113 m and 116 m.
fn pools() -> PoolWaterIn {
    let line: Vec<[f64; 2]> = (0..=22)
        .map(|i| {
            [
                AXIS + 30.0 * libm::sin(f64::from(i)),
                Y0 + 1000.0 + 1000.0 * f64::from(i),
            ]
        })
        .collect();
    let mut chainage_m = vec![0.0];
    for i in 1..line.len() {
        let s = chainage_m[i - 1]
            + libm::hypot(line[i][0] - line[i - 1][0], line[i][1] - line[i - 1][1]);
        chainage_m.push(s);
    }
    let end = *chainage_m.last().unwrap();
    let river = [
        [AXIS - 60.0, Y0 + 500.0],
        [AXIS + 60.0, Y0 + 500.0],
        [AXIS + 60.0, Y0 + 23_500.0],
        [AXIS - 60.0, Y0 + 23_500.0],
        [AXIS - 60.0, Y0 + 500.0],
    ];
    PoolWaterIn {
        line: lon_lat(&line),
        chainage_m,
        pools: vec![(1, 0.0, end / 2.0, 113.0), (2, end / 2.0, end, 116.0)],
        polygon: vec![lon_lat(&river)],
    }
}

fn tile_at(z: u8, ll: [f64; 2]) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let x = ((ll[0] + 180.0) / 360.0 * n) as u32;
    let lat = ll[1].to_radians();
    let y = ((1.0 - libm::log(libm::tan(lat) + 1.0 / libm::cos(lat)) / core::f64::consts::PI) / 2.0
        * n) as u32;
    (x, y)
}

/// The pixels of tile `z/x/y` more than 1.5 km from the axis (and on the DEM, inside its edges).
fn far_pixels(z: u8, x: u32, y: u32) -> Vec<usize> {
    let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
        unreachable!()
    };
    let ll: Vec<[f64; 2]> = (0..TILE * TILE)
        .map(|i| pixel_lon_lat(z, x, y, i % TILE, i / TILE))
        .collect();
    let xy = u.forward_all(&ll).unwrap();
    (0..TILE * TILE)
        .filter(|&i| {
            let [px, py] = xy[i];
            (px - AXIS).abs() > 1500.0
                && px > X0 + 300.0
                && px < X0 + W as f64 * CELL - 300.0
                && py > Y0 + 1500.0
                && py < Y0 + 22_500.0
        })
        .collect()
}

fn alpha(png: &[u8]) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut px = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut px).unwrap();
    px.as_chunks::<4>().0.iter().map(|c| c[3]).collect()
}

#[test]
fn the_pool_water_is_clear_well_outside_the_corridor_and_its_terrain_is_the_ground() {
    for nodata_ring in [false, true] {
        let dem = dem(nodata_ring);
        let mut maker = TileMaker::new(dem.clone()).unwrap();
        // A mask cell of tens of metres, as on a DEM of a billion cells.
        let mut water = PoolWater::with_budget(&maker, &pools(), 20_000).unwrap();
        assert!(water.stats().factor >= 10, "{:?}", water.stats());
        // 2.5 km east and west of the line, in the middle of each pool, at several zooms.
        for (dx, y) in [
            (2500.0, 6000.0),
            (-2500.0, 6000.0),
            (2500.0, 18_000.0),
            (-2500.0, 18_000.0),
        ] {
            let ll = lon_lat(&[[AXIS + dx, Y0 + y]])[0];
            for z in [10u8, 12, 13, 14, 15] {
                let (x, yy) = tile_at(z, ll);
                // Only tiles that lie wholly outside 1.5 km of the line.
                let far = far_pixels(z, x, yy);
                let a = alpha(&water.depth_tile(&maker, z, x, yy).unwrap());
                let wet = far.iter().filter(|i| a[**i] > 0).count();
                let surface = decode_png(&water.surface_tile(&maker, z, x, yy).unwrap()).unwrap();
                let plain = decode_png(&maker.tile(z, x, yy).unwrap()).unwrap();
                let differ = far
                    .iter()
                    .filter(|i| (surface[**i] - plain[**i]).abs() > 0.01)
                    .count();

                assert!(far.len() > 1000, "{dx},{y} z{z}: {} pixels", far.len());
                assert_eq!(
                    wet, 0,
                    "pool water at {dx},{y} z{z} (nodata ring {nodata_ring})"
                );
                assert_eq!(
                    differ, 0,
                    "water terrain at {dx},{y} z{z} (nodata ring {nodata_ring})"
                );
            }
        }
    }
}

mod cut {
    //! The cut-depth tiles and the After terrain well outside the corridor, on a design made
    //! through the stages: a river 3 km long, three pools from a water-level table.
    use super::{alpha, tile_at};
    use owd_format::design::crs::Crs;
    use owd_format::design::terrain::dem_sampler;
    use owd_format::design::terrain_tile::{
        TILE, TerrainState, TileMaker, decode_png, pixel_lon_lat,
    };
    use owd_format::design::{AlignMethod, Role, Session, Stage};
    use owd_format::geotiff::Dem;
    use serde_json::{Value, json};
    use std::rc::Rc;
    use std::sync::Arc;

    const UTM30: &str =
        r#","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::32630"}}"#;
    const X0: f64 = 500_000.0;
    const Y0: f64 = 600_000.0;
    const CELL: f64 = 5.0;

    /// River bed at 4 m on the axis, rising 0.04 m a metre to each side, to 140 m at 3.4 km.
    fn ground(_x: f64, y: f64) -> f64 {
        4.0 + 0.04 * (y - Y0).abs()
    }

    fn dem() -> Dem {
        let (w, h) = (680usize, 1400usize);
        let (left, top) = (X0 - 200.0, Y0 + 3500.0);
        let data = (0..w * h)
            .map(|i| {
                let (r, c) = (i / w, i % w);
                ground(
                    left + (c as f64 + 0.5) * CELL,
                    top - (r as f64 + 0.5) * CELL,
                )
            })
            .collect();
        Dem {
            width: w,
            height: h,
            data,
            geotransform: [CELL, 0.0, left, 0.0, -CELL, top],
            nodata: Some(-9999.0),
            epsg: Some(32630),
        }
    }

    fn design(dem: &Dem) -> Session {
        let centre: Vec<[f64; 2]> = (0..=30).map(|i| [X0 + 100.0 * i as f64, Y0]).collect();
        let ring = [
            [X0 - 100.0, Y0 - 60.0],
            [X0 + 3100.0, Y0 - 60.0],
            [X0 + 3100.0, Y0 + 60.0],
            [X0 - 100.0, Y0 + 60.0],
            [X0 - 100.0, Y0 - 60.0],
        ];
        let mut s = Session::new("Channel", "2026-11-20");
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
        s.set_dem("dem.tif", b"a stand-in terrain file", dem);
        s.set_water_pools(&[[0.0, 12.0], [1000.0, 10.0], [2000.0, 8.0]])
            .unwrap();
        s
    }

    #[test]
    fn the_cut_is_clear_and_the_after_ground_is_the_before_ground_well_outside_the_corridor() {
        let dem = dem();
        let mut s = design(&dem);
        let t = dem_sampler(&dem, Some(32630)).unwrap();
        s.run_volumes(&Value::Null, Some(&t)).unwrap();
        let channel = s.channel(Some(&t)).unwrap();
        let cut = s.cut_surface(&channel, &dem).unwrap().unwrap();
        let cut = Rc::new(cut);
        let terrain: owd_format::sectioned::TerrainRef = Arc::new(dem.clone());
        let mut maker = TileMaker::new(terrain).unwrap();
        maker.set_overlay(
            TerrainState::After,
            Some(cut.clone() as Rc<dyn owd_format::design::terrain_tile::Overlay>),
        );
        let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
            unreachable!()
        };
        for dy in [-2000.0, 2000.0] {
            let ll = u.inverse_all(&[[X0 + 1500.0, Y0 + dy]]).unwrap()[0];
            for z in [12u8, 13, 14, 15] {
                let (x, y) = tile_at(z, ll);
                let xy = u
                    .forward_all(
                        &(0..TILE * TILE)
                            .map(|i| pixel_lon_lat(z, x, y, i % TILE, i / TILE))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                let far: Vec<usize> = (0..TILE * TILE)
                    .filter(|&i| {
                        (xy[i][1] - Y0).abs() > 1000.0
                            && (xy[i][1] - Y0).abs() < 3300.0
                            && xy[i][0] > X0 - 100.0
                            && xy[i][0] < X0 + 3200.0
                    })
                    .collect();
                let png = cut.depth_tile(&maker, z, x, y).unwrap().unwrap_or_default();
                let wet = if png.is_empty() {
                    0
                } else {
                    let a = alpha(&png);
                    far.iter().filter(|i| a[**i] > 0).count()
                };
                let before = decode_png(&maker.tile(z, x, y).unwrap()).unwrap();
                let after =
                    decode_png(&maker.tile_in(TerrainState::After, z, x, y).unwrap()).unwrap();
                let differ = far
                    .iter()
                    .filter(|i| (before[**i] - after[**i]).abs() > 0.01)
                    .count();
                assert_eq!((wet, differ), (0, 0), "dy {dy} z{z}");
            }
        }
    }
}

#[test]
fn a_far_tile_of_the_terrain_with_the_water_reads_no_more_blocks_than_the_plain_terrain() {
    let dem = dem(false);
    let sec = || dem.sectioned().expect("a sectioned DEM");
    let maker = TileMaker::new(dem.clone()).unwrap();
    let mut water = PoolWater::with_budget(&maker, &pools(), 20_000).unwrap();
    let ll = lon_lat(&[[AXIS + 2500.0, Y0 + 6000.0]])[0];
    for z in [9u8, 10, 11] {
        let (x, y) = tile_at(z, ll);
        let misses = |f: &mut dyn FnMut()| {
            sec().set_cache_budget(0);
            sec().set_cache_budget(64 << 20);
            let before = sec().cache_stats().misses;
            f();
            sec().cache_stats().misses - before
        };
        let plain = misses(&mut || {
            maker.heights(z, x, y).unwrap();
        });
        // What the water's terrain read before map fault 1 was fixed: every pixel from the
        // full-resolution level.
        let full = misses(&mut || {
            maker.at_pixels(z, x, y, 0, 0.0, |px, py| maker.height_at(px, py));
        });
        let with_water = misses(&mut || {
            water.surface(&maker, z, x, y).unwrap();
        });
        assert!(
            full > 4 * plain,
            "z{z}: {full} blocks at full resolution, {plain} plain"
        );
        // The water's own cells (and the mask) are read at full resolution: a few blocks more.
        assert!(
            with_water <= plain + full / 4,
            "z{z}: {with_water} blocks with the water, {plain} plain, {full} at full resolution"
        );
        let _ = &mut water;
    }
}

/// A file whose bytes in `bad` cannot be read: a lost permission, a file changed on disk.
struct Holes {
    bytes: Vec<u8>,
    bad: Vec<(u64, u64)>,
}

impl owd_format::sectioned::ByteSource for Holes {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        let end = offset + buf.len() as u64;
        if self.bad.iter().any(|(a, b)| offset < *b && end > *a) {
            return Err("the file could not be read".into());
        }
        buf.copy_from_slice(&self.bytes[offset as usize..end as usize]);
        Ok(())
    }
}

#[test]
fn ground_that_cannot_be_read_is_no_result_never_low_ground() {
    // The DEM's full-resolution blocks of its western 1 km cannot be read; its overviews can.
    let whole = dem(false);
    let layout = whole.sectioned().unwrap().image(0).clone();
    let across = layout.across();
    let bad: Vec<(u64, u64)> = (0..layout.offsets.len())
        .filter(|i| i % across == 0)
        .map(|i| (layout.offsets[i], layout.offsets[i] + layout.counts[i]))
        .collect();
    let bytes = (0..whole.sectioned().unwrap().source().len())
        .step_by(1 << 20)
        .flat_map(|o| {
            let n = (1u64 << 20).min(whole.sectioned().unwrap().source().len() - o) as usize;
            whole.sectioned().unwrap().source().read_vec(o, n).unwrap()
        })
        .collect();
    let dem: TerrainRef = Arc::new(
        SectionedDem::open(Arc::new(Holes { bytes, bad }) as SharedSource, 64 << 20).unwrap(),
    );
    // Pools higher than all the ground: every readable cell joined to the line would flood.
    let mut high = pools();
    for p in &mut high.pools {
        p.3 = 300.0;
    }
    let maker = TileMaker::new(dem.clone()).unwrap();
    let mut water = PoolWater::with_budget(&maker, &high, 20_000).unwrap();
    let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
        unreachable!()
    };
    let ll = lon_lat(&[[X0 + 300.0, Y0 + 12_000.0]])[0];
    let (x, y) = tile_at(15, ll);
    let xy = u
        .forward_all(
            &(0..TILE * TILE)
                .map(|i| pixel_lon_lat(15, x, y, i % TILE, i / TILE))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    // The pixels on the unreadable blocks (with a cell's margin for the bilinear read).
    let lost: Vec<usize> = (0..TILE * TILE)
        .filter(|&i| xy[i][0] > X0 + 8.0 && xy[i][0] < X0 + 256.0 * CELL - 8.0)
        .collect();
    assert!(lost.len() > 10_000, "{}", lost.len());
    for &i in &lost {
        let ll = pixel_lon_lat(15, x, y, i % TILE, i / TILE);
        assert_eq!(
            water.at(&maker, ll[0], ll[1]),
            None,
            "no water read on lost ground"
        );
    }
    let a = alpha(&water.depth_tile(&maker, 15, x, y).unwrap());
    assert!(
        lost.iter().all(|i| a[*i] == 0),
        "the water picture is clear there"
    );
    let heights = decode_png(&water.surface_tile(&maker, 15, x, y).unwrap()).unwrap();
    let base = maker.info().base_m;
    assert!(
        lost.iter().all(|i| (heights[*i] - base).abs() < 0.01),
        "the terrain stands at the DEM's base height there, never at 0 m"
    );
    assert!(dem.read_error().is_some(), "the lost blocks are said");
}
