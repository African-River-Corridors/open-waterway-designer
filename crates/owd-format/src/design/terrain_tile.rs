//! Terrain tiles for the map's 3D view (MAP-2): the design's DEM cut into Web Mercator tiles, made
//! on demand, in memory. The map asks for tile `z/x/y`; each pixel of the tile is sampled from the
//! DEM where it lies (bilinear, through the DEM's own CRS), and the heights are written as a
//! Terrarium PNG, the encoding a `raster-dem` map source reads.
//!
//! Ground the DEM does not cover is given the DEM's base height: the middle one of its heights
//! (the median), which in river country is the level of the plain. A map draws a missing terrain
//! tile at height 0, which would put a cliff down to sea level around a DEM of high ground; a
//! tile is made for every address instead, so there is no hole. Just outside the DEM's edge the
//! ground eases from the height at the edge to the base height over an apron a tenth of the DEM
//! wide, so the edge is no cliff either. A nodata hole inside the DEM is flat at the base height:
//! its rim is a step of the difference, the one place a step is left.

use super::crs::Crs;
use super::session::DesignError;
use crate::sectioned::TerrainRef;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// The side of a tile, in pixels.
pub const TILE: usize = 256;
/// The tiles kept in memory. A tile is a PNG of at most about 200 kB, so the cache stays under
/// about 50 MB.
pub const CACHE_TILES: usize = 256;
/// The deepest zoom level the maker names as worth asking for.
const MAX_ZOOM: u8 = 16;
/// The width of the apron around the DEM, as a part of its shorter side.
const APRON: f64 = 0.1;
/// Earth radius of the spherical Web Mercator model, in metres.
const R: f64 = 6_378_137.0;

/// A height in metres as a Terrarium pixel: `height = r * 256 + g + b / 256 - 32768`, so a step is
/// 1/256 m. Heights outside -32768 m to 32768 m are held at the nearest end.
pub fn encode_terrarium(height_m: f64) -> [u8; 3] {
    let q = ((height_m + 32768.0) * 256.0)
        .round()
        .clamp(0.0, 16_777_215.0) as u32;
    [(q >> 16) as u8, (q >> 8) as u8, q as u8]
}

/// The height in metres of a Terrarium pixel.
pub fn decode_terrarium(rgb: [u8; 3]) -> f64 {
    f64::from(rgb[0]) * 256.0 + f64::from(rgb[1]) + f64::from(rgb[2]) / 256.0 - 32768.0
}

/// What the map needs to know about the tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct TileInfo {
    /// West, south, east and north of the DEM, in degrees.
    pub bounds: [f64; 4],
    /// The lowest and highest height in the DEM, in metres.
    pub min_m: f64,
    pub max_m: f64,
    /// The height given to ground the DEM does not cover: the median of its heights.
    pub base_m: f64,
    /// The zoom level at which a tile pixel is as small as a DEM cell. Deeper tiles add nothing.
    pub max_zoom: u8,
    pub tile_size: u32,
}

type Forward = Box<dyn Fn([f64; 2]) -> Option<[f64; 2]>>;
/// A tile in the cache: when it was last asked for, and its PNG.
type Kept = (u64, Arc<Vec<u8>>);

/// Makes the terrain tiles of one DEM, and keeps the last [`CACHE_TILES`] of them.
pub struct TileMaker {
    dem: TerrainRef,
    /// A tile pixel at zoom 0, in the DEM's CRS units: halved at each zoom level. Picks the level
    /// of the DEM a tile reads (COG-1): the coarsest whose cell is no larger than a pixel.
    pixel0: f64,
    /// Longitude and latitude in degrees to the DEM's CRS. `None`: the DEM is in degrees already.
    forward: Option<Forward>,
    info: TileInfo,
    /// The DEM's box grown by the apron, in degrees: a tile that misses it is flat.
    reach: [f64; 4],
    /// The box of the outermost pixel centres, in the DEM's CRS: inside it every point has four
    /// neighbours to read a height from.
    inner: [f64; 4],
    /// The width of the apron around the DEM, in the units of its CRS.
    apron: f64,
    flat: Arc<Vec<u8>>,
    cache: HashMap<(u8, u32, u32), Kept>,
    clock: u64,
    capacity: usize,
    /// What is laid on the ground in each state other than Before (see [`TerrainState`]).
    overlays: HashMap<TerrainState, Rc<dyn Overlay>>,
    /// The tiles of those states, kept apart from the tiles of the ground as it is: at most
    /// `capacity` more.
    state_cache: HashMap<(TerrainState, u8, u32, u32), Kept>,
}

impl core::fmt::Debug for TileMaker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TileMaker")
            .field("info", &self.info)
            .field("cached", &self.cache.len())
            .finish()
    }
}

fn input(message: impl Into<String>) -> DesignError {
    DesignError::Input(message.into())
}

/// The latitude in degrees of a Web Mercator `v` (0 at the north edge of the world, 1 at the south).
fn lat_of(v: f64) -> f64 {
    libm::atan(libm::sinh(core::f64::consts::PI * (1.0 - 2.0 * v))).to_degrees()
}

