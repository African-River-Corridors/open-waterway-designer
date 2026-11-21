//! The pool water (MAP-4): the ground each pool floods, made from the DEM in memory and shown on
//! the map as tiles. Nothing here is stored in the design.
//!
//! The rule. A ground cell is flooded by a pool when
//!
//! 1. it lies in that pool's reach: the nearest point of the design line to it is in the pool;
//! 2. its DEM level is below the pool's water level; and
//! 3. it is joined to the design line or the river polygon through other flooded cells of the
//!    same pool, so a hollow far from the river stays dry.
//!
//! A map tile cannot see whether a cell is joined to the river, so that is worked out once for
//! the whole DEM, on a **flood mask**: a grid over the DEM of at most [`MASK_CELLS`] cells. A
//! DEM with more cells than that is covered by mask cells of `factor` by `factor` DEM cells, and
//! a mask cell counts as below the level when any DEM cell in it is. The mask holds, for each
//! cell, its pool and whether the pool's water reaches it. A tile then reads the DEM at its full
//! detail: a pixel is water when its mask cell is reached and its own ground is below the level.
//! The search starts on the design line and spreads only through cells below the level, so its
//! work grows with the flooded area, not with the DEM.
//!
//! Where two pools meet (a weir) the two levels meet on the line of points as near to one pool's
//! reach as to the other's: at the weir this is the line across the river through the weir site.
//! Each side takes its own pool's level, and the water of the upper pool does not pass to the
//! lower side. Where the ground on that line is below the upper level, the model shows a wall of
//! water there: a real scheme needs a bank or a wall on that line, which nothing here designs.
//!
//! The design line itself joins the pool even where its ground is above the water (the channel
//! is cut there), but only ground below the level is drawn as water. Beyond the two ends of the
//! design line nothing is flooded: the model stops at the line across each end.

use super::session::DesignError;
use super::structures::PoolWaterIn;
use super::terrain_tile::{TILE, TileMaker, check, encode_png};
use crate::geotiff::Dem;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The most cells the flood mask has. At three bytes a cell kept (nine while it is made), the
/// mask of the largest DEM is 12 MB (36 MB while it is made).
pub const MASK_CELLS: usize = 4_000_000;
/// The tiles of each kind kept in memory.
pub const CACHE_TILES: usize = 256;
/// Earth radius of the spherical Web Mercator model, in metres.
const R: f64 = 6_378_137.0;
/// A mask cell in no pool.
const NO_POOL: u16 = u16::MAX;
/// A mask cell whose count of DEM cells below the level is not known yet.
const UNKNOWN: u16 = u16::MAX;
/// The pool's water reaches the mask cell.
const FLOODED: u8 = 1;
/// The mask cell's centre is inside the river polygon.
const RIVER: u8 = 2;

/// The colour of the water by its depth: `(depth in metres, [red, green, blue, alpha])`, read
/// straight between the stops and held beyond the last. Deeper is darker.
pub const DEPTH_STOPS: [(f64, [u8; 4]); 4] = [
    (0.0, [158, 202, 225, 140]),
    (2.0, [66, 146, 198, 170]),
    (5.0, [33, 102, 172, 195]),
    (10.0, [8, 48, 107, 220]),
];

/// The colour of water `depth_m` deep.
pub fn depth_colour(depth_m: f64) -> [u8; 4] {
    let d = depth_m.max(0.0);
    for w in DEPTH_STOPS.windows(2) {
        let ((d0, c0), (d1, c1)) = (w[0], w[1]);
        if d <= d1 {
            let t = (d - d0) / (d1 - d0);
            let mut c = [0u8; 4];
            for i in 0..4 {
                c[i] = (f64::from(c0[i]) + t * (f64::from(c1[i]) - f64::from(c0[i]))).round() as u8;
            }
            return c;
        }
    }
    DEPTH_STOPS[DEPTH_STOPS.len() - 1].1
}

/// The ground one pool floods.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PoolArea {
    /// The pool's number, from the river mouth.
    pub number: usize,
    pub level_m: f64,
    /// All the ground the pool floods, hectares.
    pub flooded_ha: f64,
    /// The flooded ground outside the river polygon, hectares: the land the pool drowns. `None`
    /// when the design has no river polygon.
    pub outside_river_ha: Option<f64>,
}

/// The size of the flood mask.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaskStats {
    /// A mask cell is this many DEM cells on a side.
    pub factor: usize,
    pub width: usize,
    pub height: usize,
    /// The side of a mask cell on the ground, metres (west to east).
    pub cell_m: f64,
    /// The memory the mask keeps, bytes.
    pub bytes: usize,
    /// The most memory used while it was made, bytes.
    pub peak_bytes: usize,
    /// The mask cells the water reaches.
    pub flooded_cells: usize,
}

/// What a click on the water reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WaterAt {
    /// The pool's number.
    pub pool: usize,
    pub water_level_m: f64,
    pub ground_m: f64,
    pub depth_m: f64,
}

type Kept = (u64, Arc<Vec<u8>>);

/// The last tiles made of one kind.
#[derive(Debug, Default)]
struct Cache {
    tiles: HashMap<(u8, u32, u32), Kept>,
    clock: u64,
}

impl Cache {
    fn get(&mut self, key: (u8, u32, u32)) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        let hit = self.tiles.get_mut(&key)?;
        hit.0 = self.clock;
        Some(hit.1.clone())
    }

    fn put(&mut self, key: (u8, u32, u32), tile: Arc<Vec<u8>>, capacity: usize) {
        if self.tiles.len() >= capacity
            && let Some(oldest) = self
                .tiles
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
        {
            self.tiles.remove(&oldest);
        }
        self.tiles.insert(key, (self.clock, tile));
    }
}

