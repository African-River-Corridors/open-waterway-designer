//! Contour lines of the design's DEM for the map (MAP-3), made on demand, one map tile at a time,
//! in memory. The heights are the DEM's own on the grid the terrain tiles use
//! ([`TileMaker::ground`]): the lines are traced through that grid by marching squares, joined
//! into whole lines, lightly smoothed, cut to the tile and written as a Mapbox Vector Tile, the
//! form a `vector` map source reads.
//!
//! A line is drawn only where the ground is known: a grid square with a corner off the DEM or in
//! a nodata cell has no line. The grid reaches [`MARGIN`] pixels past the tile on every side, so
//! the tile next door traces and smooths the same line across the shared edge and the two meet.
//! Every level is a whole multiple of the interval; every [`INDEX_EVERY`]th is an index contour.

use super::session::DesignError;
use super::terrain_tile::{TILE, TerrainState, TileMaker};
use std::collections::HashMap;
use std::sync::Arc;

/// The pixels of ground read past the tile's edge on every side. One is enough to trace a line
/// across the edge; the second is so the smoothing, which looks one point along the line, sees
/// the same points from both tiles.
pub const MARGIN: usize = 2;
/// Every fifth contour is an index contour: drawn heavier, and labelled with its level.
pub const INDEX_EVERY: i64 = 5;
/// The side of the vector tile, in its own units: 16 to a grid pixel.
pub const EXTENT: u32 = 4096;
/// How far past the tile's edge a line is kept, in grid pixels, so a line is drawn whole at the edge.
const BUFFER: f64 = 0.5;
/// A point of a line is dropped when the line stays within this of it, in grid pixels.
const SIMPLIFY: f64 = 0.04;
/// The smallest interval, in metres.
pub const MIN_INTERVAL_M: f64 = 0.1;
/// The most line pieces traced in one tile. A tile that would hold more (a fine interval on very
/// steep ground, far zoomed out) keeps the lowest levels only: a guard on memory, not a view.
const MAX_PIECES: usize = 600_000;
/// The contour tiles kept in memory.
pub const CACHE_TILES: usize = 128;
/// The name of the layer in the vector tile.
pub const LAYER: &str = "contours";

/// One level's lines in a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourLine {
    /// The level, in metres: a whole multiple of the interval.
    pub level_m: f64,
    /// An index contour: every [`INDEX_EVERY`]th level, counted from zero.
    pub index: bool,
    /// The lines, each a run of points in tile pixels: `[0, 0]` is the tile's north-west corner
    /// and `[256, 256]` its south-east. A closed ring ends on the point it starts on.
    pub parts: Vec<Vec<[f64; 2]>>,
}

/// The contour lines of one tile, lowest level first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContourTile {
    pub lines: Vec<ContourLine>,
}

fn input(message: impl Into<String>) -> DesignError {
    DesignError::Input(message.into())
}

fn check_interval(interval_m: f64) -> Result<(), DesignError> {
    if !interval_m.is_finite() || interval_m < MIN_INTERVAL_M {
        return Err(input(format!(
            "a contour interval is {MIN_INTERVAL_M} m or more, not {interval_m}"
        )));
    }
    Ok(())
}

/// No line piece at a grid edge.
const NONE: u32 = u32::MAX;