/// West, south, east and north of tile `z/x/y`, in degrees.
pub fn tile_bounds(z: u8, x: u32, y: u32) -> [f64; 4] {
    let n = f64::from(1u32 << z);
    [
        f64::from(x) / n * 360.0 - 180.0,
        lat_of((f64::from(y) + 1.0) / n),
        (f64::from(x) + 1.0) / n * 360.0 - 180.0,
        lat_of(f64::from(y) / n),
    ]
}

/// The longitude and latitude of a place in tile `z/x/y`, `col` and `row` pixels from its
/// north-west corner.
fn lon_lat_at(z: u8, x: u32, y: u32, col: f64, row: f64) -> [f64; 2] {
    let n = f64::from(1u32 << z);
    let u = (f64::from(x) + col / TILE as f64) / n;
    let v = (f64::from(y) + row / TILE as f64) / n;
    [u * 360.0 - 180.0, lat_of(v)]
}

/// The longitude and latitude of the centre of pixel (`col`, `row`) of tile `z/x/y`.
pub fn pixel_lon_lat(z: u8, x: u32, y: u32, col: usize, row: usize) -> [f64; 2] {
    lon_lat_at(z, x, y, col as f64 + 0.5, row as f64 + 0.5)
}

/// The pixels between two points of the lattice a tile is projected on (see
/// [`TileMaker::heights`]).
const LATTICE_STEP: usize = 16;
/// The lattice points along a side of a tile.
const LATTICE: usize = TILE / LATTICE_STEP + 1;
/// From this zoom level in, a tile is projected on the lattice. At zoom 8 a lattice square is
/// about 10 km, and a straight line across it is within about 2 m of the projection's curve:
/// 1/300 of a tile pixel there, and four times less at each level deeper.
const LATTICE_FROM_ZOOM: u8 = 8;

impl TileMaker {
    /// A tile maker for `dem`. Refused when the DEM does not name a CRS the tool reads, or holds
    /// no height at all.
    pub fn new(dem: TerrainRef) -> Result<TileMaker, DesignError> {
        TileMaker::with_capacity(dem, CACHE_TILES)
    }

    /// [`TileMaker::new`] with a cache of `capacity` tiles.
    pub fn with_capacity(dem: TerrainRef, capacity: usize) -> Result<TileMaker, DesignError> {
        let crs = dem
            .epsg()
            .ok_or_else(|| input("the DEM does not name its CRS"))
            .and_then(|e| Crs::from_epsg(e).map_err(DesignError::Input))?;
        // The lowest, highest and middle height: of every cell of a DEM in memory; of a coarse
        // overview of a sectioned one (see `Terrain::summary`).
        let Some(summary) = dem.summary() else {
            return Err(input("the DEM holds no heights"));
        };
        let (min_m, max_m, base_m) = (summary.min, summary.max, summary.median);
        // The DEM's box in degrees: its edges, walked, since a straight edge in a projected CRS
        // is a curve in longitude and latitude.
        let (x0, y0, x1, y1) = dem.bounds();
        let box_ll = |grow: f64| -> Result<[f64; 4], DesignError> {
            let (x0, y0, x1, y1) = (x0 - grow, y0 - grow, x1 + grow, y1 + grow);
            let edge: Vec<[f64; 2]> = (0..=16)
                .flat_map(|i| {
                    let t = f64::from(i) / 16.0;
                    let (x, y) = (x0 + t * (x1 - x0), y0 + t * (y1 - y0));
                    [[x, y0], [x, y1], [x0, y], [x1, y]]
                })
                .collect();
            let edge_ll = match crs {
                Crs::Wgs84 => edge,
                Crs::Utm(u) => u.inverse_all(&edge).map_err(DesignError::Input)?,
            };
            let fold = |f: fn(f64, f64) -> f64, i: usize, init: f64| {
                edge_ll.iter().fold(init, |a, c| f(a, c[i]))
            };
            Ok([
                fold(f64::min, 0, f64::INFINITY),
                fold(f64::min, 1, f64::INFINITY),
                fold(f64::max, 0, f64::NEG_INFINITY),
                fold(f64::max, 1, f64::NEG_INFINITY),
            ])
        };
        let bounds = box_ll(0.0)?;
        let apron = APRON * (x1 - x0).min(y1 - y0);
        // A little more than the apron, so a tile that misses this box is flat for certain.
        let reach = box_ll(apron * 1.05)?;
        let gt = dem.geotransform();
        let (half_x, half_y) = (gt[0].abs() / 2.0, gt[4].abs() / 2.0);
        // Just inside the last pixel centres, where the bilinear read still has its neighbours.
        let inner = [
            x0 + half_x * 1.000_001,
            y0 + half_y * 1.000_001,
            x1 - half_x * 1.000_001,
            y1 - half_y * 1.000_001,
        ];
        if !bounds.iter().all(|v| v.is_finite()) || bounds[0] >= bounds[2] || bounds[1] >= bounds[3]
        {
            return Err(input("the DEM covers no area"));
        }
        let mid_lat = (bounds[1] + bounds[3]) / 2.0;
        let cos_lat = libm::cos(mid_lat.to_radians()).max(1e-6);
        let cell_m = match crs {
            Crs::Wgs84 => gt[0].abs().to_radians() * R * cos_lat,
            Crs::Utm(_) => gt[0].abs(),
        };
        // A tile pixel at zoom z is `2 pi R cos(lat) / (TILE * 2^z)` metres wide.
        let world_px = 2.0 * core::f64::consts::PI * R * cos_lat / TILE as f64;
        let max_zoom = libm::ceil(libm::log2(world_px / cell_m.max(1e-3)))
            .clamp(0.0, f64::from(MAX_ZOOM)) as u8;
        let forward: Option<Forward> = match crs {
            Crs::Wgs84 => None,
            Crs::Utm(u) => Some(Box::new(u.forward_fn().map_err(DesignError::Input)?)),
        };
        let flat = Arc::new(encode_png(&vec![base_m as f32; TILE * TILE])?);
        let pixel0 = world_px * gt[0].abs() / cell_m.max(1e-9);
        Ok(TileMaker {
            dem,
            pixel0,
            forward,
            info: TileInfo {
                bounds,
                min_m,
                max_m,
                base_m,
                max_zoom,
                tile_size: TILE as u32,
            },
            reach,
            inner,
            apron,
            flat,
            cache: HashMap::new(),
            clock: 0,
            capacity: capacity.max(1),
            overlays: HashMap::new(),
            state_cache: HashMap::new(),
        })
    }

