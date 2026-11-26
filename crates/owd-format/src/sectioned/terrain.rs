//! One terrain interface for every reader of the ground (COG-1, part 2).
//!
//! [`Terrain`] is what a stage, the map and the drawings ask the ground through. Two readers
//! implement it: [`Dem`], the whole raster in memory (small files, and every test that builds a DEM
//! by hand), and [`SectionedDem`], a tiled GeoTIFF read a block at a time through a [`ByteSource`]
//! with a bounded cache. A [`Dem`] has one level, its own; a [`SectionedDem`] has the full level and
//! every overview the file holds.
//!
//! Full resolution: [`Terrain::sample_bilinear`] and [`Terrain::sample_many`] always read level 0,
//! so the volumes, sections and any point read give the same number from either reader.
//! Coarser levels are read only where a caller says what cell size it needs
//! ([`Terrain::level_for_cell`]): the map's tiles at a zoom, the hillshade, and the grids of the
//! optimise stage and its flood field. A [`Dem`] answers level 0 to every such ask, so it reads
//! exactly as before.

use super::cache::{CacheStats, Lru, Weigh};
use super::layout::{Geo, Image, Layout, Samples, decode_block};
use super::source::SharedSource;
use crate::geotiff::{Dem, GeoTiffError};
use std::sync::{Arc, Mutex};

/// A level of a terrain: its size and its geotransform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub geotransform: [f64; 6],
}

impl Level {
    /// The side of a cell, in the CRS's units (the larger of the two).
    pub fn cell(&self) -> f64 {
        self.geotransform[0].abs().max(self.geotransform[4].abs())
    }
}

/// The lowest, highest and middle height of a terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeightSummary {
    pub min: f64,
    pub max: f64,
    pub median: f64,
}

/// The bilinear height at `(x, y)` on a grid with geotransform `gt`, `w` by `h` cells, `v(row, col)`
/// the height of a cell (NaN for nodata). The formula of [`Dem::sample_bilinear`], so both readers
/// give the same bits.
pub fn bilinear(
    gt: &[f64; 6],
    w: usize,
    h: usize,
    x: f64,
    y: f64,
    mut v: impl FnMut(usize, usize) -> f64,
) -> f64 {
    let [a, _, c, _, e, f] = *gt;
    let col = (x - c) / a - 0.5;
    let row = (y - f) / e - 0.5;
    let (c0, r0) = (col.floor(), row.floor());
    let ok = c0 >= 0.0 && r0 >= 0.0 && c0 < (w as f64 - 1.0) && r0 < (h as f64 - 1.0);
    if !ok {
        return f64::NAN;
    }
    let (fc, fr) = (col - c0, row - r0);
    let (cc, rr) = (c0 as usize, r0 as usize);
    v(rr, cc) * (1.0 - fc) * (1.0 - fr)
        + v(rr, cc + 1) * fc * (1.0 - fr)
        + v(rr + 1, cc) * (1.0 - fc) * fr
        + v(rr + 1, cc + 1) * fc * fr
}

/// The ground, however it is held.
pub trait Terrain: Send + Sync + core::fmt::Debug {
    /// The full-resolution level.
    fn full(&self) -> Level;
    fn nodata(&self) -> Option<f64>;
    fn epsg(&self) -> Option<u32>;

    fn width(&self) -> usize {
        self.full().width
    }
    fn height(&self) -> usize {
        self.full().height
    }
    fn geotransform(&self) -> [f64; 6] {
        self.full().geotransform
    }