/// The contour lines of a square grid of heights, `side` a side, row by row from the north-west.
/// The grid holds a tile and `margin` pixels around it; heights are at pixel centres, and a
/// height that is not a number is ground not known. The lines come back cut to the tile.
pub fn contour_grid(
    heights: &[f32],
    side: usize,
    margin: usize,
    interval_m: f64,
) -> Result<ContourTile, DesignError> {
    check_interval(interval_m)?;
    if side < 2 || heights.len() != side * side {
        return Err(input(
            "a contour grid is square, two heights a side or more",
        ));
    }
    let h = |i: usize, j: usize| f64::from(heights[j * side + i]);
    // A grid edge has a number: twice its north or west sample's place, plus one for an edge
    // that runs south. A line crosses an edge at one point for a level, so pieces that share an
    // edge number are joined there exactly.
    let across = |i: usize, j: usize| (2 * (j * side + i)) as u32;
    let down = |i: usize, j: usize| (2 * (j * side + i) + 1) as u32;
    // Every piece of line: its level, as a count of intervals, and the two edges it joins.
    let mut pieces: Vec<(i32, u32, u32)> = Vec::new();
    'grid: for j in 0..side - 1 {
        for i in 0..side - 1 {
            let (a, b, c, d) = (h(i, j), h(i + 1, j), h(i + 1, j + 1), h(i, j + 1));
            let (lo, hi) = (a.min(b).min(c).min(d), a.max(b).max(c).max(d));
            // NaN fails every comparison: a square with a corner not known has no line.
            if !(lo.is_finite() && hi.is_finite() && (a + b + c + d).is_finite()) || lo == hi {
                continue;
            }
            let (first, last) = (
                libm::floor(lo / interval_m) + 1.0,
                libm::floor(hi / interval_m),
            );
            if last < first || first.abs() > 1e9 || last.abs() > 1e9 {
                continue;
            }
            let (top, right, bottom, left) =
                (across(i, j), down(i + 1, j), across(i, j + 1), down(i, j));
            for k in first as i32..=last as i32 {
                let level = f64::from(k) * interval_m;
                let case = u8::from(a >= level)
                    | u8::from(b >= level) << 1
                    | u8::from(c >= level) << 2
                    | u8::from(d >= level) << 3;
                // A saddle (opposite corners high): the middle of the square says which pair joins.
                let high_middle = (a + b + c + d) / 4.0 >= level;
                let mut add = |e1: u32, e2: u32| pieces.push((k, e1, e2));
                match case {
                    1 | 14 => add(left, top),
                    2 | 13 => add(top, right),
                    3 | 12 => add(left, right),
                    4 | 11 => add(right, bottom),
                    6 | 9 => add(top, bottom),
                    7 | 8 => add(left, bottom),
                    5 | 10 if (case == 5) == high_middle => {
                        add(top, right);
                        add(bottom, left);
                    }
                    5 | 10 => {
                        add(left, top);
                        add(right, bottom);
                    }
                    _ => {}
                }
                if pieces.len() >= MAX_PIECES {
                    break 'grid;
                }
            }
        }
    }
    pieces.sort_unstable_by_key(|p| p.0);

    // Where a level crosses an edge, in grid samples from the north-west sample.
    let crossing = |level: f64, edge: u32| -> [f64; 2] {
        let at = (edge >> 1) as usize;
        let (i, j) = (at % side, at / side);
        let south = edge & 1 == 1;
        let (a, b) = if south {
            (h(i, j), h(i, j + 1))
        } else {
            (h(i, j), h(i + 1, j))
        };
        let t = if b == a {
            0.5
        } else {
            ((level - a) / (b - a)).clamp(0.0, 1.0)
        };
        if south {
            [i as f64, j as f64 + t]
        } else {
            [i as f64 + t, j as f64]
        }
    };
    // Grid samples to tile pixels: sample `margin` is the centre of the tile's first pixel.
    let shift = 0.5 - margin as f64;
    let (low, high) = (-BUFFER, TILE as f64 + BUFFER);

    // The two pieces at each edge, for the level being joined now (`stamp` says which).
    let mut at_edge: Vec<[u32; 2]> = vec![[NONE; 2]; 2 * side * side];
    let mut stamp: Vec<u32> = vec![0; 2 * side * side];
    let mut lines = Vec::new();
    let mut start = 0;
    let mut round = 0u32;
    while start < pieces.len() {
        let k = pieces[start].0;
        let end = start + pieces[start..].partition_point(|p| p.0 == k);
        let level = f64::from(k) * interval_m;
        round += 1;
        for (n, &(_, e1, e2)) in pieces[start..end].iter().enumerate() {
            for e in [e1 as usize, e2 as usize] {
                if stamp[e] != round {
                    stamp[e] = round;
                    at_edge[e] = [NONE; 2];
                }
                let slot = if at_edge[e][0] == NONE { 0 } else { 1 };
                at_edge[e][slot] = n as u32;
            }
        }
        let group = &pieces[start..end];
        let mut used = vec![false; group.len()];
        // Walk from piece `n`, leaving it by edge `from`'s other end, to the line's end.
        let walk = |n: usize, from: u32, used: &mut Vec<bool>| -> Vec<[f64; 2]> {
            let mut run = vec![crossing(level, from)];
            let (mut n, mut edge) = (n, from);
            loop {
                used[n] = true;
                let (_, e1, e2) = group[n];
                edge = if e1 == edge { e2 } else { e1 };
                run.push(crossing(level, edge));
                let [p, q] = at_edge[edge as usize];
                let next = if p as usize == n { q } else { p };
                if next == NONE || used[next as usize] {
                    return run;
                }
                n = next as usize;
            }
        };
        let mut parts = Vec::new();
        let mut keep = |run: Vec<[f64; 2]>| {
            let smooth = chaikin(&run);
            for part in clip(&smooth, shift, low, high) {
                let part = simplify(&part);
                if part.len() >= 2 {
                    parts.push(part);
                }
            }
        };
        // Lines with an end first (an edge with one piece), then what is left: closed rings.
        for n in 0..group.len() {
            let (_, e1, e2) = group[n];
            for e in [e1, e2] {
                if !used[n] && at_edge[e as usize][1] == NONE {
                    keep(walk(n, e, &mut used));
                }
            }
        }
        for n in 0..group.len() {
            if !used[n] {
                keep(walk(n, group[n].1, &mut used));
            }
        }
        if !parts.is_empty() {
            lines.push(ContourLine {
                level_m: level,
                index: i64::from(k).rem_euclid(INDEX_EVERY) == 0,
                parts,
            });
        }
        start = end;
    }
    Ok(ContourTile { lines })
}

/// One pass of corner cutting: each corner of the line is replaced by two points a quarter of the
/// way along the lines that meet there. The ends of an open line stay; a ring stays closed.
fn chaikin(run: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if run.len() < 3 {
        return run.to_vec();
    }
    let closed = run[0] == run[run.len() - 1];
    let mut out = Vec::with_capacity(run.len() * 2 + 2);
    if !closed {
        out.push(run[0]);
    }
    for w in run.windows(2) {
        let (p, q) = (w[0], w[1]);
        out.push([0.75 * p[0] + 0.25 * q[0], 0.75 * p[1] + 0.25 * q[1]]);
        out.push([0.25 * p[0] + 0.75 * q[0], 0.25 * p[1] + 0.75 * q[1]]);
    }
    if closed {
        out.push(out[0]);
    } else {
        out.push(run[run.len() - 1]);
    }
    out
}