    pub fn info(&self) -> &TileInfo {
        &self.info
    }

    /// The DEM the tiles are cut from.
    pub fn dem(&self) -> &TerrainRef {
        &self.dem
    }

    /// The level of the DEM the tiles of zoom `z` read: the coarsest whose cell is no larger than
    /// a tile pixel. Always 0 for a DEM in memory (it has one level).
    pub fn level_at_zoom(&self, z: u8) -> usize {
        self.dem
            .level_for_cell(self.pixel0 / f64::from(1u32 << z.min(31)))
    }

    /// The number of tiles in the cache now.
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Whether tile `z/x/y` holds any of the DEM's box or of the apron around it.
    fn touches(&self, z: u8, x: u32, y: u32) -> bool {
        let [w, s, e, n] = tile_bounds(z, x, y);
        let [dw, ds, de, dn] = self.reach;
        w < de && e > dw && s < dn && n > ds
    }

    /// The height drawn at a point in the DEM's CRS: the DEM's own where it has one; the base
    /// height in a nodata hole; and outside the DEM the height at the nearest point of its edge,
    /// eased to the base height across the apron.
    pub fn height_at(&self, px: f64, py: f64) -> f64 {
        self.height_at_level(0, px, py)
    }

    /// [`TileMaker::height_at`] read from `level` of the DEM.
    pub fn height_at_level(&self, level: usize, px: f64, py: f64) -> f64 {
        let h = self.dem.sample_bilinear_level(level, px, py);
        if h.is_finite() {
            return h;
        }
        let base = self.info.base_m;
        let [x0, y0, x1, y1] = self.inner;
        if !(x0 < x1 && y0 < y1) {
            return base;
        }
        let (cx, cy) = (px.clamp(x0, x1), py.clamp(y0, y1));
        let d = libm::hypot(px - cx, py - cy);
        if d == 0.0 || d >= self.apron {
            return base;
        }
        let edge = self.dem.sample_bilinear_level(level, cx, cy);
        if !edge.is_finite() {
            return base;
        }
        let t = d / self.apron;
        let t = t * t * (3.0 - 2.0 * t);
        edge * (1.0 - t) + base * t
    }

    /// The heights of tile `z/x/y` in metres, row by row from the north-west pixel, each sampled
    /// at its pixel centre (see [`TileMaker::height_at`]).
    pub fn heights(&self, z: u8, x: u32, y: u32) -> Result<Vec<f32>, DesignError> {
        check(z, x, y)?;
        let base = self.info.base_m as f32;
        if !self.touches(z, x, y) {
            return Ok(vec![base; TILE * TILE]);
        }
        let level = self.level_at_zoom(z);
        Ok(self.sampled(z, x, y, 0, base, |px, py| {
            self.height_at_level(level, px, py) as f32
        }))
    }

    /// The DEM's own heights on the grid of tile `z/x/y`, with `margin` more pixels on every
    /// side: `TILE + 2 * margin` heights a side, row by row from the north-west, each at a pixel
    /// centre. NaN where the DEM has no height (off the DEM, or a nodata cell): no base height and
    /// no apron, since a contour is drawn only where the ground is known.
    pub fn ground(&self, z: u8, x: u32, y: u32, margin: usize) -> Result<Vec<f32>, DesignError> {
        check(z, x, y)?;
        let level = self.level_at_zoom(z);
        Ok(self.sampled(z, x, y, margin, f32::NAN, |px, py| {
            self.dem.sample_bilinear_level(level, px, py) as f32
        }))
    }

    /// Whether tile `z/x/y` holds any of the DEM's own box.
    pub fn holds_dem(&self, z: u8, x: u32, y: u32) -> bool {
        let [w, s, e, n] = tile_bounds(z, x, y);
        let [dw, ds, de, dn] = self.info.bounds;
        w < de && e > dw && s < dn && n > ds
    }