/// The flood mask of one DEM and one set of pools, and the tiles made from it.
#[derive(Debug)]
pub struct PoolWater {
    dem: Arc<Dem>,
    factor: usize,
    width: usize,
    /// The pool of each mask cell, as an index into `levels`; [`NO_POOL`] for none.
    pool: Vec<u16>,
    /// [`FLOODED`] and [`RIVER`] of each mask cell.
    state: Vec<u8>,
    levels: Vec<f64>,
    numbers: Vec<usize>,
    areas: Vec<PoolArea>,
    stats: MaskStats,
    depth: Cache,
    surface: Cache,
    blank: Arc<Vec<u8>>,
}

fn refuse(message: impl Into<String>) -> DesignError {
    DesignError::Input(message.into())
}

/// A point of the design line on the mask, with its pool.
struct Seed {
    x: f32,
    y: f32,
    pool: u16,
}

/// The DEM cells of mask cell (`mi`, `mj`) whose level is below `level`.
fn below(dem: &Dem, factor: usize, mi: usize, mj: usize, level: f64) -> u16 {
    let (r1, c1) = (
        ((mj + 1) * factor).min(dem.height),
        ((mi + 1) * factor).min(dem.width),
    );
    let mut n = 0u32;
    for r in mj * factor..r1 {
        let row = &dem.data[r * dem.width + mi * factor..r * dem.width + c1];
        for &v in row {
            if v < level && dem.nodata != Some(v) {
                n += 1;
            }
        }
    }
    n.min(u32::from(UNKNOWN) - 1) as u16
}

impl PoolWater {
    /// The flood mask of `input`'s pools on the DEM of `maker`.
    pub fn build(maker: &TileMaker, input: &PoolWaterIn) -> Result<PoolWater, DesignError> {
        PoolWater::with_budget(maker, input, MASK_CELLS)
    }

