//! Write a sectioned GeoTIFF (COG-1, part 4): tiles, DEFLATE or LZW with a predictor, overviews by
//! averaging, BigTIFF when the file needs it, in the cloud-optimised order, in bounded memory.
//!
//! The rows come in bands of one tile's height, north first; nothing holds more than a band of
//! each level. Each level's compressed tiles go to a spool of their own as they are made (a file
//! beside the output, or memory), and the file is put together at the end: the header, every
//! directory with its tables (the full image first, then the overviews from the largest), then the
//! tile data, the smallest overview first and the full image last. A reader over HTTP gets every
//! directory in the first request and the coarse levels in the next few.
//!
//! An overview cell is the mean of the cells of the level above it that are not nodata (two by two,
//! fewer at a right or bottom edge); with none, it is nodata.

use super::layout::SampleType;
use std::io::{Read, Seek, SeekFrom, Write};

/// The compression of the tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Deflate,
    Lzw,
}

impl Compression {
    fn code(self) -> u16 {
        match self {
            Compression::None => 1,
            Compression::Deflate => 8,
            Compression::Lzw => 5,
        }
    }
}

/// What to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub width: usize,
    pub height: usize,
    /// The side of a tile (a multiple of 16).
    pub tile: usize,
    pub sample: SampleType,
    pub compression: Compression,
    /// 1 none, 2 horizontal, 3 floating point (floats only).
    pub predictor: u16,
    /// `None`: BigTIFF when the file would pass 4 GiB.
    pub big: Option<bool>,
    /// How many overviews (each half the one above). `None`: down to about 256 cells a side.
    pub overviews: Option<usize>,
    pub nodata: Option<f64>,
    pub geotransform: [f64; 6],
    pub epsg: Option<u32>,
    /// Bytes left empty between the directories and the tile data: a test pushes the tiles past
    /// 4 GiB this way without writing 4 GiB.
    pub gap: u64,
}

impl Spec {
    /// The predictor that suits the sample type: floating point for floats, horizontal for
    /// integers.
    pub fn default_predictor(sample: SampleType) -> u16 {
        match sample {
            SampleType::F32 | SampleType::F64 => 3,
            _ => 2,
        }
    }

    /// The size of every level, the full one first.
    pub fn levels(&self) -> Vec<(usize, usize)> {
        let mut out = vec![(self.width, self.height)];
        let mut n = 0;
        loop {
            let (w, h) = *out.last().expect("one level");
            let more = match self.overviews {
                Some(k) => n < k,
                None => w.max(h) > 256,
            };
            if !more || (w == 1 && h == 1) {
                break;
            }
            out.push((w.div_ceil(2), h.div_ceil(2)));
            n += 1;
        }
        out
    }
}

/// Somewhere to keep a level's compressed tiles until the file is put together.
pub trait Spool: Read + Write + Seek {}
impl<T: Read + Write + Seek> Spool for T {}

/// One level being made: a band of rows waiting to be cut into tiles.
struct Building {
    w: usize,
    h: usize,
    /// The rows of the current tile row, `w` cells each.
    rows: Vec<f64>,
    /// The first row of the band in `rows`.
    band_row: usize,
    /// Half-rows waiting for their pair, for the level below (an odd row at the bottom is alone).
    pending: Option<Vec<f64>>,
    spool: Box<dyn Spool>,
    written: u64,
    counts: Vec<u64>,
}