    /// A point given as longitude and latitude in degrees, in the DEM's CRS. `None` where the
    /// projection refuses it.
    pub(crate) fn to_dem(&self, lon_lat: [f64; 2]) -> Option<[f64; 2]> {
        match &self.forward {
            None => Some(lon_lat),
            Some(f) => f(lon_lat),
        }
    }

    /// `height(px, py)` in the DEM's CRS at every pixel centre of tile `z/x/y` and of `margin`
    /// pixels around it. A pixel the projection refuses is `unplaced`.
    pub(crate) fn sampled<T: Copy>(
        &self,
        z: u8,
        x: u32,
        y: u32,
        margin: usize,
        unplaced: T,
        height: impl Fn(f64, f64) -> T,
    ) -> Vec<T> {
        let side = TILE + 2 * margin;
        let mut out = vec![unplaced; side * side];
        // Projecting every pixel is most of the work. A tile is small beside the projection's
        // curve, so only a lattice of points is projected, and a pixel is placed between the
        // four around it (a pixel of the margin: just beyond them, on the same straight lines).
        // A lattice point the projection refuses sends its pixels the long way.
        let lattice: Option<Vec<Option<[f64; 2]>>> = match &self.forward {
            Some(f) if z >= LATTICE_FROM_ZOOM => Some(
                (0..LATTICE * LATTICE)
                    .map(|i| {
                        let (r, c) = (i / LATTICE, i % LATTICE);
                        let (col, row) = ((c * LATTICE_STEP) as f64, (r * LATTICE_STEP) as f64);
                        f(lon_lat_at(z, x, y, col, row))
                    })
                    .collect(),
            ),
            _ => None,
        };
        let between = |col: f64, row: f64| -> Option<[f64; 2]> {
            let lattice = lattice.as_ref()?;
            let (fc, fr) = (col / LATTICE_STEP as f64, row / LATTICE_STEP as f64);
            let last = (LATTICE - 2) as f64;
            let (c, r) = (
                libm::floor(fc).clamp(0.0, last) as usize,
                libm::floor(fr).clamp(0.0, last) as usize,
            );
            let (tc, tr) = (fc - c as f64, fr - r as f64);
            let (a, b) = (lattice[r * LATTICE + c]?, lattice[r * LATTICE + c + 1]?);
            let (d, e) = (
                lattice[(r + 1) * LATTICE + c]?,
                lattice[(r + 1) * LATTICE + c + 1]?,
            );
            let mix = |i: usize| {
                (a[i] * (1.0 - tc) + b[i] * tc) * (1.0 - tr) + (d[i] * (1.0 - tc) + e[i] * tc) * tr
            };
            Some([mix(0), mix(1)])
        };
        for j in 0..side {
            for i in 0..side {
                // The pixel's centre, in tile pixels from the north-west corner of the tile.
                let (col, row) = (
                    i as f64 - margin as f64 + 0.5,
                    j as f64 - margin as f64 + 0.5,
                );
                let at = between(col, row).or_else(|| {
                    let ll = lon_lat_at(z, x, y, col, row);
                    match &self.forward {
                        None => Some(ll),
                        Some(f) => f(ll),
                    }
                });
                if let Some([px, py]) = at {
                    out[j * side + i] = height(px, py);
                }
            }
        }
        out
    }

    /// Tile `z/x/y` as a Terrarium PNG, from the cache when it is there.
    pub fn tile(&mut self, z: u8, x: u32, y: u32) -> Result<Arc<Vec<u8>>, DesignError> {
        check(z, x, y)?;
        if !self.touches(z, x, y) {
            return Ok(self.flat.clone());
        }
        self.clock += 1;
        if let Some(hit) = self.cache.get_mut(&(z, x, y)) {
            hit.0 = self.clock;
            return Ok(hit.1.clone());
        }
        let png = Arc::new(encode_png(&self.heights(z, x, y)?)?);
        if self.cache.len() >= self.capacity
            && let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
        {
            self.cache.remove(&oldest);
        }
        self.cache.insert((z, x, y), (self.clock, png.clone()));
        Ok(png)
    }
}

/// The ground a terrain tile shows (MAP-5). `Before` is the DEM as it is. Each other state is the
/// DEM with one [`Overlay`] laid on it ([`TileMaker::set_overlay`]); a state with no overlay shows
/// the DEM. A new variant is one more arm here, with its name: the tile maker is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TerrainState {
    /// The existing ground.
    #[default]
    Before,
    /// The ground with the channel cut into it.
    After,
}