    /// `(xmin, ymin, xmax, ymax)` of the raster's outer edges.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        let l = self.full();
        let [a, _, c, _, e, f] = l.geotransform;
        let (x1, y1) = (c + a * l.width as f64, f + e * l.height as f64);
        (c.min(x1), f.min(y1), c.max(x1), f.max(y1))
    }

    /// Every level, the full one first. One level unless the file holds overviews.
    fn levels(&self) -> Vec<Level> {
        vec![self.full()]
    }

    /// The coarsest level whose cell is no larger than `cell` (in the CRS's units): the level a
    /// grid of that cell size reads. 0 when `cell` is finer than the full level, or not finite.
    fn level_for_cell(&self, cell: f64) -> usize {
        let levels = self.levels();
        if !cell.is_finite() {
            return 0;
        }
        (0..levels.len())
            .rev()
            .find(|&l| levels[l].cell() <= cell * (1.0 + 1e-9))
            .unwrap_or(0)
    }

    /// The height of cell (`row`, `col`) of `level`, NaN for nodata or a block that could not be
    /// read.
    fn level_value(&self, level: usize, row: usize, col: usize) -> f64;

    /// The height of cell (`row`, `col`) at full resolution, NaN for nodata.
    fn value(&self, row: usize, col: usize) -> f64 {
        self.level_value(0, row, col)
    }

    /// The bilinear height at a world point, at full resolution (see [`Dem::sample_bilinear`]).
    fn sample_bilinear(&self, x: f64, y: f64) -> f64 {
        self.sample_bilinear_level(0, x, y)
    }

    /// The bilinear height at a world point on `level`.
    fn sample_bilinear_level(&self, level: usize, x: f64, y: f64) -> f64 {
        let l = self.levels()[level];
        bilinear(&l.geotransform, l.width, l.height, x, y, |r, c| {
            self.level_value(level, r, c)
        })
    }

    /// [`Terrain::sample_bilinear`] for many points (full resolution).
    fn sample_many(&self, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        self.sample_many_level(0, xs, ys)
    }

    /// Bilinear heights of many points on `level`.
    fn sample_many_level(&self, level: usize, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        xs.iter()
            .zip(ys)
            .map(|(&x, &y)| self.sample_bilinear_level(level, x, y))
            .collect()
    }

    /// How many full-resolution cells in rows `r0..r1` and columns `c0..c1` are below `level`
    /// (nodata never is).
    fn count_below(&self, r0: usize, r1: usize, c0: usize, c1: usize, level: f64) -> u32 {
        let mut n = 0u32;
        for r in r0..r1 {
            for c in c0..c1 {
                if self.value(r, c) < level {
                    n += 1;
                }
            }
        }
        n
    }

    /// The lowest, highest and middle height, or `None` when the terrain holds none.
    fn summary(&self) -> Option<HeightSummary>;

    /// The cells of `level` that cover `bounds` (`xmin, ymin, xmax, ymax`, in the CRS), as a DEM in
    /// memory with its own geotransform. Nodata stays the file's nodata value.
    fn window_level(&self, level: usize, bounds: (f64, f64, f64, f64)) -> Dem {
        let l = self.levels()[level];
        let [a, _, c, _, e, f] = l.geotransform;
        let span = |lo: f64, hi: f64, origin: f64, step: f64, n: usize| {
            let (i0, i1) = ((lo - origin) / step, (hi - origin) / step);
            let (i0, i1) = (i0.min(i1).floor(), i0.max(i1).ceil());
            let i0 = i0.clamp(0.0, n as f64) as usize;
            let i1 = i1.clamp(0.0, n as f64) as usize;
            (i0, i1.max(i0))
        };
        let (c0, c1) = span(bounds.0, bounds.2, c, a, l.width);
        let (r0, r1) = span(bounds.1, bounds.3, f, e, l.height);
        let nodata = self.nodata();
        let fill = nodata.unwrap_or(f64::NAN);
        let mut data = Vec::with_capacity((r1 - r0) * (c1 - c0));
        for r in r0..r1 {
            for col in c0..c1 {
                let v = self.level_value(level, r, col);
                data.push(if v.is_nan() { fill } else { v });
            }
        }
        Dem {
            width: c1 - c0,
            height: r1 - r0,
            data,
            geotransform: [a, 0.0, c + a * c0 as f64, 0.0, e, f + e * r0 as f64],
            nodata,
            epsg: self.epsg(),
        }
    }

    /// [`Terrain::window_level`] at the coarsest level whose cell is no larger than `cell`.
    fn window(&self, bounds: (f64, f64, f64, f64), cell: f64) -> Dem {
        self.window_level(self.level_for_cell(cell), bounds)
    }

    /// [`Terrain::window_level`] at the finest level whose window holds at most `max_cells` cells.
    fn window_within(&self, bounds: (f64, f64, f64, f64), max_cells: usize) -> Dem {
        let levels = self.levels();
        let (w, h) = (bounds.2 - bounds.0, bounds.3 - bounds.1);
        let level = (0..levels.len())
            .find(|&l| {
                let g = levels[l].geotransform;
                (w / g[0].abs()).ceil() * (h / g[4].abs()).ceil() <= max_cells as f64
            })
            .unwrap_or(levels.len() - 1);
        self.window_level(level, bounds)
    }

    /// The sectioned reader behind this terrain, if it is one.
    fn sectioned(&self) -> Option<&SectionedDem> {
        None
    }

    /// The last block the reader could not read, as a message. A read that fails gives NaN; a
    /// surface asks here after a stage so a lost file is said plainly, not taken for missing
    /// ground.
    fn read_error(&self) -> Option<String> {
        None
    }
}

/// A terrain shared between threads and surfaces.
pub type TerrainRef = Arc<dyn Terrain>;