/// The parts of a line inside the box `low..high` (both ways), moved by `shift` first.
fn clip(run: &[[f64; 2]], shift: f64, low: f64, high: f64) -> Vec<Vec<[f64; 2]>> {
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut open = false;
    for w in run.windows(2) {
        let p = [w[0][0] + shift, w[0][1] + shift];
        let q = [w[1][0] + shift, w[1][1] + shift];
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        // Liang-Barsky: the part of p..q inside the box is t0..t1 along it.
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut inside = true;
        for (d, from_low, to_high) in [(dx, p[0] - low, high - p[0]), (dy, p[1] - low, high - p[1])]
        {
            if d == 0.0 {
                inside &= from_low >= 0.0 && to_high >= 0.0;
            } else {
                let (ta, tb) = (-from_low / d, to_high / d);
                t0 = t0.max(ta.min(tb));
                t1 = t1.min(ta.max(tb));
            }
        }
        if !inside || t0 > t1 {
            open = false;
            continue;
        }
        let a = [p[0] + t0 * dx, p[1] + t0 * dy];
        let b = [p[0] + t1 * dx, p[1] + t1 * dy];
        if !open || t0 > 0.0 {
            parts.push(vec![a]);
        }
        if let Some(part) = parts.last_mut() {
            part.push(b);
        }
        open = t1 >= 1.0;
    }
    parts
}