/// Write `spec`'s raster to `out`. `band(r0, r1)` gives rows `r0..r1` (row-major, `width` cells
/// each, NaN or the nodata value for no data); it is asked for one tile row at a time, north first.
/// `spool(level)` gives a place for a level's tiles. `progress(rows_done, rows)` is called after
/// each band. Returns the size of the file.
pub fn write<W: Write + Seek>(
    spec: &Spec,
    mut band: impl FnMut(usize, usize) -> Result<Vec<f64>, String>,
    mut spool: impl FnMut(usize) -> std::io::Result<Box<dyn Spool>>,
    out: &mut W,
    mut progress: impl FnMut(usize, usize),
) -> Result<u64, String> {
    if spec.tile == 0 || !spec.tile.is_multiple_of(16) {
        return Err("the tile size must be a multiple of 16".into());
    }
    if spec.predictor == 3 && !matches!(spec.sample, SampleType::F32 | SampleType::F64) {
        return Err("the floating-point predictor is for float samples".into());
    }
    let sizes = spec.levels();
    let mut levels: Vec<Building> = Vec::with_capacity(sizes.len());
    for (k, &(w, h)) in sizes.iter().enumerate() {
        levels.push(Building {
            w,
            h,
            rows: Vec::new(),
            band_row: 0,
            pending: None,
            spool: spool(k).map_err(|e| format!("cannot make the spool of level {k}: {e}"))?,
            written: 0,
            counts: Vec::new(),
        });
    }
    let t = spec.tile;
    let mut r0 = 0;
    while r0 < spec.height {
        let r1 = (r0 + t).min(spec.height);
        let rows = band(r0, r1)?;
        if rows.len() != (r1 - r0) * spec.width {
            return Err(format!(
                "rows {r0}..{r1} came with {} cells, expected {}",
                rows.len(),
                (r1 - r0) * spec.width
            ));
        }
        for r in 0..(r1 - r0) {
            push_row(
                spec,
                &mut levels,
                0,
                &rows[r * spec.width..(r + 1) * spec.width],
            )?;
        }
        r0 = r1;
        progress(r0, spec.height);
    }
    // The last, odd rows of each level.
    for k in 0..levels.len() {
        if let Some(row) = levels[k].pending.take()
            && k + 1 < levels.len()
        {
            let half = halve(&row, None, spec.nodata);
            push_row(spec, &mut levels, k + 1, &half)?;
        }
        if !levels[k].rows.is_empty() {
            flush(spec, &mut levels[k])?;
        }
    }
    assemble(spec, &mut levels, out)
}

fn is_nodata(v: f64, nodata: Option<f64>) -> bool {
    v.is_nan() || nodata == Some(v)
}

/// A row of the level below: the mean of each 2 x 2 (or smaller at an edge) of rows `a` and `b`,
/// nodata left out.
fn halve(a: &[f64], b: Option<&[f64]>, nodata: Option<f64>) -> Vec<f64> {
    let w = a.len().div_ceil(2);
    let fill = nodata.unwrap_or(f64::NAN);
    (0..w)
        .map(|i| {
            let (mut s, mut n) = (0.0, 0u32);
            for row in [Some(a), b].into_iter().flatten() {
                for c in [2 * i, 2 * i + 1] {
                    if let Some(&v) = row.get(c)
                        && !is_nodata(v, nodata)
                    {
                        s += v;
                        n += 1;
                    }
                }
            }
            if n == 0 { fill } else { s / f64::from(n) }
        })
        .collect()
}

fn push_row(spec: &Spec, levels: &mut [Building], k: usize, row: &[f64]) -> Result<(), String> {
    let lv = &mut levels[k];
    lv.rows.extend_from_slice(row);
    if lv.rows.len() == spec.tile * lv.w {
        flush(spec, lv)?;
    }
    if k + 1 < levels.len() {
        match levels[k].pending.take() {
            None => levels[k].pending = Some(row.to_vec()),
            Some(above) => {
                let half = halve(&above, Some(row), spec.nodata);
                push_row(spec, levels, k + 1, &half)?;
            }
        }
    }
    Ok(())
}

/// Cut the band of `lv` into tiles and spool them.
fn flush(spec: &Spec, lv: &mut Building) -> Result<(), String> {
    let t = spec.tile;
    let n_rows = lv.rows.len() / lv.w;
    let pad = spec.nodata.unwrap_or(0.0);
    for tx in 0..lv.w.div_ceil(t) {
        let mut cells = Vec::with_capacity(t * t);
        for r in 0..t {
            for c in tx * t..(tx + 1) * t {
                cells.push(if r < n_rows && c < lv.w {
                    lv.rows[r * lv.w + c]
                } else {
                    pad
                });
            }
        }
        let bytes = encode_tile(spec, &cells)?;
        lv.spool
            .write_all(&bytes)
            .map_err(|e| format!("cannot write a tile: {e}"))?;
        lv.written += bytes.len() as u64;
        lv.counts.push(bytes.len() as u64);
    }
    lv.band_row += n_rows;
    lv.rows.clear();
    let _ = lv.h;
    Ok(())
}