/// A shared terrain reads as the terrain it holds: every method forwarded, so nothing changes.
impl<T: Terrain + ?Sized> Terrain for Arc<T> {
    fn full(&self) -> Level {
        (**self).full()
    }
    fn nodata(&self) -> Option<f64> {
        (**self).nodata()
    }
    fn epsg(&self) -> Option<u32> {
        (**self).epsg()
    }
    fn width(&self) -> usize {
        (**self).width()
    }
    fn height(&self) -> usize {
        (**self).height()
    }
    fn geotransform(&self) -> [f64; 6] {
        (**self).geotransform()
    }
    fn bounds(&self) -> (f64, f64, f64, f64) {
        (**self).bounds()
    }
    fn levels(&self) -> Vec<Level> {
        (**self).levels()
    }
    fn level_for_cell(&self, cell: f64) -> usize {
        (**self).level_for_cell(cell)
    }
    fn level_value(&self, level: usize, row: usize, col: usize) -> f64 {
        (**self).level_value(level, row, col)
    }
    fn value(&self, row: usize, col: usize) -> f64 {
        (**self).value(row, col)
    }
    fn sample_bilinear(&self, x: f64, y: f64) -> f64 {
        (**self).sample_bilinear(x, y)
    }
    fn sample_bilinear_level(&self, level: usize, x: f64, y: f64) -> f64 {
        (**self).sample_bilinear_level(level, x, y)
    }
    fn sample_many(&self, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        (**self).sample_many(xs, ys)
    }
    fn sample_many_level(&self, level: usize, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        (**self).sample_many_level(level, xs, ys)
    }
    fn count_below(&self, r0: usize, r1: usize, c0: usize, c1: usize, level: f64) -> u32 {
        (**self).count_below(r0, r1, c0, c1, level)
    }
    fn summary(&self) -> Option<HeightSummary> {
        (**self).summary()
    }
    fn window_level(&self, level: usize, bounds: (f64, f64, f64, f64)) -> Dem {
        (**self).window_level(level, bounds)
    }
    fn window(&self, bounds: (f64, f64, f64, f64), cell: f64) -> Dem {
        (**self).window(bounds, cell)
    }
    fn window_within(&self, bounds: (f64, f64, f64, f64), max_cells: usize) -> Dem {
        (**self).window_within(bounds, max_cells)
    }
    fn sectioned(&self) -> Option<&SectionedDem> {
        (**self).sectioned()
    }
    fn read_error(&self) -> Option<String> {
        (**self).read_error()
    }
}

impl Terrain for Dem {
    fn full(&self) -> Level {
        Level {
            width: self.width,
            height: self.height,
            geotransform: self.geotransform,
        }
    }
    fn nodata(&self) -> Option<f64> {
        self.nodata
    }
    fn epsg(&self) -> Option<u32> {
        self.epsg
    }
    fn level_value(&self, _level: usize, row: usize, col: usize) -> f64 {
        Dem::value(self, row, col)
    }
    fn sample_bilinear(&self, x: f64, y: f64) -> f64 {
        Dem::sample_bilinear(self, x, y)
    }
    fn sample_bilinear_level(&self, _level: usize, x: f64, y: f64) -> f64 {
        Dem::sample_bilinear(self, x, y)
    }
    fn count_below(&self, r0: usize, r1: usize, c0: usize, c1: usize, level: f64) -> u32 {
        let mut n = 0u32;
        for r in r0..r1 {
            for &v in &self.data[r * self.width + c0..r * self.width + c1] {
                if v < level && self.nodata != Some(v) {
                    n += 1;
                }
            }
        }
        n
    }
    fn summary(&self) -> Option<HeightSummary> {
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for &v in &self.data {
            if v.is_finite() && self.nodata != Some(v) {
                min = min.min(v);
                max = max.max(v);
            }
        }
        if min > max {
            return None;
        }
        // The median, from at most about 200,000 heights spread over the DEM.
        let every = (self.data.len() / 200_000).max(1);
        let mut some: Vec<f64> = self
            .data
            .iter()
            .step_by(every)
            .copied()
            .filter(|v| v.is_finite() && self.nodata != Some(*v))
            .collect();
        let median = if some.is_empty() {
            min
        } else {
            let mid = some.len() / 2;
            *some.select_nth_unstable_by(mid, f64::total_cmp).1
        };
        Some(HeightSummary { min, max, median })
    }
}

/// A decoded block of one band: `f32` unless the file's type needs more.
#[derive(Debug, Clone)]
pub enum Block {
    F32(Arc<[f32]>),
    F64(Arc<[f64]>),
}