/// The line without the points it passes within [`SIMPLIFY`] of (Douglas-Peucker). Its ends stay.
fn simplify(run: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if run.len() < 3 {
        return run.to_vec();
    }
    let mut keep = vec![false; run.len()];
    keep[0] = true;
    keep[run.len() - 1] = true;
    let mut todo = vec![(0usize, run.len() - 1)];
    while let Some((a, b)) = todo.pop() {
        if b <= a + 1 {
            continue;
        }
        let (p, q) = (run[a], run[b]);
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let len = libm::hypot(dx, dy);
        let (mut worst, mut at) = (0.0, a);
        for (n, r) in run.iter().enumerate().take(b).skip(a + 1) {
            let d = if len == 0.0 {
                libm::hypot(r[0] - p[0], r[1] - p[1])
            } else {
                ((r[0] - p[0]) * dy - (r[1] - p[1]) * dx).abs() / len
            };
            if d > worst {
                (worst, at) = (d, n);
            }
        }
        if worst > SIMPLIFY {
            keep[at] = true;
            todo.push((a, at));
            todo.push((at, b));
        }
    }
    run.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

/// The contour lines of tile `z/x/y` of the maker's DEM, at every `interval_m` metres.
pub fn contours(
    maker: &TileMaker,
    z: u8,
    x: u32,
    y: u32,
    interval_m: f64,
) -> Result<ContourTile, DesignError> {
    contours_in(maker, TerrainState::Before, z, x, y, interval_m)
}

/// [`contours`] of the ground in `state` (MAP-5): the contours of the After ground show the cut.
pub fn contours_in(
    maker: &TileMaker,
    state: TerrainState,
    z: u8,
    x: u32,
    y: u32,
    interval_m: f64,
) -> Result<ContourTile, DesignError> {
    check_interval(interval_m)?;
    let ground = maker.ground_in(state, z, x, y, MARGIN)?;
    if !maker.holds_dem(z, x, y) {
        return Ok(ContourTile::default());
    }
    contour_grid(&ground, TILE + 2 * MARGIN, MARGIN, interval_m)
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// A length-delimited field of a protocol buffer message.
fn field(out: &mut Vec<u8>, number: u32, bytes: &[u8]) {
    varint(out, u64::from(number << 3 | 2));
    varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn zigzag(v: i32) -> u64 {
    u64::from(((v << 1) ^ (v >> 31)) as u32)
}

impl ContourTile {
    /// The tile as a Mapbox Vector Tile (version 2): one layer, [`LAYER`], with one feature for
    /// each level, its lines as a `LineString` geometry, and the properties `level` (metres, a
    /// number) and `index` (true or false). A tile with no lines is no bytes at all.
    pub fn to_mvt(&self) -> Vec<u8> {
        if self.lines.is_empty() {
            return Vec::new();
        }
        let scale = f64::from(EXTENT) / TILE as f64;
        let mut layer = Vec::new();
        field(&mut layer, 1, LAYER.as_bytes());
        // Values: false, true, then each level. Keys: level, index.
        let mut values: Vec<Vec<u8>> = [false, true]
            .iter()
            .map(|b| vec![7 << 3, u8::from(*b)])
            .collect();
        for line in &self.lines {
            let mut geometry: Vec<u32> = Vec::new();
            let (mut cx, mut cy) = (0i32, 0i32);
            for part in &line.parts {
                let mut points: Vec<[i32; 2]> = part
                    .iter()
                    .map(|p| {
                        [
                            libm::round(p[0] * scale) as i32,
                            libm::round(p[1] * scale) as i32,
                        ]
                    })
                    .collect();
                points.dedup();
                if points.len() < 2 {
                    continue;
                }
                let mut delta = |p: [i32; 2], geometry: &mut Vec<u32>| {
                    geometry.push(zigzag(p[0] - cx) as u32);
                    geometry.push(zigzag(p[1] - cy) as u32);
                    (cx, cy) = (p[0], p[1]);
                };
                // MoveTo (command 1) once, then LineTo (command 2) for the rest.
                geometry.push(1 | 1 << 3);
                delta(points[0], &mut geometry);
                geometry.push(2 | ((points.len() - 1) as u32) << 3);
                for p in &points[1..] {
                    delta(*p, &mut geometry);
                }
            }
            if geometry.is_empty() {
                continue;
            }
            let mut value = vec![3 << 3 | 1];
            value.extend_from_slice(&line.level_m.to_le_bytes());
            let level_at = values.len() as u64;
            values.push(value);
            let mut feature = Vec::new();
            let mut tags = Vec::new();
            for v in [0, level_at, 1, u64::from(line.index)] {
                varint(&mut tags, v);
            }
            field(&mut feature, 2, &tags);
            feature.extend_from_slice(&[3 << 3, 2]);
            let mut packed = Vec::with_capacity(geometry.len() * 2);
            for g in geometry {
                varint(&mut packed, u64::from(g));
            }
            field(&mut feature, 4, &packed);
            field(&mut layer, 2, &feature);
        }
        for key in ["level", "index"] {
            field(&mut layer, 3, key.as_bytes());
        }
        for value in &values {
            field(&mut layer, 4, value);
        }
        layer.push(5 << 3);
        varint(&mut layer, u64::from(EXTENT));
        layer.push(15 << 3);
        layer.push(2);
        let mut tile = Vec::new();
        field(&mut tile, 3, &layer);
        tile
    }
}

/// A tile in the cache: when it was last asked for, and its bytes.
type Kept = (u64, Arc<Vec<u8>>);

/// The last contour tiles made, as vector tiles: at most `capacity` of them.
#[derive(Debug)]
pub struct ContourCache {
    /// By terrain state, tile and interval (in millimetres).
    cache: HashMap<(TerrainState, u8, u32, u32, u32), Kept>,
    clock: u64,
    capacity: usize,
}

impl Default for ContourCache {
    fn default() -> Self {
        ContourCache::with_capacity(CACHE_TILES)
    }
}

impl ContourCache {
    pub fn with_capacity(capacity: usize) -> ContourCache {
        ContourCache {
            cache: HashMap::new(),
            clock: 0,
            capacity: capacity.max(1),
        }
    }

    /// The number of tiles in the cache now.
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Forget every tile: the DEM they were made from is gone.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// The contours of tile `z/x/y` at `interval_m` as a vector tile, from the cache when it is
    /// there. The tiles must all be of one `maker`: [`ContourCache::clear`] before another.
    pub fn tile(
        &mut self,
        maker: &TileMaker,
        z: u8,
        x: u32,
        y: u32,
        interval_m: f64,
    ) -> Result<Arc<Vec<u8>>, DesignError> {
        self.tile_in(maker, TerrainState::Before, z, x, y, interval_m)
    }

    /// Forget the tiles of one terrain state: what is laid on the ground in it has changed.
    pub fn clear_state(&mut self, state: TerrainState) {
        self.cache.retain(|k, _| k.0 != state);
    }

    /// [`ContourCache::tile`] of the ground in `state`.
    pub fn tile_in(
        &mut self,
        maker: &TileMaker,
        state: TerrainState,
        z: u8,
        x: u32,
        y: u32,
        interval_m: f64,
    ) -> Result<Arc<Vec<u8>>, DesignError> {
        check_interval(interval_m)?;
        let key = (state, z, x, y, libm::round(interval_m * 1000.0) as u32);
        self.clock += 1;
        if let Some(hit) = self.cache.get_mut(&key) {
            hit.0 = self.clock;
            return Ok(hit.1.clone());
        }
        let bytes = Arc::new(contours_in(maker, state, z, x, y, interval_m)?.to_mvt());
        if self.cache.len() >= self.capacity
            && let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| *k)
        {
            self.cache.remove(&oldest);
        }
        self.cache.insert(key, (self.clock, bytes.clone()));
        Ok(bytes)
    }
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

    /// A grid of `f(x, y)`, with x and y in tile pixels (the centre of each grid pixel).
    fn grid(side: usize, margin: usize, f: impl Fn(f64, f64) -> f64) -> Vec<f32> {
        (0..side * side)
            .map(|n| {
                let (i, j) = (n % side, n / side);
                f(
                    i as f64 + 0.5 - margin as f64,
                    j as f64 + 0.5 - margin as f64,
                ) as f32
            })
            .collect()
    }

    const SIDE: usize = TILE + 2 * MARGIN;

    fn tile_of(f: impl Fn(f64, f64) -> f64, interval: f64) -> ContourTile {
        contour_grid(&grid(SIDE, MARGIN, f), SIDE, MARGIN, interval).unwrap()
    }

    /// A UTM DEM of `f(x, y)`: `n` by `n` cells of `cell` metres, south-west corner at
    /// (500000, 600000).
    fn utm_dem(n: usize, cell: f64, f: impl Fn(f64, f64) -> f64) -> Dem {
        let top = 600_000.0 + n as f64 * cell;
        Dem {
            width: n,
            height: n,
            data: (0..n * n)
                .map(|i| {
                    let (r, c) = (i / n, i % n);
                    f(
                        500_000.0 + (c as f64 + 0.5) * cell,
                        top - (r as f64 + 0.5) * cell,
                    )
                })
                .collect(),
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

    /// A point of tile `z/x/y`, given in tile pixels, in the DEM's CRS.
    fn to_utm(z: u8, x: u32, y: u32, p: [f64; 2]) -> [f64; 2] {
        let n = f64::from(1u32 << z);
        let lon = (f64::from(x) + p[0] / TILE as f64) / n * 360.0 - 180.0;
        let v = (f64::from(y) + p[1] / TILE as f64) / n;
        let lat = libm::atan(libm::sinh(core::f64::consts::PI * (1.0 - 2.0 * v))).to_degrees();
        ZONE.forward(lon, lat).unwrap()
    }

    #[test]
    fn a_plane_gives_straight_evenly_spaced_lines_at_the_right_levels() {
        // 10 m at the tile's west edge, rising 0.1 m a pixel to the east and 0.05 m to the south.
        let plane = |x: f64, y: f64| 10.0 + 0.1 * x + 0.05 * y;
        let tile = tile_of(plane, 2.0);
        // The tile holds heights 10 m to 48.4 m: the levels 10 (its corner) to 48, every 2 m.
        let levels: Vec<f64> = tile.lines.iter().map(|l| l.level_m).collect();
        let want: Vec<f64> = (5..=24).map(|k| f64::from(k) * 2.0).collect();
        assert_eq!(levels, want);
        let norm = libm::hypot(0.1, 0.05);
        let mut last: Option<f64> = None;
        for line in &tile.lines {
            assert_eq!(line.index, (line.level_m / 2.0) as i64 % 5 == 0, "{line:?}");
            assert_eq!(line.parts.len(), 1, "one line a level: {line:?}");
            let part = &line.parts[0];
            // Straight: a plane's contour is a line, so smoothing and thinning leave its two ends.
            assert_eq!(part.len(), 2, "a straight line is two points: {part:?}");
            for p in part {
                assert!(
                    (plane(p[0], p[1]) - line.level_m).abs() < 1e-4,
                    "{p:?} is at {} m, not {} m",
                    plane(p[0], p[1]),
                    line.level_m
                );
                assert!(
                    p.iter()
                        .all(|v| (-BUFFER..=TILE as f64 + BUFFER).contains(v))
                );
            }
            // Evenly spaced: each line is the same distance from the one below it.
            let off = (0.1 * part[0][0] + 0.05 * part[0][1]) / norm;
            if let Some(before) = last
                && line.level_m > 12.0
            {
                assert!(
                    (off - before - 2.0 / norm).abs() < 1e-3,
                    "{} m is {} px on",
                    line.level_m,
                    off - before
                );
            }
            last = Some(off);
        }
    }

    #[test]
    fn a_cone_gives_closed_rings_around_its_top() {
        // A hill 30 m high in the middle of the tile, falling 0.3 m a pixel.
        let cone = |x: f64, y: f64| 30.0 - 0.3 * libm::hypot(x - 128.0, y - 128.0);
        let tile = tile_of(cone, 5.0);
        let levels: Vec<f64> = tile.lines.iter().map(|l| l.level_m).collect();
        // The ring of 0 m has a radius of 100 px: inside the tile. Lower levels are cut by its edge.
        assert!(
            levels.contains(&0.0) && levels.contains(&25.0),
            "{levels:?}"
        );
        assert!(
            levels.iter().all(|l| (l / 5.0).fract() == 0.0),
            "{levels:?}"
        );
        for line in tile.lines.iter().filter(|l| l.level_m >= 0.0) {
            assert_eq!(line.parts.len(), 1, "one ring at {} m", line.level_m);
            let ring = &line.parts[0];
            assert_eq!(
                ring.first(),
                ring.last(),
                "the ring at {} m closes",
                line.level_m
            );
            assert!(ring.len() > 8);
            let radius = (30.0 - line.level_m) / 0.3;
            for p in ring {
                let r = libm::hypot(p[0] - 128.0, p[1] - 128.0);
                // Light smoothing: the ring stays within a third of a pixel of the true circle.
                assert!(
                    (r - radius).abs() < 0.35,
                    "{} m: radius {r} for {radius}",
                    line.level_m
                );
            }
            assert_eq!(line.index, line.level_m == 0.0 || line.level_m == 25.0);
        }
        // A level cut by the tile's edge is open lines, one for each corner it is cut at.
        let cut = tile.lines.iter().find(|l| l.level_m == -15.0).unwrap();
        assert_eq!(cut.parts.len(), 4);
        assert!(cut.parts.iter().all(|p| p.first() != p.last()));
    }

    #[test]
    fn the_levels_are_whole_multiples_of_the_interval_below_zero_too() {
        let slope = |x: f64, _y: f64| -7.3 + 0.07 * x;
        for interval in [1.0, 2.0, 5.0, 10.0, 20.0, 0.5] {
            let tile = tile_of(slope, interval);
            assert!(!tile.lines.is_empty() || interval > 10.0, "{interval}");
            let mut last = f64::NEG_INFINITY;
            for line in &tile.lines {
                let k = line.level_m / interval;
                assert!(
                    (k - libm::round(k)).abs() < 1e-9,
                    "{} at {interval}",
                    line.level_m
                );
                assert_eq!(line.index, (libm::round(k) as i64).rem_euclid(5) == 0);
                assert!(line.level_m > last, "lowest first");
                last = line.level_m;
                // Inside the heights of the grid.
                assert!(line.level_m > -7.3 - 0.2 && line.level_m < -7.3 + 0.07 * 258.0);
            }
        }
        // -5 m and 0 m, 5 m and 10 m: the index contours of a 1 m interval here.
        let ones = tile_of(slope, 1.0);
        let index: Vec<f64> = ones
            .lines
            .iter()
            .filter(|l| l.index)
            .map(|l| l.level_m)
            .collect();
        assert_eq!(index, [-5.0, 0.0, 5.0, 10.0]);
        assert!(contour_grid(&[0.0; 4], 2, 0, 0.0).is_err());
        assert!(contour_grid(&[0.0; 4], 2, 0, f64::NAN).is_err());
        assert!(contour_grid(&[0.0; 5], 2, 0, 1.0).is_err());
    }

    #[test]
    fn ground_that_is_not_known_has_no_lines() {
        // Nothing known at all.
        assert!(tile_of(|_, _| f64::NAN, 1.0).lines.is_empty());
        // Flat ground has no contour either, even at a level.
        assert!(tile_of(|_, _| 5.0, 1.0).lines.is_empty());
        // A hole in a slope: no line in the hole, nor in the squares that touch it.
        let hole = |x: f64, y: f64| {
            if (100.0..140.0).contains(&x) && (60.0..200.0).contains(&y) {
                f64::NAN
            } else {
                0.1 * x + 0.02 * y
            }
        };
        let tile = tile_of(hole, 1.0);
        assert!(tile.lines.len() > 20);
        let mut ends_at_the_hole = 0;
        for line in &tile.lines {
            for part in &line.parts {
                for p in part {
                    assert!(
                        !(p[0] > 99.5 && p[0] < 139.5 && p[1] > 59.5 && p[1] < 199.5),
                        "{} m has a point in the hole: {p:?}",
                        line.level_m
                    );
                    assert!(p[0].is_finite() && p[1].is_finite());
                }
                for end in [part[0], part[part.len() - 1]] {
                    let inside = |v: f64| v > 1.0 && v < TILE as f64 - 1.0;
                    ends_at_the_hole += usize::from(inside(end[0]) && inside(end[1]));
                }
            }
        }
        assert!(
            ends_at_the_hole > 4,
            "lines stop at the hole's rim ({ends_at_the_hole})"
        );
    }

    /// Where the lines of a level cross the line `x = at` (or `y = at`), in tile pixels.
    fn crossings(tile: &ContourTile, level: f64, axis: usize, at: f64) -> Vec<f64> {
        let mut out = Vec::new();
        for line in tile.lines.iter().filter(|l| l.level_m == level) {
            for part in &line.parts {
                for w in part.windows(2) {
                    let (a, b) = (w[0][axis] - at, w[1][axis] - at);
                    if (a < 0.0) != (b < 0.0) {
                        let t = a / (a - b);
                        out.push(w[0][1 - axis] + t * (w[1][1 - axis] - w[0][1 - axis]));
                    }
                }
            }
        }
        out.sort_by(f64::total_cmp);
        out
    }

    /// How far a point is from the lines of a level, in tile pixels.
    fn distance(tile: &ContourTile, level: f64, p: [f64; 2]) -> f64 {
        let mut best = f64::INFINITY;
        for line in tile.lines.iter().filter(|l| l.level_m == level) {
            for w in line.parts.iter().flat_map(|part| part.windows(2)) {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                let t = ((p[0] - w[0][0]) * dx + (p[1] - w[0][1]) * dy) / (dx * dx + dy * dy);
                let t = t.clamp(0.0, 1.0);
                best = best.min(libm::hypot(
                    p[0] - w[0][0] - t * dx,
                    p[1] - w[0][1] - t * dy,
                ));
            }
        }
        best
    }

    /// Rolling ground: valleys and ridges a few hundred metres across, 100 m to 140 m.
    fn rolling(x: f64, y: f64) -> f64 {
        120.0
            + 12.0 * libm::sin((x - 500_000.0) / 310.0)
            + 8.0 * libm::cos((y - 600_000.0) / 190.0 + (x - 500_000.0) / 800.0)
    }

    #[test]
    fn lines_meet_across_a_tile_edge() {
        let dem = Arc::new(utm_dem(800, 10.0, rolling));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        let [lon, lat] = ZONE.inverse((x0 + x1) / 2.0, (y0 + y1) / 2.0).unwrap();
        let mut met = 0;
        for z in [13u8, 14, 15] {
            let (x, y) = tile_at(z, lon, lat);
            let here = contours(&maker, z, x, y, 2.0).unwrap();
            let east = contours(&maker, z, x + 1, y, 2.0).unwrap();
            let south = contours(&maker, z, x, y + 1, 2.0).unwrap();
            assert!(here.lines.len() > 3, "z{z}: {} levels", here.lines.len());
            for (other, axis) in [(&east, 0), (&south, 1)] {
                // The shared edge is pixel 256 of this tile and pixel 0 of the next.
                for (from, at, to, shift) in [
                    (&here, TILE as f64, other, -(TILE as f64)),
                    (other, 0.0, &here, TILE as f64),
                ] {
                    for line in &from.lines {
                        for along in crossings(from, line.level_m, axis, at) {
                            let mut p = [along, along];
                            p[axis] = at + shift;
                            // Each tile thins its own line, to within SIMPLIFY of it: where one
                            // tile's line reaches the edge, the other's passes within a twentieth
                            // of a grid pixel (a tenth of a screen pixel).
                            let gap = distance(to, line.level_m, p);
                            assert!(
                                gap < SIMPLIFY + 0.01,
                                "z{z} {} m: the lines are {gap} px apart at the edge",
                                line.level_m
                            );
                            met += 1;
                        }
                    }
                }
            }
        }
        assert!(met > 20, "lines cross the shared edges ({met})");
    }

    #[test]
    fn a_line_lies_where_the_dem_has_its_level() {
        let dem = Arc::new(utm_dem(800, 10.0, rolling));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        let [lon, lat] = ZONE.inverse((x0 + x1) / 2.0, (y0 + y1) / 2.0).unwrap();
        let (x, y) = tile_at(15, lon, lat);
        let tile = contours(&maker, 15, x, y, 1.0).unwrap();
        let mut seen = 0;
        for line in &tile.lines {
            assert!(line.level_m >= 100.0 && line.level_m <= 140.0);
            assert_eq!(line.level_m.fract(), 0.0);
            for p in line.parts.iter().flatten() {
                let [px, py] = to_utm(15, x, y, *p);
                // Smoothing may move a point a quarter of a pixel (about 1.2 m of ground here)
                // on ground that falls at most about 0.06 m in a metre.
                assert!(
                    (rolling(px, py) - line.level_m).abs() < 0.12,
                    "{} m line at ground of {} m",
                    line.level_m,
                    rolling(px, py)
                );
                seen += 1;
            }
        }
        assert!(seen > 200, "{seen} points");
    }

    #[test]
    fn nodata_in_the_dem_and_ground_off_the_dem_have_no_lines() {
        // A slope with a nodata block in its middle.
        let mut dem = utm_dem(400, 10.0, |x, y| {
            50.0 + (x - 500_000.0) * 0.02 + (y - 600_000.0) * 0.003
        });
        for r in 150..250 {
            for c in 150..250 {
                dem.data[r * 400 + c] = -9999.0;
            }
        }
        let dem = Arc::new(dem);
        let maker = TileMaker::new(dem.clone()).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        let [lon, lat] = ZONE.inverse((x0 + x1) / 2.0, (y0 + y1) / 2.0).unwrap();
        let (x, y) = tile_at(13, lon, lat);
        let tile = contours(&maker, 13, x, y, 5.0).unwrap();
        let mut seen = 0;
        for line in &tile.lines {
            // Never the nodata value, nor a level between it and the ground.
            assert!(
                line.level_m >= 50.0 && line.level_m <= 142.0,
                "{}",
                line.level_m
            );
            for p in line.parts.iter().flatten() {
                let [px, py] = to_utm(13, x, y, *p);
                assert!(
                    px > x0 && px < x1 && py > y0 && py < y1,
                    "off the DEM: {px}, {py}"
                );
                let hole =
                    px > x0 + 1500.0 && px < x0 + 2500.0 && py > y0 + 1500.0 && py < y0 + 2500.0;
                assert!(!hole, "in the nodata block: {px}, {py}");
                seen += 1;
            }
        }
        assert!(seen > 10, "{seen} points on the ground that is known");
        // A tile that holds none of the DEM: nothing, and no bytes.
        assert!(!maker.holds_dem(13, x + 3, y));
        let far = contours(&maker, 13, x + 3, y, 5.0).unwrap();
        assert!(far.lines.is_empty());
        assert!(far.to_mvt().is_empty());
        assert!(
            contours(&maker, 13, 1 << 13, 0, 5.0).is_err(),
            "no such tile"
        );
        assert!(contours(&maker, 13, x, y, 0.0).is_err(), "no such interval");
    }

    /// A reader of the vector tile, for the test: `(level, index, parts)` of each feature, with
    /// the layer's name and extent.
    #[allow(clippy::type_complexity)]
    fn read_mvt(bytes: &[u8]) -> (String, u32, Vec<(f64, bool, Vec<Vec<[i32; 2]>>)>) {
        fn varint(b: &[u8], at: &mut usize) -> u64 {
            let (mut v, mut shift) = (0u64, 0);
            loop {
                let byte = b[*at];
                *at += 1;
                v |= u64::from(byte & 0x7f) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    return v;
                }
            }
        }
        /// The fields of a message: `(number, wire type, varint value, bytes)`.
        fn fields(b: &[u8]) -> Vec<(u64, u64, u64, &[u8])> {
            let mut at = 0;
            let mut out = Vec::new();
            while at < b.len() {
                let key = varint(b, &mut at);
                match key & 7 {
                    0 => out.push((key >> 3, 0, varint(b, &mut at), &b[0..0])),
                    1 => {
                        out.push((key >> 3, 1, 0, &b[at..at + 8]));
                        at += 8;
                    }
                    2 => {
                        let len = varint(b, &mut at) as usize;
                        out.push((key >> 3, 2, 0, &b[at..at + len]));
                        at += len;
                    }
                    other => panic!("wire type {other}"),
                }
            }
            out
        }
        let tile = fields(bytes);
        assert_eq!(tile.len(), 1, "one layer");
        assert_eq!(tile[0].0, 3);
        let layer = fields(tile[0].3);
        let name = layer
            .iter()
            .find(|f| f.0 == 1)
            .map(|f| String::from_utf8(f.3.to_vec()).unwrap())
            .unwrap();
        let extent = layer.iter().find(|f| f.0 == 5).unwrap().2 as u32;
        assert_eq!(layer.iter().find(|f| f.0 == 15).unwrap().2, 2, "version 2");
        let keys: Vec<&[u8]> = layer.iter().filter(|f| f.0 == 3).map(|f| f.3).collect();
        assert_eq!(keys, [b"level".as_slice(), b"index".as_slice()]);
        let values: Vec<Vec<(u64, u64, u64, &[u8])>> = layer
            .iter()
            .filter(|f| f.0 == 4)
            .map(|f| fields(f.3))
            .collect();
        let mut out = Vec::new();
        for f in layer.iter().filter(|f| f.0 == 2) {
            let feature = fields(f.3);
            assert_eq!(
                feature.iter().find(|f| f.0 == 3).unwrap().2,
                2,
                "a LineString"
            );
            let tags = feature.iter().find(|f| f.0 == 2).unwrap().3;
            let mut at = 0;
            let mut tag = Vec::new();
            while at < tags.len() {
                tag.push(varint(tags, &mut at) as usize);
            }
            assert_eq!((tag[0], tag[2]), (0, 1), "level, then index");
            let level = &values[tag[1]][0];
            assert_eq!((level.0, level.1), (3, 1), "a double");
            let level = f64::from_le_bytes(level.3.try_into().unwrap());
            let index = &values[tag[3]][0];
            assert_eq!(index.0, 7, "a bool");
            let geometry = feature.iter().find(|f| f.0 == 4).unwrap().3;
            let mut at = 0;
            let (mut x, mut y) = (0i32, 0i32);
            let mut parts: Vec<Vec<[i32; 2]>> = Vec::new();
            while at < geometry.len() {
                let command = varint(geometry, &mut at);
                for _ in 0..command >> 3 {
                    let unzig = |v: u64| ((v >> 1) as i32) ^ -((v & 1) as i32);
                    x += unzig(varint(geometry, &mut at));
                    y += unzig(varint(geometry, &mut at));
                    if command & 7 == 1 {
                        parts.push(Vec::new());
                    }
                    parts.last_mut().unwrap().push([x, y]);
                }
            }
            out.push((level, index.2 == 1, parts));
        }
        (name, extent, out)
    }

    #[test]
    fn the_vector_tile_holds_every_line_with_its_level_and_index() {
        let cone = |x: f64, y: f64| 31.0 - 0.3 * libm::hypot(x - 100.0, y - 150.0);
        let tile = tile_of(cone, 1.0);
        let bytes = tile.to_mvt();
        let (name, extent, features) = read_mvt(&bytes);
        assert_eq!(name, "contours");
        assert_eq!(extent, 4096);
        assert_eq!(features.len(), tile.lines.len());
        for ((level, index, parts), line) in features.iter().zip(&tile.lines) {
            assert_eq!(*level, line.level_m);
            assert_eq!(*index, line.index);
            assert_eq!(parts.len(), line.parts.len(), "{level} m");
            for (got, want) in parts.iter().zip(&line.parts) {
                // 16 units to a pixel: each end is within a 32nd of a pixel of the line's.
                for (g, w) in [
                    (got[0], want[0]),
                    (got[got.len() - 1], want[want.len() - 1]),
                ] {
                    assert!((f64::from(g[0]) / 16.0 - w[0]).abs() <= 1.0 / 32.0 + 1e-9);
                    assert!((f64::from(g[1]) / 16.0 - w[1]).abs() <= 1.0 / 32.0 + 1e-9);
                }
            }
        }
        assert!(features.iter().any(|f| f.1) && features.iter().any(|f| !f.1));
        assert!(ContourTile::default().to_mvt().is_empty());
    }

    #[test]
    fn the_cache_is_bounded_and_keyed_by_the_interval() {
        let dem = Arc::new(utm_dem(400, 10.0, rolling));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        let [lon, lat] = ZONE.inverse((x0 + x1) / 2.0, (y0 + y1) / 2.0).unwrap();
        let (x, y) = tile_at(14, lon, lat);
        let mut cache = ContourCache::with_capacity(3);
        let one = cache.tile(&maker, 14, x, y, 1.0).unwrap();
        let five = cache.tile(&maker, 14, x, y, 5.0).unwrap();
        assert!(
            one.len() > five.len() && !five.is_empty(),
            "{} and {}",
            one.len(),
            five.len()
        );
        assert_eq!(cache.cached(), 2);
        assert!(
            Arc::ptr_eq(&one, &cache.tile(&maker, 14, x, y, 1.0).unwrap()),
            "kept"
        );
        cache.tile(&maker, 14, x + 1, y, 1.0).unwrap();
        cache.tile(&maker, 14, x, y + 1, 1.0).unwrap();
        assert_eq!(cache.cached(), 3, "never more than its size");
        // The 5 m tile was the one asked for longest ago: it went, and is made the same again.
        let again = cache.tile(&maker, 14, x, y, 5.0).unwrap();
        assert!(!Arc::ptr_eq(&five, &again));
        assert_eq!(*five, *again);
        assert!(cache.tile(&maker, 14, x, y, 0.01).is_err());
        cache.clear();
        assert_eq!(cache.cached(), 0);
    }

    /// A synthetic river valley: a plain that falls 3 m over the DEM, a meandering channel 4 m
    /// deep, valley sides that rise 40 m, and small rough ground.
    fn valley(x: f64, y: f64) -> f64 {
        let (u, v) = (x - 500_000.0, y - 600_000.0);
        let mid = 12_000.0 + 2_500.0 * libm::sin(u / 3_000.0);
        let off = (v - mid).abs();
        let side = 40.0 / (1.0 + libm::exp(-(off - 5_000.0) / 600.0));
        let channel = -4.0 * libm::exp(-(off / 120.0) * (off / 120.0));
        let rough = 0.4 * libm::sin(u / 37.0) * libm::cos(v / 53.0);
        20.0 - u * 3.0 / 24_000.0 + side + channel + rough
    }

    /// The time to make a contour tile, on a 4000 by 4000 DEM of 6 m cells. Run with
    /// `cargo test --release -p owd-format contours -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing, not a check"]
    fn time_per_tile_on_a_4000_by_4000_dem() {
        let dem = Arc::new(utm_dem(4000, 6.0, valley));
        let maker = TileMaker::new(dem.clone()).unwrap();
        let (x0, y0, x1, y1) = dem.bounds();
        // Three places: the river on the plain, a valley side, and the two together.
        for (what, fx, fy) in [
            ("plain and river", 0.5, 0.5),
            ("valley side", 0.5, 0.29),
            ("river bend", 0.2, 0.6),
        ] {
            let [lon, lat] = ZONE
                .inverse(x0 + fx * (x1 - x0), y0 + fy * (y1 - y0))
                .unwrap();
            for z in [11u8, 12, 13, 14, 15] {
                let (x, y) = tile_at(z, lon, lat);
                for interval in [1.0, 5.0] {
                    let runs = 5;
                    let start = std::time::Instant::now();
                    for _ in 0..runs {
                        maker.ground(z, x, y, MARGIN).unwrap();
                    }
                    let ground = start.elapsed().as_secs_f64() * 1e3 / f64::from(runs);
                    let start = std::time::Instant::now();
                    let mut bytes = 0;
                    let mut points = 0;
                    let mut levels = 0;
                    for _ in 0..runs {
                        let tile = contours(&maker, z, x, y, interval).unwrap();
                        levels = tile.lines.len();
                        points = tile.lines.iter().flat_map(|l| &l.parts).map(Vec::len).sum();
                        bytes = tile.to_mvt().len();
                    }
                    let all = start.elapsed().as_secs_f64() * 1e3 / f64::from(runs);
                    println!(
                        "{what} z{z} at {interval} m: {all:.2} ms a tile ({ground:.2} ms of it reading the ground); {levels} levels, {points} points, {bytes} bytes"
                    );
                }
            }
        }
    }
}