impl TerrainState {
    /// The state's name in a tile address.
    pub fn name(self) -> &'static str {
        match self {
            TerrainState::Before => "before",
            TerrainState::After => "after",
        }
    }

    /// The state a tile address names.
    pub fn from_name(name: &str) -> Option<TerrainState> {
        [TerrainState::Before, TerrainState::After]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// A surface laid on the DEM's ground in one [`TerrainState`].
pub trait Overlay {
    /// West, south, east and north, in degrees, of the ground it may change. A tile that misses
    /// this box is the tile of the ground as it is.
    fn bounds(&self) -> [f64; 4];
    /// The level at a point of the DEM's CRS where the DEM's own ground is `ground` (finite).
    fn level(&self, x: f64, y: f64, ground: f64) -> f64;
}

impl TileMaker {
    /// Lay `overlay` on the ground in `state` (`None`: take it off). The tiles kept for the state
    /// are forgotten. `Before` takes no overlay: it is the ground as it is.
    pub fn set_overlay(&mut self, state: TerrainState, overlay: Option<Rc<dyn Overlay>>) {
        self.state_cache.retain(|k, _| k.0 != state);
        match overlay {
            Some(o) if state != TerrainState::Before => {
                self.overlays.insert(state, o);
            }
            _ => {
                self.overlays.remove(&state);
            }
        }
    }

    /// The overlay of `state` when tile `z/x/y`, with `margin` pixels around it, holds any of its
    /// box.
    fn overlay_on(
        &self,
        state: TerrainState,
        z: u8,
        x: u32,
        y: u32,
        margin: usize,
    ) -> Option<&Rc<dyn Overlay>> {
        let overlay = self.overlays.get(&state)?;
        let [w, s, e, n] = tile_bounds(z, x, y);
        let grow = margin as f64 / TILE as f64;
        let (dx, dy) = ((e - w) * grow, (n - s) * grow);
        let [ow, os, oe, on] = overlay.bounds();
        (w - dx < oe && e + dx > ow && s - dy < on && n + dy > os).then_some(overlay)
    }

    /// [`TileMaker::heights`] of the ground in `state`.
    pub fn heights_in(
        &self,
        state: TerrainState,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Vec<f32>, DesignError> {
        let Some(overlay) = self.overlay_on(state, z, x, y, 0) else {
            return self.heights(z, x, y);
        };
        check(z, x, y)?;
        let base = self.info.base_m as f32;
        let level = self.level_at_zoom(z);
        Ok(self.at_pixels(z, x, y, 0, base, |px, py| {
            let ground = self.dem.sample_bilinear_level(level, px, py);
            if ground.is_finite() {
                overlay.level(px, py, ground)
            } else {
                self.height_at_level(level, px, py)
            }
        }))
    }

    /// [`TileMaker::ground`] in `state`: the overlay's level where the DEM has a height, NaN
    /// where it has none.
    pub fn ground_in(
        &self,
        state: TerrainState,
        z: u8,
        x: u32,
        y: u32,
        margin: usize,
    ) -> Result<Vec<f32>, DesignError> {
        let Some(overlay) = self.overlay_on(state, z, x, y, margin) else {
            return self.ground(z, x, y, margin);
        };
        check(z, x, y)?;
        let level = self.level_at_zoom(z);
        Ok(self.at_pixels(z, x, y, margin, f32::NAN, |px, py| {
            let ground = self.dem.sample_bilinear_level(level, px, py);
            if ground.is_finite() {
                overlay.level(px, py, ground)
            } else {
                ground
            }
        }))
    }

    /// `value(px, py)` at every pixel centre of tile `z/x/y` and of `margin` pixels around it,
    /// the point given in the DEM's CRS: for a picture laid on the terrain (the depth of cut).
    /// A pixel the projection refuses is `unplaced`.
    pub fn at_pixels(
        &self,
        z: u8,
        x: u32,
        y: u32,
        margin: usize,
        unplaced: f32,
        value: impl Fn(f64, f64) -> f64,
    ) -> Vec<f32> {
        self.sampled(z, x, y, margin, unplaced, |px, py| value(px, py) as f32)
    }

    /// [`TileMaker::tile`] of the ground in `state`. A tile the state's overlay does not reach is
    /// the tile of the ground as it is, kept once.
    pub fn tile_in(
        &mut self,
        state: TerrainState,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, DesignError> {
        check(z, x, y)?;
        if self.overlay_on(state, z, x, y, 0).is_none() {
            return self.tile(z, x, y);
        }
        self.clock += 1;
        if let Some(hit) = self.state_cache.get_mut(&(state, z, x, y)) {
            hit.0 = self.clock;
            return Ok(hit.1.clone());
        }
        let png = Arc::new(encode_png(&self.heights_in(state, z, x, y)?)?);
        if self.state_cache.len() >= self.capacity
            && let Some(oldest) = self
                .state_cache
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
        {
            self.state_cache.remove(&oldest);
        }
        self.state_cache
            .insert((state, z, x, y), (self.clock, png.clone()));
        Ok(png)
    }
}

pub(crate) fn check(z: u8, x: u32, y: u32) -> Result<(), DesignError> {
    if z > 24 || x >= (1u32 << z) || y >= (1u32 << z) {
        return Err(input(format!("there is no map tile {z}/{x}/{y}")));
    }
    Ok(())
}

/// A tile of heights as a Terrarium PNG (8-bit RGB).
pub fn encode_png(heights: &[f32]) -> Result<Vec<u8>, DesignError> {
    if heights.len() != TILE * TILE {
        return Err(input("a terrain tile is 256 by 256 heights"));
    }
    let mut px = Vec::with_capacity(TILE * TILE * 3);
    for &h in heights {
        px.extend_from_slice(&encode_terrarium(f64::from(h)));
    }
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, TILE as u32, TILE as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        // A tile is made while the user pans: speed matters more than the last bytes.
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| input(e.to_string()))?;
        writer
            .write_image_data(&px)
            .map_err(|e| input(e.to_string()))?;
    }
    Ok(png)
}

/// The heights in a Terrarium PNG, row by row.
pub fn decode_png(png: &[u8]) -> Result<Vec<f64>, DesignError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .map_err(|e| input(e.to_string()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| input("the tile is too large to read"))?;
    let mut buf = vec![0; size];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| input(e.to_string()))?;
    if frame.color_type != png::ColorType::Rgb || frame.bit_depth != png::BitDepth::Eight {
        return Err(input("the tile is not an 8-bit RGB picture"));
    }
    Ok(buf[..frame.buffer_size()]
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| decode_terrarium(*p))
        .collect())
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::design::crs::Utm;
    use crate::geotiff::Dem;

    const ZONE: Utm = Utm {
        zone: 30,
        south: false,
    };
    /// The plane every test DEM is cut from: 100 m at the origin, rising 2 m per km east and
    /// 1 m per km north.
    fn plane(x: f64, y: f64) -> f64 {
        100.0 + (x - 500_000.0) * 0.002 + (y - 600_000.0) * 0.001
    }

    /// A UTM DEM of the plane: `n` by `n` cells of `cell` metres, its north-west corner at
    /// (500000, 600000 + n * cell).
    fn utm_plane(n: usize, cell: f64) -> Dem {
        let top = 600_000.0 + n as f64 * cell;
        let data = (0..n * n)
            .map(|i| {
                let (r, c) = (i / n, i % n);
                plane(
                    500_000.0 + (c as f64 + 0.5) * cell,
                    top - (r as f64 + 0.5) * cell,
                )
            })
            .collect();
        Dem {
            width: n,
            height: n,
            data,
            geotransform: [cell, 0.0, 500_000.0, 0.0, -cell, top],
            nodata: Some(-9999.0),
            epsg: Some(ZONE.epsg()),
        }
    }

    /// The tile at zoom `z` that holds a point.
    fn tile_at(z: u8, lon: f64, lat: f64) -> (u32, u32) {
        let n = f64::from(1u32 << z);
        let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
        let s = libm::sin(lat.to_radians());
        let v = 0.5 - libm::log((1.0 + s) / (1.0 - s)) / (4.0 * core::f64::consts::PI);
        (x, (v * n).floor() as u32)
    }

    fn middle(dem: &Dem) -> [f64; 2] {
        let (x0, y0, x1, y1) = dem.bounds();
        ZONE.inverse((x0 + x1) / 2.0, (y0 + y1) / 2.0).unwrap()
    }

    #[test]
    fn terrarium_round_trips_to_a_256th_of_a_metre() {
        for h in [-420.0, -0.004, 0.0, 0.5, 12.3456, 311.99, 8848.86] {
            let back = decode_terrarium(encode_terrarium(h));
            assert!(
                (back - h).abs() <= 1.0 / 512.0 + 1e-12,
                "{h} came back {back}"
            );
        }
        assert_eq!(encode_terrarium(0.0), [128, 0, 0]);
        assert_eq!(decode_terrarium([128, 1, 128]), 1.5);
        // Out of range is held at the ends, never wrapped.
        assert_eq!(encode_terrarium(-1e9), [0, 0, 0]);
        assert_eq!(encode_terrarium(1e9), [255, 255, 255]);
    }

    #[test]
    fn a_plane_dem_gives_the_planes_height_at_every_tile_pixel() {
        let dem = Arc::new(utm_plane(400, 10.0));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let [lon, lat] = middle(&dem);
        // Zoom 14: a tile is about 2.4 km, inside the 4 km DEM around its middle or across its edge.
        let (x, y) = tile_at(14, lon, lat);
        let heights = maker.heights(14, x, y).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        let mut inside = 0;
        for (row, col) in [
            (0, 0),
            (17, 200),
            (128, 128),
            (200, 31),
            (255, 255),
            (90, 250),
        ] {
            let [plon, plat] = pixel_lon_lat(14, x, y, col, row);
            let [px, py] = ZONE.forward(plon, plat).unwrap();
            let got = f64::from(heights[row * TILE + col]);
            // Half a cell in from the edge the DEM has no neighbour to interpolate with.
            if px > x0 + 5.0 && px < x1 - 5.0 && py > y0 + 5.0 && py < y1 - 5.0 {
                inside += 1;
                assert!(
                    (got - plane(px, py)).abs() < 1e-3,
                    "pixel {row},{col}: {got} for {}",
                    plane(px, py)
                );
            } else {
                // Off the DEM: on the apron, between the heights the DEM has.
                let info = maker.info();
                assert!(
                    got >= info.min_m - 1e-3 && got <= info.max_m + 1e-3,
                    "pixel {row},{col}: {got}"
                );
            }
        }
        assert!(inside >= 3, "the tile is mostly on the DEM ({inside} of 6)");
        // And through the PNG: the same heights, to the encoding's step.
        let mut maker = maker;
        let png = maker.tile(14, x, y).unwrap();
        let decoded = decode_png(&png).unwrap();
        assert_eq!(decoded.len(), TILE * TILE);
        for (a, b) in decoded.iter().zip(&heights) {
            assert!(
                (a - f64::from(*b)).abs() <= 1.0 / 512.0 + 1e-4,
                "{a} vs {b}"
            );
        }
    }

    #[test]
    fn the_lattice_places_a_pixel_where_the_projection_does() {
        // A big DEM of the plane, 100 km of 250 m cells: at zoom 8, the first level on the
        // lattice and the worst, a height is within 1 cm of the projection's own (the plane
        // rises 2 m per km, so that is 5 m on the ground, under 1/100 of a tile pixel).
        let dem = Arc::new(utm_plane(400, 250.0));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let [lon, lat] = middle(&dem);
        for z in [8u8, 11] {
            let (x, y) = tile_at(z, lon, lat);
            let heights = maker.heights(z, x, y).unwrap();
            let (x0, y0, x1, y1) = dem.bounds();
            let mut checked = 0;
            for row in (0..TILE).step_by(7) {
                for col in (0..TILE).step_by(5) {
                    let [plon, plat] = pixel_lon_lat(z, x, y, col, row);
                    let [px, py] = ZONE.forward(plon, plat).unwrap();
                    if px > x0 + 500.0 && px < x1 - 500.0 && py > y0 + 500.0 && py < y1 - 500.0 {
                        checked += 1;
                        let got = f64::from(heights[row * TILE + col]);
                        assert!(
                            (got - plane(px, py)).abs() < 0.01,
                            "z{z} {row},{col}: {got}"
                        );
                    }
                }
            }
            assert!(checked > 100, "z{z}: {checked} pixels on the DEM");
        }
    }

    #[test]
    fn a_dem_in_degrees_is_sampled_without_a_projection() {
        let (n, step) = (200usize, 0.0005);
        let data = (0..n * n).map(|i| 50.0 + (i % n) as f64 * 0.25).collect();
        let dem = Arc::new(Dem {
            width: n,
            height: n,
            data,
            geotransform: [step, 0.0, -2.0, 0.0, -step, 5.1],
            nodata: None,
            epsg: Some(4326),
        });
        let maker = TileMaker::new(dem).unwrap();
        let (x, y) = tile_at(13, -1.95, 5.05);
        let heights = maker.heights(13, x, y).unwrap();
        let [lon, lat] = pixel_lon_lat(13, x, y, 128, 128);
        assert!(lon > -2.0 + step && lon < -1.9 - step && lat < 5.1 - step && lat > 5.0 + step);
        // Heights rise 0.25 m per cell east; pixel centres are half a cell in.
        let expect = 50.0 + ((lon + 2.0) / step - 0.5) * 0.25;
        assert!((f64::from(heights[128 * TILE + 128]) - expect).abs() < 1e-3);
    }

    #[test]
    fn nodata_takes_the_base_height_and_never_the_nodata_value() {
        let mut dem = utm_plane(400, 10.0);
        // A hole of 2 km around the middle.
        for r in 100..300 {
            for c in 100..300 {
                dem.data[r * 400 + c] = -9999.0;
            }
        }
        let lowest = dem
            .data
            .iter()
            .copied()
            .filter(|v| *v != -9999.0)
            .fold(f64::INFINITY, f64::min);
        let dem = Arc::new(dem);
        let mut maker = TileMaker::new(dem.clone()).unwrap();
        assert_eq!(maker.info().min_m, lowest, "the nodata value is no height");
        // The base is the median of the heights: on this plane, near its middle height.
        let lowest = maker.info().base_m;
        assert!(
            (lowest - plane(502_000.0, 602_000.0)).abs() < 1.0,
            "{lowest}"
        );
        let [lon, lat] = middle(&dem);
        let (x, y) = tile_at(16, lon, lat);
        // Zoom 16: the whole tile (about 600 m) is in the hole.
        let decoded = decode_png(&maker.tile(16, x, y).unwrap()).unwrap();
        for h in &decoded {
            assert!(
                (h - lowest).abs() < 0.01,
                "{h} in the hole, the base is {lowest}"
            );
        }
    }

    #[test]
    fn a_tile_outside_the_dem_is_flat_at_the_base_height() {
        let dem = Arc::new(utm_plane(400, 10.0));
        let mut maker = TileMaker::new(dem.clone()).unwrap();
        let base = maker.info().base_m;
        // The far side of the world, and the tile next to the DEM's own.
        let [lon, lat] = middle(&dem);
        let (x, y) = tile_at(12, lon, lat);
        for (z, tx, ty) in [(3, 6, 2), (12, x + 3, y), (12, x, y - 3), (0, 0, 0)] {
            let png = maker.tile(z, tx, ty).unwrap();
            if z > 0 {
                let decoded = decode_png(&png).unwrap();
                assert!(
                    decoded.iter().all(|h| (h - base).abs() < 0.01),
                    "tile {z}/{tx}/{ty} is flat"
                );
            }
        }
        // Tiles off the DEM are one shared picture: they take no room in the cache.
        assert_eq!(
            maker.cached(),
            1,
            "only the world tile, which holds the DEM"
        );
        assert!(
            maker
                .tile(5, 99, 0)
                .unwrap_err()
                .to_string()
                .contains("5/99/0")
        );
    }

    #[test]
    fn the_dems_edge_is_no_cliff_the_ground_eases_to_the_base_across_the_apron() {
        // 4 km of the plane: 100 m at the west edge, 108 m at the east, the base about 106 m.
        let dem = Arc::new(utm_plane(400, 10.0));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let base = maker.info().base_m;
        let y = 602_000.0;
        // Walk west out of the DEM in 5 m steps: no step in height is more than a few centimetres.
        let mut last = maker.height_at(500_100.0, y);
        assert!(
            (last - plane(500_100.0, y)).abs() < 1e-9,
            "on the DEM: its own height"
        );
        let mut x = 500_100.0;
        while x > 499_000.0 {
            x -= 5.0;
            let h = maker.height_at(x, y);
            assert!(
                (h - last).abs() < 0.2,
                "a step of {} m at x = {x}",
                h - last
            );
            last = h;
        }
        // The apron is a tenth of the DEM (400 m): past it the ground is the base height.
        assert!(
            maker.height_at(499_999.0, y) < base - 3.0,
            "at the edge: the edge's height"
        );
        assert_eq!(maker.height_at(499_590.0, y), base);
        assert_eq!(maker.height_at(400_000.0, 100_000.0), base);
        // Off a corner too.
        let corner = maker.height_at(499_900.0, 599_900.0);
        assert!(
            corner > plane(500_005.0, 600_005.0) && corner < base,
            "{corner}"
        );
    }

    #[test]
    fn the_cache_is_bounded_and_gives_the_same_tile_again() {
        let dem = Arc::new(utm_plane(400, 10.0));
        let mut maker = TileMaker::with_capacity(dem.clone(), 3).unwrap();
        let [lon, lat] = middle(&dem);
        let first = {
            let (x, y) = tile_at(14, lon, lat);
            maker.tile(14, x, y).unwrap()
        };
        let again = {
            let (x, y) = tile_at(14, lon, lat);
            maker.tile(14, x, y).unwrap()
        };
        assert!(
            Arc::ptr_eq(&first, &again),
            "the second ask is the cached tile"
        );
        for z in 8..14 {
            let (x, y) = tile_at(z, lon, lat);
            maker.tile(z, x, y).unwrap();
            assert!(maker.cached() <= 3, "{} tiles kept", maker.cached());
        }
        // The first tile was pushed out; asked again it is made again, equal byte for byte.
        let (x, y) = tile_at(14, lon, lat);
        let remade = maker.tile(14, x, y).unwrap();
        assert!(!Arc::ptr_eq(&first, &remade));
        assert_eq!(*first, *remade);
    }

    #[test]
    fn the_info_names_the_dems_box_and_the_zoom_of_its_cells() {
        let dem = Arc::new(utm_plane(400, 10.0));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let info = maker.info();
        let [w, s] = ZONE.inverse(500_000.0, 600_000.0).unwrap();
        let [e, n] = ZONE.inverse(504_000.0, 604_000.0).unwrap();
        for (got, want) in info.bounds.iter().zip([w, s, e, n]) {
            assert!((got - want).abs() < 1e-4, "{got} vs {want}");
        }
        // A zoom-14 pixel is about 9.5 m near the equator: the first as small as a 10 m cell.
        assert_eq!(info.max_zoom, 14);
        assert!(info.min_m < info.base_m && info.base_m < info.max_m);
    }

    #[test]
    fn a_dem_with_no_crs_or_no_heights_is_refused_with_the_reason() {
        let mut d = utm_plane(8, 10.0);
        d.epsg = None;
        let e = TileMaker::new(Arc::new(d)).unwrap_err().to_string();
        assert!(e.contains("CRS"), "{e}");
        let mut d = utm_plane(8, 10.0);
        d.data.fill(-9999.0);
        let e = TileMaker::new(Arc::new(d)).unwrap_err().to_string();
        assert!(e.contains("no heights"), "{e}");
    }

    /// The time to make a tile, on a 4000 by 4000 DEM of 6 m cells. Run with
    /// `cargo test --release -p owd-format terrain_tile -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing, not a check"]
    fn time_per_tile_on_a_4000_by_4000_dem() {
        let dem = Arc::new(utm_plane(4000, 6.0));
        let start = std::time::Instant::now();
        let mut maker = TileMaker::new(dem.clone()).unwrap();
        println!("maker: {:.1} ms", start.elapsed().as_secs_f64() * 1e3);
        let [lon, lat] = middle(&dem);
        for z in [10u8, 12, 14, 15] {
            let (x, y) = tile_at(z, lon, lat);
            let start = std::time::Instant::now();
            let heights = maker.heights(z, x, y).unwrap();
            let sampled = start.elapsed().as_secs_f64() * 1e3;
            let start = std::time::Instant::now();
            let png = encode_png(&heights).unwrap();
            let encoded = start.elapsed().as_secs_f64() * 1e3;
            let start = std::time::Instant::now();
            maker.tile(z, x, y).unwrap();
            maker.tile(z, x, y).unwrap();
            let both = start.elapsed().as_secs_f64() * 1e3;
            println!(
                "z{z}: sample {sampled:.2} ms, encode {encoded:.2} ms ({} bytes), make + cached read {both:.2} ms",
                png.len()
            );
        }
    }
}