/// One tile's cells as the file stores them: little-endian samples, the predictor, compressed.
pub fn encode_tile(spec: &Spec, cells: &[f64]) -> Result<Vec<u8>, String> {
    let s = spec.sample;
    let bps = s.bytes();
    let nd = spec.nodata;
    let mut raw = Vec::with_capacity(cells.len() * bps);
    for &v in cells {
        let v = if v.is_nan() {
            nd.unwrap_or(f64::NAN)
        } else {
            v
        };
        match s {
            SampleType::U8 => raw.push(v.round() as u8),
            SampleType::I8 => raw.push(v.round() as i8 as u8),
            SampleType::U16 => raw.extend_from_slice(&(v.round() as u16).to_le_bytes()),
            SampleType::I16 => raw.extend_from_slice(&(v.round() as i16).to_le_bytes()),
            SampleType::U32 => raw.extend_from_slice(&(v.round() as u32).to_le_bytes()),
            SampleType::I32 => raw.extend_from_slice(&(v.round() as i32).to_le_bytes()),
            SampleType::F32 => raw.extend_from_slice(&(v as f32).to_le_bytes()),
            SampleType::F64 => raw.extend_from_slice(&v.to_le_bytes()),
        }
    }
    let row_bytes = spec.tile * bps;
    match spec.predictor {
        2 => {
            for row in raw.chunks_mut(row_bytes) {
                let words = row.len() / bps;
                for i in (1..words).rev() {
                    let get = |b: &[u8]| {
                        let mut a = [0u8; 8];
                        a[..bps].copy_from_slice(b);
                        u64::from_le_bytes(a)
                    };
                    let cur = get(&row[i * bps..(i + 1) * bps]);
                    let prev = get(&row[(i - 1) * bps..i * bps]);
                    let d = cur.wrapping_sub(prev);
                    row[i * bps..(i + 1) * bps].copy_from_slice(&d.to_le_bytes()[..bps]);
                }
            }
        }
        3 => {
            for row in raw.chunks_mut(row_bytes) {
                let wc = row.len() / bps;
                let tmp = row.to_vec();
                for j in 0..wc {
                    for k in 0..bps {
                        // Plane k holds byte k of each value, most significant first.
                        row[k * wc + j] = tmp[j * bps + (bps - 1 - k)];
                    }
                }
                for i in (1..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - 1]);
                }
            }
        }
        _ => {}
    }
    Ok(match spec.compression {
        Compression::None => raw,
        Compression::Deflate => miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
        Compression::Lzw => weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(&raw)
            .map_err(|e| format!("LZW: {e}"))?,
    })
}

/// A directory entry to write: tag, type, values (as bytes, little-endian), count.
struct Tag {
    tag: u16,
    typ: u16,
    count: u64,
    data: Vec<u8>,
}