    /// [`PoolWater::build`] with a mask of at most `cells` cells.
    pub fn with_budget(
        maker: &TileMaker,
        input: &PoolWaterIn,
        cells: usize,
    ) -> Result<PoolWater, DesignError> {
        if input.pools.is_empty() || input.line.len() < 2 {
            return Err(input_error());
        }
        if input.pools.len() >= usize::from(NO_POOL) || input.line.len() != input.chainage_m.len() {
            return Err(refuse("the pools of the design cannot be read"));
        }
        let dem = maker.dem().clone();
        let (w, h) = (dem.width, dem.height);
        let factor = (libm::ceil(libm::sqrt((w * h) as f64 / cells.max(1) as f64)) as usize).max(1);
        let (mw, mh) = (w.div_ceil(factor), h.div_ceil(factor));
        let [a, _, c, _, e, f] = dem.geotransform;
        let k = factor as f64;
        // The side of a DEM cell on the ground, metres. A DEM in degrees: at the latitude of each row.
        let degrees = dem.epsg == Some(4326);
        let row_lat = |row: f64| (f + e * row).to_radians();
        let cell_x = |row: f64| {
            if degrees {
                a.abs().to_radians() * R * libm::cos(row_lat(row)).max(1e-6)
            } else {
                a.abs()
            }
        };
        let cell_y = if degrees {
            e.abs().to_radians() * R
        } else {
            e.abs()
        };
        // A mask cell in metres, for the distance to the line: one size for the whole DEM.
        let (sx, sy) = (cell_x(h as f64 / 2.0) * k, cell_y * k);

        // The design line on the mask: a seed every half cell, each with its pool.
        let on_mask = |ll: [f64; 2]| {
            maker
                .to_dem(ll)
                .map(|[x, y]| [(x - c) / a / k, (y - f) / e / k])
        };
        let pool_of = |s: f64| -> u16 {
            let n = input.pools.len();
            input
                .pools
                .iter()
                .position(|p| s >= p.1 - 1e-6 && s < p.2)
                .or_else(|| ((s - input.pools[n - 1].2).abs() < 1e-6).then_some(n - 1))
                .map_or(NO_POOL, |i| i as u16)
        };
        let pts: Vec<Option<[f64; 2]>> = input.line.iter().map(|p| on_mask(*p)).collect();
        let mut seeds: Vec<Seed> = Vec::new();
        let mut joined = false;
        for i in 0..pts.len() - 1 {
            let (Some(p0), Some(p1)) = (pts[i], pts[i + 1]) else {
                joined = false;
                continue;
            };
            let len = libm::hypot(p1[0] - p0[0], p1[1] - p0[1]);
            let steps = (libm::ceil(len / 0.5) as usize).max(1);
            for j in usize::from(joined)..=steps {
                let t = j as f64 / steps as f64;
                let s = input.chainage_m[i] + t * (input.chainage_m[i + 1] - input.chainage_m[i]);
                seeds.push(Seed {
                    x: (p0[0] + t * (p1[0] - p0[0])) as f32,
                    y: (p0[1] + t * (p1[1] - p0[1])) as f32,
                    pool: pool_of(s),
                });
            }
            joined = true;
        }
        if seeds.len() < 2 || seeds.len() >= u32::MAX as usize {
            return Err(refuse("the design line cannot be placed on the DEM"));
        }
        // The line across each end of the design line: beyond it nothing is flooded.
        let n_pts = pts.len();
        let outward = |end: Option<[f64; 2]>, inner: Option<[f64; 2]>| {
            let (end, inner) = (end?, inner?);
            Some([(end[0] - inner[0]) * sx, (end[1] - inner[1]) * sy])
        };
        let ends: [(u32, Option<[f64; 2]>); 2] = [
            (0, outward(pts[0], pts[1])),
            (
                (seeds.len() - 1) as u32,
                outward(pts[n_pts - 1], pts[n_pts - 2]),
            ),
        ];

        // The nearest seed to each mask cell: seeded on the line, then spread in two sweeps.
        let d2 = |i: usize, j: usize, s: &Seed| {
            let (dx, dy) = (
                (i as f64 + 0.5 - f64::from(s.x)) * sx,
                (j as f64 + 0.5 - f64::from(s.y)) * sy,
            );
            dx * dx + dy * dy
        };
        const NONE: u32 = u32::MAX;
        let mut near = vec![NONE; mw * mh];
        let mut line_cells: Vec<u32> = Vec::new();
        for (n, s) in seeds.iter().enumerate() {
            if !(s.x >= 0.0 && s.y >= 0.0 && (s.x as usize) < mw && (s.y as usize) < mh) {
                continue;
            }
            let (i, j) = (s.x as usize, s.y as usize);
            let cell = j * mw + i;
            // An end of the line keeps its cell, so that the cells beyond it find it.
            let is_end = |n: u32| ends.iter().any(|(end, _)| *end == n);
            if near[cell] == NONE {
                line_cells.push(cell as u32);
                near[cell] = n as u32;
            } else if !is_end(near[cell])
                && (is_end(n as u32) || d2(i, j, s) < d2(i, j, &seeds[near[cell] as usize]))
            {
                near[cell] = n as u32;
            }
        }
        if line_cells.is_empty() {
            return Err(refuse("the design line does not cross the DEM"));
        }
        let take = |near: &mut [u32], i: usize, j: usize, ni: isize, nj: isize| {
            if ni < 0 || nj < 0 || ni >= mw as isize || nj >= mh as isize {
                return;
            }
            let s = near[nj as usize * mw + ni as usize];
            if s == NONE {
                return;
            }
            let cur = near[j * mw + i];
            if cur == NONE || d2(i, j, &seeds[s as usize]) < d2(i, j, &seeds[cur as usize]) {
                near[j * mw + i] = s;
            }
        };
        for j in 0..mh {
            let (ji, up) = (j as isize, j as isize - 1);
            for i in 0..mw {
                let ii = i as isize;
                take(&mut near, i, j, ii - 1, ji);
                take(&mut near, i, j, ii - 1, up);
                take(&mut near, i, j, ii, up);
                take(&mut near, i, j, ii + 1, up);
            }
            for i in (0..mw).rev() {
                take(&mut near, i, j, i as isize + 1, ji);
            }
        }
        for j in (0..mh).rev() {
            let (ji, down) = (j as isize, j as isize + 1);
            for i in (0..mw).rev() {
                let ii = i as isize;
                take(&mut near, i, j, ii + 1, ji);
                take(&mut near, i, j, ii + 1, down);
                take(&mut near, i, j, ii, down);
                take(&mut near, i, j, ii - 1, down);
            }
            for i in 0..mw {
                take(&mut near, i, j, i as isize - 1, ji);
            }
        }
        let mut pool = vec![NO_POOL; mw * mh];
        for j in 0..mh {
            for i in 0..mw {
                let n = near[j * mw + i];
                if n == NONE {
                    continue;
                }
                let s = &seeds[n as usize];
                let beyond = ends.iter().any(|(end, out)| {
                    *end == n
                        && out.is_some_and(|o| {
                            (i as f64 + 0.5 - f64::from(s.x)) * sx * o[0]
                                + (j as f64 + 0.5 - f64::from(s.y)) * sy * o[1]
                                > 0.0
                        })
                });
                if !beyond {
                    pool[j * mw + i] = s.pool;
                }
            }
        }
        let peak_bytes = near.len() * 9 + seeds.len() * core::mem::size_of::<Seed>();
        drop(near);
        drop(seeds);

        // The river polygon on the mask: a cell is in the river when its centre is.
        let mut state = vec![0u8; mw * mh];
        let has_polygon = !input.polygon.is_empty();
        if has_polygon {
            let mut rows: Vec<Vec<f32>> = vec![Vec::new(); mh];
            for ring in &input.polygon {
                let ring: Vec<[f64; 2]> = ring.iter().filter_map(|p| on_mask(*p)).collect();
                for edge in ring.windows(2) {
                    let (p, q) = (edge[0], edge[1]);
                    let (lo, hi) = (p[1].min(q[1]), p[1].max(q[1]));
                    // The rows whose centre line `y = j + 0.5` the edge crosses: lo <= y < hi.
                    let first = libm::ceil(lo - 0.5).max(0.0) as usize;
                    let mut j = first;
                    while j < mh && (j as f64 + 0.5) < hi {
                        let y = j as f64 + 0.5;
                        rows[j].push((p[0] + (y - p[1]) / (q[1] - p[1]) * (q[0] - p[0])) as f32);
                        j += 1;
                    }
                }
            }
            for (j, xs) in rows.iter_mut().enumerate() {
                xs.sort_by(f32::total_cmp);
                for pair in xs.as_chunks::<2>().0 {
                    let from = libm::ceil(f64::from(pair[0]) - 0.5).max(0.0) as usize;
                    let to = (libm::ceil(f64::from(pair[1]) - 0.5).max(0.0) as usize).min(mw);
                    for cell in &mut state[j * mw + from.min(to)..j * mw + to] {
                        *cell |= RIVER;
                    }
                }
            }
        }

        // The search: from the design line, and from the river where it is below its pool's
        // level, through the cells of the same pool that are below the level.
        let levels: Vec<f64> = input.pools.iter().map(|p| p.3).collect();
        let mut wet = vec![UNKNOWN; mw * mh];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &cell in &line_cells {
            let cell = cell as usize;
            if pool[cell] != NO_POOL && state[cell] & FLOODED == 0 {
                state[cell] |= FLOODED;
                queue.push_back(cell as u32);
            }
        }
        drop(line_cells);
        if has_polygon {
            for cell in 0..mw * mh {
                if state[cell] & RIVER == 0 || state[cell] & FLOODED != 0 || pool[cell] == NO_POOL {
                    continue;
                }
                wet[cell] = below(
                    &dem,
                    factor,
                    cell % mw,
                    cell / mw,
                    levels[usize::from(pool[cell])],
                );
                if wet[cell] > 0 {
                    state[cell] |= FLOODED;
                    queue.push_back(cell as u32);
                }
            }
        }
        while let Some(cell) = queue.pop_front() {
            let cell = cell as usize;
            let (i, j) = (cell % mw, cell / mw);
            let p = pool[cell];
            let mut visit = |n: usize| {
                if state[n] & FLOODED != 0 || pool[n] != p {
                    return;
                }
                if wet[n] == UNKNOWN {
                    wet[n] = below(&dem, factor, n % mw, n / mw, levels[usize::from(p)]);
                }
                if wet[n] > 0 {
                    state[n] |= FLOODED;
                    queue.push_back(n as u32);
                }
            };
            if i > 0 {
                visit(cell - 1);
            }
            if i + 1 < mw {
                visit(cell + 1);
            }
            if j > 0 {
                visit(cell - mw);
            }
            if j + 1 < mh {
                visit(cell + mw);
            }
        }

        // The flooded ground of each pool: its DEM cells below the level, in the cells reached.
        let mut flooded = vec![0.0f64; levels.len()];
        let mut outside = vec![0.0f64; levels.len()];
        let mut flooded_cells = 0;
        for j in 0..mh {
            let cell_m2 = cell_x((j as f64 + 0.5) * k) * cell_y;
            for i in 0..mw {
                let cell = j * mw + i;
                if state[cell] & FLOODED == 0 {
                    continue;
                }
                flooded_cells += 1;
                let p = usize::from(pool[cell]);
                if wet[cell] == UNKNOWN {
                    wet[cell] = below(&dem, factor, i, j, levels[p]);
                }
                let area = f64::from(wet[cell]) * cell_m2;
                flooded[p] += area;
                if state[cell] & RIVER == 0 {
                    outside[p] += area;
                }
            }
        }
        let areas = input
            .pools
            .iter()
            .enumerate()
            .map(|(i, p)| PoolArea {
                number: p.0,
                level_m: p.3,
                flooded_ha: flooded[i] / 10_000.0,
                outside_river_ha: has_polygon.then_some(outside[i] / 10_000.0),
            })
            .collect();
        Ok(PoolWater {
            dem,
            factor,
            width: mw,
            stats: MaskStats {
                factor,
                width: mw,
                height: mh,
                cell_m: sx,
                bytes: mw * mh * 3,
                peak_bytes,
                flooded_cells,
            },
            pool,
            state,
            levels,
            numbers: input.pools.iter().map(|p| p.0).collect(),
            areas,
            depth: Cache::default(),
            surface: Cache::default(),
            blank: Arc::new(encode_rgba(&vec![0u8; TILE * TILE * 4])?),
        })
    }