impl Block {
    fn get(&self, i: usize) -> f64 {
        match self {
            Block::F32(v) => f64::from(v[i]),
            Block::F64(v) => v[i],
        }
    }
}

impl Weigh for Block {
    fn bytes(&self) -> u64 {
        match self {
            Block::F32(v) => 4 * v.len() as u64,
            Block::F64(v) => 8 * v.len() as u64,
        }
    }
}

/// The default memory for decoded blocks: about 256 MiB natively, 128 MiB in the browser.
pub const DEFAULT_CACHE_BYTES: u64 = if cfg!(target_arch = "wasm32") {
    128 << 20
} else {
    256 << 20
};

/// A tiled GeoTIFF read a block at a time.
pub struct SectionedDem {
    source: SharedSource,
    layout: Layout,
    geo: Geo,
    levels: Vec<Level>,
    /// The image of each level in `layout.images`.
    images: Vec<usize>,
    cache: Mutex<Lru<(u8, u32), Block>>,
    error: Mutex<Option<String>>,
}

impl core::fmt::Debug for SectionedDem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SectionedDem")
            .field("levels", &self.levels)
            .field("nodata", &self.geo.nodata)
            .field("epsg", &self.geo.epsg)
            .finish()
    }
}

/// The plain refusal of a file in strips, with what to do.
pub fn strips_message() -> String {
    "this GeoTIFF is in strips, not tiles: run `owd dem prepare` to make a sectioned copy".into()
}

impl SectionedDem {
    /// Open a tiled GeoTIFF: its directories are read, no block is. `cache_bytes` is the memory
    /// the decoded blocks may use.
    pub fn open(source: SharedSource, cache_bytes: u64) -> Result<SectionedDem, GeoTiffError> {
        let layout = Layout::read(&*source)?;
        let full = &layout.images[0];
        if !full.tiled {
            return Err(GeoTiffError::Unsupported(strips_message()));
        }
        if full.bands != 1 {
            return Err(GeoTiffError::Unsupported(format!(
                "{} bands; terrain is read from a single-band raster",
                full.bands
            )));
        }
        full.readable()?;
        let geo = layout.geo.clone()?;
        let [a, _, c, _, e, f] = geo.geotransform;
        let (w0, h0) = (full.width as f64, full.height as f64);
        let mut levels = Vec::new();
        let mut images = Vec::new();
        for (k, img) in layout.images.iter().enumerate() {
            let usable = img.tiled && img.bands == 1 && img.readable().is_ok();
            if k > 0 && !usable {
                continue; // an overview the reader cannot use is passed over, not refused
            }
            levels.push(Level {
                width: img.width,
                height: img.height,
                geotransform: [
                    a * w0 / img.width as f64,
                    0.0,
                    c,
                    0.0,
                    e * h0 / img.height as f64,
                    f,
                ],
            });
            images.push(k);
        }
        levels[0].geotransform = geo.geotransform;
        Ok(SectionedDem {
            source,
            layout,
            geo,
            levels,
            images,
            cache: Mutex::new(Lru::new(cache_bytes)),
            error: Mutex::new(None),
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn source(&self) -> &SharedSource {
        &self.source
    }

    pub fn image(&self, level: usize) -> &Image {
        &self.layout.images[self.images[level]]
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).stats()
    }

    pub fn reset_peak(&self) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset_peak();
    }

    pub fn cache_budget(&self) -> u64 {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .budget()
    }

    /// Set the memory the decoded blocks may use.
    pub fn set_cache_budget(&self, bytes: u64) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_budget(bytes);
    }

    /// Block `i` of `level`, from the cache or decoded.
    fn block(&self, cache: &mut Lru<(u8, u32), Block>, level: usize, i: usize) -> Option<Block> {
        let key = (level as u8, i as u32);
        if let Some(b) = cache.get(&key) {
            return Some(b);
        }
        let img = self.image(level);
        let fill = self.geo.nodata.unwrap_or(f64::NAN);
        match decode_block(&*self.source, self.layout.little, img, i, fill) {
            Ok(s) => {
                let b = match s {
                    Samples::F32(v) => Block::F32(v.into()),
                    Samples::F64(v) => Block::F64(v.into()),
                    other if img.sample.is_some_and(|t| t.fits_f32()) => {
                        Block::F32((0..other.len()).map(|k| other.get(k) as f32).collect())
                    }
                    other => Block::F64((0..other.len()).map(|k| other.get(k)).collect()),
                };
                cache.put(key, b.clone());
                Some(b)
            }
            Err(e) => {
                *self.error.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("block {i} of level {level}: {e}"));
                None
            }
        }
    }

    /// Read cells through a short list of the blocks used last, holding the cache once.
    fn with_reader<T>(
        &self,
        level: usize,
        f: impl FnOnce(&mut dyn FnMut(usize, usize) -> f64) -> T,
    ) -> T {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let img = self.image(level);
        let (bw, bh, across) = (img.block_w, img.block_h, img.across());
        let nodata = self.geo.nodata;
        let mut recent: Vec<(usize, Option<Block>)> = Vec::with_capacity(4);
        let mut read = |r: usize, c: usize| -> f64 {
            let i = (r / bh) * across + c / bw;
            let k = match recent.iter().position(|(j, _)| *j == i) {
                Some(k) => k,
                None => {
                    if recent.len() == 4 {
                        recent.remove(0);
                    }
                    recent.push((i, self.block(&mut cache, level, i)));
                    recent.len() - 1
                }
            };
            let Some(b) = &recent[k].1 else {
                return f64::NAN;
            };
            let v = b.get((r % bh) * bw + c % bw);
            match nodata {
                Some(nd) if v == nd => f64::NAN,
                _ => v,
            }
        };
        f(&mut read)
    }
}