fn short(tag: u16, v: &[u16]) -> Tag {
    Tag {
        tag,
        typ: 3,
        count: v.len() as u64,
        data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}
fn long(tag: u16, v: u32) -> Tag {
    Tag {
        tag,
        typ: 4,
        count: 1,
        data: v.to_le_bytes().to_vec(),
    }
}
fn doubles(tag: u16, v: &[f64]) -> Tag {
    Tag {
        tag,
        typ: 12,
        count: v.len() as u64,
        data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}
fn offsets(tag: u16, v: &[u64], big: bool) -> Tag {
    if big {
        Tag {
            tag,
            typ: 16,
            count: v.len() as u64,
            data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    } else {
        Tag {
            tag,
            typ: 4,
            count: v.len() as u64,
            data: v.iter().flat_map(|&x| (x as u32).to_le_bytes()).collect(),
        }
    }
}

/// The tags of level `k`, with `offs` and `counts` its tiles' places.
fn tags(
    spec: &Spec,
    k: usize,
    w: usize,
    h: usize,
    offs: &[u64],
    counts: &[u64],
    big: bool,
) -> Vec<Tag> {
    let (bits, format) = match spec.sample {
        SampleType::U8 => (8, 1),
        SampleType::I8 => (8, 2),
        SampleType::U16 => (16, 1),
        SampleType::I16 => (16, 2),
        SampleType::U32 => (32, 1),
        SampleType::I32 => (32, 2),
        SampleType::F32 => (32, 3),
        SampleType::F64 => (64, 3),
    };
    let mut t = Vec::new();
    if k > 0 {
        t.push(long(254, 1));
    }
    t.push(long(256, w as u32));
    t.push(long(257, h as u32));
    t.push(short(258, &[bits]));
    t.push(short(259, &[spec.compression.code()]));
    t.push(short(262, &[1]));
    t.push(short(277, &[1]));
    t.push(short(284, &[1]));
    if spec.predictor != 1 {
        t.push(short(317, &[spec.predictor]));
    }
    t.push(long(322, spec.tile as u32));
    t.push(long(323, spec.tile as u32));
    t.push(offsets(324, offs, big));
    t.push(offsets(325, counts, big));
    t.push(short(339, &[format]));
    if k == 0 {
        let [a, _, c, _, e, f] = spec.geotransform;
        t.push(doubles(33550, &[a, -e, 0.0]));
        t.push(doubles(33922, &[0.0, 0.0, 0.0, c, f, 0.0]));
        let mut keys: Vec<u16> = vec![1, 1, 0, 0];
        if let Some(code) = spec.epsg {
            let geographic = code == 4326 || (4000..5000).contains(&code);
            keys.extend([1024, 0, 1, if geographic { 2 } else { 1 }]);
            keys.extend([1025, 0, 1, 1]);
            keys.extend([if geographic { 2048 } else { 3072 }, 0, 1, code as u16]);
        } else {
            keys.extend([1025, 0, 1, 1]);
        }
        keys[3] = ((keys.len() - 4) / 4) as u16;
        t.push(short(34735, &keys));
    }
    if let Some(nd) = spec.nodata {
        let mut s = format!("{nd}").into_bytes();
        s.push(0);
        t.push(Tag {
            tag: 42113,
            typ: 2,
            count: s.len() as u64,
            data: s,
        });
    }
    t
}

/// The size of a directory and its out-of-line values.
fn ifd_size(tags: &[Tag], big: bool) -> u64 {
    let (head, entry, tail, inline) = if big { (8, 20, 8, 8) } else { (2, 12, 4, 4) };
    let mut n = head + entry * tags.len() as u64 + tail;
    for t in tags {
        let len = t.data.len() as u64;
        if len > inline {
            n += len + (len & 1);
        }
    }
    n
}

/// Write one directory at `at` (`next`: the next directory's offset, 0 for none).
fn ifd_bytes(tags: &[Tag], big: bool, at: u64, next: u64) -> Vec<u8> {
    let (head, entry, tail, inline) = if big {
        (8u64, 20u64, 8u64, 8usize)
    } else {
        (2, 12, 4, 4)
    };
    let mut out = Vec::new();
    let mut extra = Vec::new();
    let mut extra_at = at + head + entry * tags.len() as u64 + tail;
    if big {
        out.extend((tags.len() as u64).to_le_bytes());
    } else {
        out.extend((tags.len() as u16).to_le_bytes());
    }
    for t in tags {
        out.extend(t.tag.to_le_bytes());
        out.extend(t.typ.to_le_bytes());
        if big {
            out.extend(t.count.to_le_bytes());
        } else {
            out.extend((t.count as u32).to_le_bytes());
        }
        let mut value = vec![0u8; inline];
        if t.data.len() <= inline {
            value[..t.data.len()].copy_from_slice(&t.data);
        } else {
            let off = if big {
                extra_at.to_le_bytes().to_vec()
            } else {
                (extra_at as u32).to_le_bytes().to_vec()
            };
            value.copy_from_slice(&off);
            extra.extend(&t.data);
            extra_at += t.data.len() as u64;
            if t.data.len() % 2 == 1 {
                extra.push(0);
                extra_at += 1;
            }
        }
        out.extend(value);
    }
    if big {
        out.extend(next.to_le_bytes());
    } else {
        out.extend((next as u32).to_le_bytes());
    }
    out.extend(extra);
    out
}

fn assemble<W: Write + Seek>(
    spec: &Spec,
    levels: &mut [Building],
    out: &mut W,
) -> Result<u64, String> {
    let data_total: u64 = levels.iter().map(|l| l.written).sum();
    let plan = |big: bool| -> (u64, Vec<u64>) {
        let head = if big { 16 } else { 8 };
        let mut at = head;
        let mut ifds = Vec::new();
        for (k, l) in levels.iter().enumerate() {
            let n = l.counts.len();
            let tg = tags(spec, k, l.w, l.h, &vec![0; n], &vec![0; n], big);
            ifds.push(at);
            at += ifd_size(&tg, big);
        }
        (at, ifds)
    };
    let big = match spec.big {
        Some(b) => b,
        None => {
            let (head, _) = plan(false);
            head + spec.gap + data_total > u64::from(u32::MAX) - 1024
        }
    };
    if !big && plan(false).0 + spec.gap + data_total > u64::from(u32::MAX) {
        return Err("the file passes 4 GiB: it must be a BigTIFF".into());
    }
    let (dirs_end, ifd_at) = plan(big);
    // Where each level's tiles start: the smallest overview first, the full image last.
    let mut start = vec![0u64; levels.len()];
    let mut at = dirs_end + spec.gap;
    for k in (0..levels.len()).rev() {
        start[k] = at;
        at += levels[k].written;
    }
    let total = at;
    let err = |e: std::io::Error| format!("cannot write the file: {e}");
    out.seek(SeekFrom::Start(0)).map_err(err)?;
    if big {
        out.write_all(b"II").map_err(err)?;
        out.write_all(&43u16.to_le_bytes()).map_err(err)?;
        out.write_all(&8u16.to_le_bytes()).map_err(err)?;
        out.write_all(&0u16.to_le_bytes()).map_err(err)?;
        out.write_all(&ifd_at[0].to_le_bytes()).map_err(err)?;
    } else {
        out.write_all(b"II").map_err(err)?;
        out.write_all(&42u16.to_le_bytes()).map_err(err)?;
        out.write_all(&(ifd_at[0] as u32).to_le_bytes())
            .map_err(err)?;
    }
    for (k, l) in levels.iter().enumerate() {
        let mut offs = Vec::with_capacity(l.counts.len());
        let mut o = start[k];
        for &c in &l.counts {
            offs.push(o);
            o += c;
        }
        let tg = tags(spec, k, l.w, l.h, &offs, &l.counts, big);
        let next = ifd_at.get(k + 1).copied().unwrap_or(0);
        out.write_all(&ifd_bytes(&tg, big, ifd_at[k], next))
            .map_err(err)?;
    }
    for k in (0..levels.len()).rev() {
        out.seek(SeekFrom::Start(start[k])).map_err(err)?;
        let l = &mut levels[k];
        l.spool.seek(SeekFrom::Start(0)).map_err(err)?;
        let copied = std::io::copy(&mut (&mut l.spool).take(l.written), out).map_err(err)?;
        if copied != l.written {
            return Err(format!("the spool of level {k} is short"));
        }
    }
    out.flush().map_err(err)?;
    Ok(total)
}

/// A file made of the parts written to it, with holes between: a [`Write`] + [`Seek`] target that
/// is also a [`super::source::ByteSource`], so a test can lay tiles past 4 GiB without 4 GiB of
/// memory. Bytes in a hole read as zero.
#[derive(Debug, Default)]
pub struct SparseFile {
    parts: std::collections::BTreeMap<u64, Vec<u8>>,
    pos: u64,
    len: u64,
}

impl Write for SparseFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Append to the part that ends here, or start one.
        let joined = self
            .parts
            .range_mut(..=self.pos)
            .next_back()
            .filter(|(s, v)| **s + v.len() as u64 == self.pos);
        match joined {
            Some((_, v)) => v.extend_from_slice(buf),
            None => {
                self.parts.insert(self.pos, buf.to_vec());
            }
        }
        self.pos += buf.len() as u64;
        self.len = self.len.max(self.pos);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for SparseFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::End(d) => (self.len as i64 + d) as u64,
            SeekFrom::Current(d) => (self.pos as i64 + d) as u64,
        };
        Ok(self.pos)
    }
}

impl super::source::ByteSource for SparseFile {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        if offset + buf.len() as u64 > self.len {
            return Err("a read past the end".into());
        }
        buf.fill(0);
        let end = offset + buf.len() as u64;
        for (&s, v) in self.parts.range(..end) {
            let e = s + v.len() as u64;
            if e <= offset {
                continue;
            }
            let (a, b) = (s.max(offset), e.min(end));
            buf[(a - offset) as usize..(b - offset) as usize]
                .copy_from_slice(&v[(a - s) as usize..(b - s) as usize]);
        }
        Ok(())
    }
}

/// Write `spec`'s raster in memory, the rows from `cell(row, col)`: for tests and small files.
pub fn write_to_vec(spec: &Spec, cell: impl Fn(usize, usize) -> f64) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    write(
        spec,
        |r0, r1| {
            Ok((r0..r1)
                .flat_map(|r| (0..spec.width).map(move |c| (r, c)))
                .map(|(r, c)| cell(r, c))
                .collect())
        },
        |_| Ok(Box::new(std::io::Cursor::new(Vec::new())) as Box<dyn Spool>),
        &mut out,
        |_, _| {},
    )?;
    Ok(out.into_inner())
}