    /// The flooded ground of each pool, in the pools' order.
    pub fn areas(&self) -> &[PoolArea] {
        &self.areas
    }

    pub fn stats(&self) -> &MaskStats {
        &self.stats
    }

    /// The DEM the mask was made for.
    pub fn dem(&self) -> &Arc<Dem> {
        &self.dem
    }

    /// The water at a point in the DEM's CRS: `(pool index, water level, ground)`. `None` on dry
    /// ground and off the DEM.
    fn water(&self, px: f64, py: f64) -> Option<(usize, f64, f64)> {
        let [a, _, c, _, e, f] = self.dem.geotransform;
        let (col, row) = ((px - c) / a, (py - f) / e);
        if !(col >= 0.0
            && row >= 0.0
            && col < self.dem.width as f64
            && row < self.dem.height as f64)
        {
            return None;
        }
        let cell = (row as usize / self.factor) * self.width + col as usize / self.factor;
        if self.state[cell] & FLOODED == 0 {
            return None;
        }
        let p = usize::from(self.pool[cell]);
        let level = *self.levels.get(p)?;
        let ground = self.dem.sample_bilinear(px, py);
        (ground < level).then_some((p, level, ground))
    }

    /// What a click reads at a longitude and latitude in degrees. `None` on dry ground.
    pub fn at(&self, maker: &TileMaker, lon: f64, lat: f64) -> Option<WaterAt> {
        let [px, py] = maker.to_dem([lon, lat])?;
        let (p, level, ground) = self.water(px, py)?;
        Some(WaterAt {
            pool: self.numbers[p],
            water_level_m: level,
            ground_m: ground,
            depth_m: level - ground,
        })
    }