impl Terrain for SectionedDem {
    fn full(&self) -> Level {
        self.levels[0]
    }
    fn nodata(&self) -> Option<f64> {
        self.geo.nodata
    }
    fn epsg(&self) -> Option<u32> {
        self.geo.epsg
    }
    fn levels(&self) -> Vec<Level> {
        self.levels.clone()
    }
    fn level_value(&self, level: usize, row: usize, col: usize) -> f64 {
        self.with_reader(level, |read| read(row, col))
    }
    fn sample_bilinear_level(&self, level: usize, x: f64, y: f64) -> f64 {
        let l = self.levels[level];
        self.with_reader(level, |read| {
            bilinear(&l.geotransform, l.width, l.height, x, y, read)
        })
    }
    fn sample_many_level(&self, level: usize, xs: &[f64], ys: &[f64]) -> Vec<f64> {
        let l = self.levels[level];
        self.with_reader(level, |read| {
            xs.iter()
                .zip(ys)
                .map(|(&x, &y)| bilinear(&l.geotransform, l.width, l.height, x, y, &mut *read))
                .collect()
        })
    }
    fn count_below(&self, r0: usize, r1: usize, c0: usize, c1: usize, level: f64) -> u32 {
        self.with_reader(0, |read| {
            let mut n = 0u32;
            for r in r0..r1 {
                for c in c0..c1 {
                    if read(r, c) < level {
                        n += 1;
                    }
                }
            }
            n
        })
    }
    fn window_level(&self, level: usize, bounds: (f64, f64, f64, f64)) -> Dem {
        // As the trait's, but through one hold of the cache.
        let l = self.levels[level];
        let [a, _, c, _, e, f] = l.geotransform;
        let span = |lo: f64, hi: f64, origin: f64, step: f64, n: usize| {
            let (i0, i1) = ((lo - origin) / step, (hi - origin) / step);
            let (i0, i1) = (i0.min(i1).floor(), i0.max(i1).ceil());
            let i0 = i0.clamp(0.0, n as f64) as usize;
            let i1 = i1.clamp(0.0, n as f64) as usize;
            (i0, i1.max(i0))
        };
        let (c0, c1) = span(bounds.0, bounds.2, c, a, l.width);
        let (r0, r1) = span(bounds.1, bounds.3, f, e, l.height);
        let nodata = self.geo.nodata;
        let fill = nodata.unwrap_or(f64::NAN);
        let data = self.with_reader(level, |read| {
            let mut data = Vec::with_capacity((r1 - r0) * (c1 - c0));
            for r in r0..r1 {
                for col in c0..c1 {
                    let v = read(r, col);
                    data.push(if v.is_nan() { fill } else { v });
                }
            }
            data
        });
        Dem {
            width: c1 - c0,
            height: r1 - r0,
            data,
            geotransform: [a, 0.0, c + a * c0 as f64, 0.0, e, f + e * r0 as f64],
            nodata,
            epsg: self.geo.epsg,
        }
    }
    /// From the coarsest level whose whole grid is at most about a million cells: the overviews
    /// are averages, so the lowest and highest heights are those of the averaged ground, a little
    /// inside the true ones.
    fn summary(&self) -> Option<HeightSummary> {
        let level = (0..self.levels.len())
            .find(|&l| self.levels[l].width * self.levels[l].height <= 1 << 20)
            .unwrap_or(self.levels.len() - 1);
        self.window_level(level, self.bounds()).summary()
    }
    fn sectioned(&self) -> Option<&SectionedDem> {
        Some(self)
    }
    fn read_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