    /// The depth of the water at each pixel centre of tile `z/x/y`, metres, row by row from the
    /// north-west pixel. NaN on dry ground.
    pub fn depths(
        &self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Vec<f32>, DesignError> {
        check(z, x, y)?;
        Ok(maker.sampled(z, x, y, 0, f32::NAN, |px, py| {
            self.water(px, py)
                .map_or(f32::NAN, |(_, level, ground)| (level - ground) as f32)
        }))
    }

    /// The surface of tile `z/x/y`, metres: the pool's level where there is water, the ground
    /// (as [`TileMaker::heights`] gives it) everywhere else. Water drawn on this surface is flat
    /// at each pool's level.
    pub fn surface(
        &self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Vec<f32>, DesignError> {
        if !maker.holds_dem(z, x, y) {
            return maker.heights(z, x, y);
        }
        Ok(
            maker.sampled(z, x, y, 0, maker.info().base_m as f32, |px, py| {
                self.water(px, py)
                    .map_or_else(|| maker.height_at(px, py), |(_, level, _)| level)
                    as f32
            }),
        )
    }

    /// The water of tile `z/x/y` as a PNG with transparency, coloured by depth
    /// ([`DEPTH_STOPS`]) and clear on dry ground. From the cache when it is there.
    pub fn depth_tile(
        &mut self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, DesignError> {
        check(z, x, y)?;
        if !maker.holds_dem(z, x, y) {
            return Ok(self.blank.clone());
        }
        if let Some(hit) = self.depth.get((z, x, y)) {
            return Ok(hit);
        }
        let depths = self.depths(maker, z, x, y)?;
        let tile = if depths.iter().all(|d| d.is_nan()) {
            self.blank.clone()
        } else {
            let mut px = vec![0u8; TILE * TILE * 4];
            for (d, out) in depths.iter().zip(px.as_chunks_mut::<4>().0) {
                if !d.is_nan() {
                    out.copy_from_slice(&depth_colour(f64::from(*d)));
                }
            }
            Arc::new(encode_rgba(&px)?)
        };
        self.depth.put((z, x, y), tile.clone(), CACHE_TILES);
        Ok(tile)
    }

    /// [`PoolWater::surface`] of tile `z/x/y` as a Terrarium PNG, for the map's 3D view while
    /// the pool water is shown. From the cache when it is there.
    pub fn surface_tile(
        &mut self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
    ) -> Result<Arc<Vec<u8>>, DesignError> {
        check(z, x, y)?;
        if let Some(hit) = self.surface.get((z, x, y)) {
            return Ok(hit);
        }
        let tile = Arc::new(encode_png(&self.surface(maker, z, x, y)?)?);
        self.surface.put((z, x, y), tile.clone(), CACHE_TILES);
        Ok(tile)
    }
}

fn input_error() -> DesignError {
    refuse("the design has no pools to flood")
}

/// A tile of colours as a PNG (8-bit RGBA).
fn encode_rgba(px: &[u8]) -> Result<Vec<u8>, DesignError> {
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, TILE as u32, TILE as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| refuse(e.to_string()))?;
        writer
            .write_image_data(px)
            .map_err(|e| refuse(e.to_string()))?;
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    //! Label: property
    //!
    //! A straight valley in UTM 30N, 5 m cells: the ground is `BED + SLOPE * distance from the
    //! axis`, so a pool at level `L` floods `(L - BED) / SLOPE` metres to each side. The design
    //! line runs along the axis for 2000 m, with a pool at 8 m to 1000 m and one at 10 m beyond.
    use super::super::crs::Crs;
    use super::super::terrain_tile::{decode_png, pixel_lon_lat};
    use super::*;

    const X0: f64 = 500_000.0;
    const Y0: f64 = 600_000.0;
    const CELL: f64 = 5.0;
    const BED: f64 = 5.0;
    const SLOPE: f64 = 0.05;
    /// The DEM runs 200 m beyond each end of the line, and 500 m to each side of the axis.
    const MARGIN: f64 = 200.0;
    const HALF: f64 = 500.0;

    fn ground(x: f64, y: f64) -> f64 {
        // A hollow 300 m north of the axis, well below both pools, with high ground around it.
        if libm::hypot(x - (X0 + 500.0), y - (Y0 + 300.0)) < 30.0 {
            return 2.0;
        }
        BED + SLOPE * (y - Y0).abs()
    }

    fn valley() -> Arc<Dem> {
        let (w, h) = (
            ((2000.0 + 2.0 * MARGIN) / CELL) as usize,
            (2.0 * HALF / CELL) as usize,
        );
        let mut data = Vec::with_capacity(w * h);
        for r in 0..h {
            for c in 0..w {
                data.push(ground(
                    X0 - MARGIN + (c as f64 + 0.5) * CELL,
                    Y0 + HALF - (r as f64 + 0.5) * CELL,
                ));
            }
        }
        Arc::new(Dem {
            width: w,
            height: h,
            data,
            geotransform: [CELL, 0.0, X0 - MARGIN, 0.0, -CELL, Y0 + HALF],
            nodata: Some(-9999.0),
            epsg: Some(32630),
        })
    }

    fn lon_lat(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let Crs::Utm(u) = Crs::from_epsg(32630).unwrap() else {
            unreachable!()
        };
        u.inverse_all(pts).unwrap()
    }

    fn pools(polygon: bool) -> PoolWaterIn {
        let line: Vec<[f64; 2]> = (0..=20).map(|i| [X0 + 100.0 * f64::from(i), Y0]).collect();
        let strip = [
            [X0, Y0 - 20.0],
            [X0 + 2000.0, Y0 - 20.0],
            [X0 + 2000.0, Y0 + 20.0],
            [X0, Y0 + 20.0],
            [X0, Y0 - 20.0],
        ];
        PoolWaterIn {
            line: lon_lat(&line),
            chainage_m: (0..=20).map(|i| 100.0 * f64::from(i)).collect(),
            pools: vec![(1, 0.0, 1000.0, 8.0), (2, 1000.0, 2000.0, 10.0)],
            polygon: if polygon {
                vec![lon_lat(&strip)]
            } else {
                Vec::new()
            },
        }
    }

    fn at(w: &PoolWater, maker: &TileMaker, x: f64, y: f64) -> Option<WaterAt> {
        let ll = lon_lat(&[[X0 + x, Y0 + y]])[0];
        w.at(maker, ll[0], ll[1])
    }

    #[test]
    fn the_flooded_width_of_a_valley_matches_the_pool_level() {
        let maker = TileMaker::new(valley()).unwrap();
        let w = PoolWater::build(&maker, &pools(false)).unwrap();
        assert_eq!(
            w.stats().factor,
            1,
            "a small DEM has a mask cell for each DEM cell"
        );
        // Pool 1 at 8 m: 3 m over the bed, 60 m to each side. Pool 2 at 10 m: 100 m.
        for (x, half, level, pool) in [(500.0, 60.0, 8.0, 1), (1500.0, 100.0, 10.0, 2)] {
            for side in [-1.0, 1.0] {
                let wet = at(&w, &maker, x, side * (half - 6.0)).expect("inside the water's edge");
                assert_eq!(wet.pool, pool);
                assert_eq!(wet.water_level_m, level);
                let ground = BED + SLOPE * (half - 6.0);
                assert!((wet.ground_m - ground).abs() < 0.01, "{wet:?}");
                assert!((wet.depth_m - (level - ground)).abs() < 0.01, "{wet:?}");
                assert_eq!(
                    at(&w, &maker, x, side * (half + 6.0)),
                    None,
                    "beyond the edge"
                );
            }
            let mid = at(&w, &maker, x, 0.0).unwrap();
            assert!(
                (mid.depth_m - (level - BED)).abs() < 0.2,
                "deepest on the axis: {mid:?}"
            );
        }
        // The areas: 1000 m of valley, 120 m and 200 m wide.
        let a = w.areas();
        assert_eq!((a[0].number, a[1].number), (1, 2));
        assert!((a[0].flooded_ha - 12.0).abs() < 0.25, "{a:?}");
        assert!((a[1].flooded_ha - 20.0).abs() < 0.25, "{a:?}");
        assert_eq!(
            a[0].outside_river_ha, None,
            "no river polygon, no land figure"
        );
    }

    #[test]
    fn a_hollow_not_joined_to_the_river_stays_dry() {
        let maker = TileMaker::new(valley()).unwrap();
        let w = PoolWater::build(&maker, &pools(false)).unwrap();
        // The hollow's ground is 2 m, under the pool at 8 m, but the ground between is dry.
        assert!(ground(X0 + 500.0, Y0 + 300.0) < 8.0);
        assert_eq!(at(&w, &maker, 500.0, 300.0), None);
        assert_eq!(at(&w, &maker, 500.0, 200.0), None, "the ground between");
        // A coarse mask keeps it dry too, and reads the same water from the DEM's own cells.
        let coarse = PoolWater::with_budget(&maker, &pools(false), 3000).unwrap();
        assert!(coarse.stats().factor >= 5, "{:?}", coarse.stats());
        assert_eq!(at(&coarse, &maker, 500.0, 300.0), None);
        assert!(at(&coarse, &maker, 500.0, 54.0).is_some());
        assert_eq!(at(&coarse, &maker, 500.0, 66.0), None);
        for (fine, rough) in w.areas().iter().zip(coarse.areas()) {
            // The same but for the mask cells the line's ends and the weir's line pass through:
            // here a 30 m mask cell at the start, 120 m of water wide.
            assert!(
                (fine.flooded_ha - rough.flooded_ha).abs() < 0.4,
                "{fine:?} {rough:?}"
            );
        }
    }

    #[test]
    fn each_side_of_a_weir_takes_its_own_pool_level_and_nothing_floods_beyond_the_line_ends() {
        let maker = TileMaker::new(valley()).unwrap();
        let w = PoolWater::build(&maker, &pools(false)).unwrap();
        // The weir is at 1000 m: the line across the river there parts the two levels.
        let below = at(&w, &maker, 990.0, 0.0).unwrap();
        let above = at(&w, &maker, 1010.0, 0.0).unwrap();
        assert_eq!((below.pool, below.water_level_m), (1, 8.0));
        assert_eq!((above.pool, above.water_level_m), (2, 10.0));
        // 80 m from the axis the upper pool is 1 m deep and the lower pool's side is dry: the
        // upper pool's water does not pass the line.
        assert_eq!(at(&w, &maker, 990.0, 80.0), None);
        let flank = at(&w, &maker, 1010.0, 80.0).unwrap();
        assert!((flank.depth_m - 1.0).abs() < 0.01, "{flank:?}");
        // The valley goes on beyond both ends of the design line; the model does not.
        assert!(ground(X0 - 100.0, Y0) < 8.0);
        assert_eq!(at(&w, &maker, -100.0, 0.0), None);
        assert_eq!(at(&w, &maker, 2100.0, 0.0), None);
        assert!(at(&w, &maker, 10.0, 0.0).is_some());
        assert!(at(&w, &maker, 1990.0, 0.0).is_some());
    }

    #[test]
    fn the_land_figure_is_the_flooded_ground_outside_the_river_polygon_in_hectares() {
        let maker = TileMaker::new(valley()).unwrap();
        let w = PoolWater::build(&maker, &pools(true)).unwrap();
        // The river polygon is a strip 40 m wide: 4 ha of each pool's 1000 m.
        let a = w.areas();
        assert!((a[0].flooded_ha - 12.0).abs() < 0.25, "{a:?}");
        assert!((a[0].outside_river_ha.unwrap() - 8.0).abs() < 0.25, "{a:?}");
        assert!(
            (a[1].outside_river_ha.unwrap() - 16.0).abs() < 0.25,
            "{a:?}"
        );
    }

    #[test]
    fn a_tile_is_water_coloured_by_depth_and_its_surface_is_flat_at_the_pool_level() {
        let maker = TileMaker::new(valley()).unwrap();
        let mut w = PoolWater::build(&maker, &pools(false)).unwrap();
        // The tile at zoom 14 that holds the middle of pool 2.
        let ll = lon_lat(&[[X0 + 1500.0, Y0]])[0];
        let n = f64::from(1u32 << 14);
        let x = ((ll[0] + 180.0) / 360.0 * n) as u32;
        let lat = ll[1].to_radians();
        let y = ((1.0 - libm::log(libm::tan(lat) + 1.0 / libm::cos(lat)) / core::f64::consts::PI)
            / 2.0
            * n) as u32;
        let depths = w.depths(&maker, 14, x, y).unwrap();
        let surface = w.surface(&maker, 14, x, y).unwrap();
        let ground = maker.heights(14, x, y).unwrap();
        let (mut wet, mut dry) = (0, 0);
        for i in 0..TILE * TILE {
            if depths[i].is_nan() {
                dry += 1;
                assert_eq!(surface[i], ground[i], "dry ground is the ground");
                continue;
            }
            wet += 1;
            let p = pixel_lon_lat(14, x, y, i % TILE, i / TILE);
            let level = w.at(&maker, p[0], p[1]).unwrap().water_level_m;
            assert!(level == 8.0 || level == 10.0);
            assert_eq!(
                f64::from(surface[i]),
                level,
                "the water is flat at its pool's level"
            );
            assert!((f64::from(depths[i]) - (level - f64::from(ground[i]))).abs() < 0.01);
        }
        assert!(wet > 500 && dry > 500, "{wet} wet, {dry} dry");
        // The picture: clear where dry, and darker where deeper.
        let png = w.depth_tile(&maker, 14, x, y).unwrap();
        assert!(
            Arc::ptr_eq(&png, &w.depth_tile(&maker, 14, x, y).unwrap()),
            "from the cache"
        );
        let mut reader = png::Decoder::new(std::io::Cursor::new(&png[..]))
            .read_info()
            .unwrap();
        let mut px = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut px).unwrap();
        for (i, c) in px.as_chunks::<4>().0.iter().enumerate() {
            if depths[i].is_nan() {
                assert_eq!(c[3], 0);
            } else {
                assert_eq!(*c, depth_colour(f64::from(depths[i])));
            }
        }
        assert!(
            depth_colour(0.0)[2] > depth_colour(10.0)[2]
                && depth_colour(0.0)[3] < depth_colour(10.0)[3]
        );
        assert_eq!(depth_colour(50.0), depth_colour(10.0));
        // The 3D surface is a terrain tile of the same heights.
        let heights = decode_png(&w.surface_tile(&maker, 14, x, y).unwrap()).unwrap();
        for i in 0..TILE * TILE {
            assert!((heights[i] - f64::from(surface[i])).abs() < 0.005);
        }
        // A tile that holds none of the DEM: no water, and the plain terrain.
        let far = w.depth_tile(&maker, 14, 0, 0).unwrap();
        assert!(Arc::ptr_eq(&far, &w.blank));
        assert_eq!(
            w.surface(&maker, 14, 0, 0).unwrap(),
            maker.heights(14, 0, 0).unwrap()
        );
        assert!(w.depth_tile(&maker, 30, 0, 0).is_err());
    }

    #[test]
    fn a_dem_in_degrees_floods_the_same_width_in_metres() {
        // The same valley on a grid of 0.00005 degrees (about 5.5 m), along a parallel near 5 N.
        let (w_px, h_px, d) = (480usize, 200usize, 0.00005);
        let (lon0, lat0) = (-3.0, 5.0 + d * 100.0);
        let m_per_deg = R.to_radians();
        let mut data = Vec::new();
        for r in 0..h_px {
            let north = (100.0 - (r as f64 + 0.5)) * d * m_per_deg;
            data.extend(core::iter::repeat_n(BED + SLOPE * north.abs(), w_px));
        }
        let dem = Arc::new(Dem {
            width: w_px,
            height: h_px,
            data,
            geotransform: [d, 0.0, lon0, 0.0, -d, lat0],
            nodata: None,
            epsg: Some(4326),
        });
        let maker = TileMaker::new(dem).unwrap();
        let line = vec![[lon0 + d * 40.0, 5.0], [lon0 + d * 440.0, 5.0]];
        let length = d * 400.0 * m_per_deg * libm::cos(5f64.to_radians());
        let input = PoolWaterIn {
            line,
            chainage_m: vec![0.0, length],
            pools: vec![(1, 0.0, length, 8.0)],
            polygon: Vec::new(),
        };
        let w = PoolWater::build(&maker, &input).unwrap();
        let wet = |north_m: f64| w.at(&maker, lon0 + d * 240.0, 5.0 + north_m / m_per_deg);
        assert!(wet(50.0).is_some() && wet(-50.0).is_some());
        assert_eq!(wet(70.0), None);
        let ha = length * 120.0 / 10_000.0;
        assert!(
            (w.areas()[0].flooded_ha - ha).abs() < 0.06 * ha,
            "{:?} against {ha}",
            w.areas()
        );
        // Beyond the ends of the line the valley is dry.
        assert_eq!(w.at(&maker, lon0 + d * 20.0, 5.0), None);
    }

    #[test]
    fn a_design_with_no_pools_or_a_line_off_the_dem_is_refused_plainly() {
        let maker = TileMaker::new(valley()).unwrap();
        let mut none = pools(false);
        none.pools.clear();
        let e = PoolWater::build(&maker, &none).unwrap_err().to_string();
        assert!(e.contains("no pools"), "{e}");
        let mut off = pools(false);
        off.line = lon_lat(&[[X0 + 50_000.0, Y0], [X0 + 52_000.0, Y0]]);
        off.chainage_m = vec![0.0, 2000.0];
        let e = PoolWater::build(&maker, &off).unwrap_err().to_string();
        assert!(e.contains("does not cross the DEM"), "{e}");
    }

    /// The speed and the memory on a large DEM (MAP-4): run with
    /// `cargo test --release -p owd-format --lib pool_water::tests::speed -- --ignored --nocapture`.
    /// `OWD_POOL_DEM=15000x60000` sets the DEM's size in cells (the default is 4000x4000).
    #[test]
    #[ignore = "a measurement, not a check"]
    fn speed_on_a_large_dem_with_ten_pools() {
        let size = std::env::var("OWD_POOL_DEM").unwrap_or_else(|_| "4000x4000".into());
        let (w_px, h_px) = size.split_once('x').unwrap();
        let (w_px, h_px): (usize, usize) = (w_px.parse().unwrap(), h_px.parse().unwrap());
        let cell = 10.0;
        // A valley that winds from south to north, rising 3 m for each of ten pools.
        let (len, wide) = (h_px as f64 * cell, w_px as f64 * cell);
        let axis = |y: f64| wide / 2.0 + wide * 0.2 * libm::sin(y / len * 12.0);
        let t0 = std::time::Instant::now();
        let mut data = vec![0.0f64; w_px * h_px];
        for r in 0..h_px {
            let y = len - (r as f64 + 0.5) * cell;
            let (mid, bed) = (axis(y), 30.0 * y / len);
            for (c, v) in data[r * w_px..(r + 1) * w_px].iter_mut().enumerate() {
                let x = (c as f64 + 0.5) * cell;
                *v = bed + 0.01 * (x - mid).abs() + 0.3 * libm::sin(x / 37.0) * libm::cos(y / 53.0);
            }
        }
        let dem = Arc::new(Dem {
            width: w_px,
            height: h_px,
            data,
            geotransform: [cell, 0.0, X0 - wide / 2.0, 0.0, -cell, 100_000.0 + len],
            nodata: None,
            epsg: Some(32630),
        });
        println!(
            "DEM {w_px} x {h_px} cells made in {:.1} s",
            t0.elapsed().as_secs_f64()
        );
        let maker = TileMaker::new(dem).unwrap();
        let n = 2000;
        let line: Vec<[f64; 2]> = (0..=n)
            .map(|i| {
                let y = len * (0.02 + 0.96 * f64::from(i) / f64::from(n));
                [X0 - wide / 2.0 + axis(y), 100_000.0 + y]
            })
            .collect();
        let mut chainage_m = vec![0.0];
        for p in line.windows(2) {
            chainage_m.push(
                chainage_m[chainage_m.len() - 1]
                    + libm::hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]),
            );
        }
        let total = chainage_m[n as usize];
        let input = PoolWaterIn {
            line: lon_lat(&line),
            chainage_m,
            pools: (0..10)
                .map(|i| {
                    (
                        i + 1,
                        total * i as f64 / 10.0,
                        total * (i + 1) as f64 / 10.0,
                        5.0 + 3.0 * (i + 1) as f64,
                    )
                })
                .collect(),
            polygon: Vec::new(),
        };
        let t = std::time::Instant::now();
        let mut w = PoolWater::build(&maker, &input).unwrap();
        println!(
            "flood mask: {:.0} ms, {:?}",
            t.elapsed().as_secs_f64() * 1000.0,
            w.stats()
        );
        println!(
            "hectares: {:?}",
            w.areas()
                .iter()
                .map(|a| a.flooded_ha.round())
                .collect::<Vec<_>>()
        );
        // Tiles along the line at three zoom levels.
        for z in [10u8, 12, 14] {
            let scale = f64::from(1u32 << z);
            let mut tiles: Vec<(u32, u32)> = input
                .line
                .iter()
                .map(|p| {
                    let lat = p[1].to_radians();
                    (
                        ((p[0] + 180.0) / 360.0 * scale) as u32,
                        ((1.0
                            - libm::log(libm::tan(lat) + 1.0 / libm::cos(lat))
                                / core::f64::consts::PI)
                            / 2.0
                            * scale) as u32,
                    )
                })
                .collect();
            tiles.dedup();
            tiles.truncate(200);
            let t = std::time::Instant::now();
            for &(x, y) in &tiles {
                w.depth.tiles.clear();
                w.depth_tile(&maker, z, x, y).unwrap();
            }
            let depth = t.elapsed().as_secs_f64() * 1000.0 / tiles.len() as f64;
            let t = std::time::Instant::now();
            for &(x, y) in &tiles {
                w.surface.tiles.clear();
                w.surface_tile(&maker, z, x, y).unwrap();
            }
            let surface = t.elapsed().as_secs_f64() * 1000.0 / tiles.len() as f64;
            println!(
                "zoom {z}: {} tiles, {depth:.2} ms a water tile, {surface:.2} ms a surface tile",
                tiles.len()
            );
        }
    }
}
