//! The layout of a TIFF read by byte range (COG-1): the header, every image directory (the full
//! image and its overviews), and one block (a tile or a strip) decoded at a time.
//!
//! What it reads: classic TIFF and BigTIFF, either byte order; tiles or strips; any number of bands
//! stored pixel-interleaved (a DEM has one, an orthophoto three); 8, 16, 32 bit integers and 32 or
//! 64 bit floats; uncompressed, DEFLATE and LZW; no predictor, the horizontal one (on integers, and
//! on floats as libtiff applies it, to the words as integers) and the floating-point one.
//!
//! What it refuses, by name: any other compression (JPEG among them: a later ticket reads it with
//! the JPEG tables the directory carries, which are kept here), bands stored as separate planes,
//! 64 bit integers, 16 bit floats, and a damaged file. The geotransform, EPSG and nodata are read
//! exactly as [`crate::geotiff`] reads them, so the two readers agree.

use super::source::ByteSource;
use crate::geotiff::GeoTiffError;

fn corrupt<T>(m: impl Into<String>) -> Result<T, GeoTiffError> {
    Err(GeoTiffError::Corrupt(m.into()))
}

fn unsupported<T>(m: impl Into<String>) -> Result<T, GeoTiffError> {
    Err(GeoTiffError::Unsupported(m.into()))
}

fn io(e: String) -> GeoTiffError {
    GeoTiffError::Io(e)
}

/// How samples are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    F64,
}

impl SampleType {
    pub fn bytes(self) -> usize {
        match self {
            SampleType::U8 | SampleType::I8 => 1,
            SampleType::U16 | SampleType::I16 => 2,
            SampleType::U32 | SampleType::I32 | SampleType::F32 => 4,
            SampleType::F64 => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SampleType::U8 => "8-bit unsigned integer",
            SampleType::I8 => "8-bit signed integer",
            SampleType::U16 => "16-bit unsigned integer",
            SampleType::I16 => "16-bit signed integer",
            SampleType::U32 => "32-bit unsigned integer",
            SampleType::I32 => "32-bit signed integer",
            SampleType::F32 => "32-bit float",
            SampleType::F64 => "64-bit float",
        }
    }

    /// Whether a value of this type is held exactly in an `f32`.
    pub fn fits_f32(self) -> bool {
        !matches!(self, SampleType::U32 | SampleType::I32 | SampleType::F64)
    }
}

/// The name of a TIFF compression code.
pub fn compression_name(code: u16) -> String {
    let name = match code {
        1 => "none",
        2 => "CCITT Huffman",
        3 => "CCITT Group 3 fax",
        4 => "CCITT Group 4 fax",
        5 => "LZW",
        6 | 7 => "JPEG",
        8 | 32946 => "DEFLATE",
        32773 => "PackBits",
        34887 => "LERC",
        34925 => "LZMA",
        50000 => "ZSTD",
        50001 => "WebP",
        _ => "an unknown compression",
    };
    format!("{name} (TIFF compression code {code})")
}

/// One image of the file: the full one or an overview.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Where its directory starts in the file.
    pub ifd_offset: u64,
    pub width: usize,
    pub height: usize,
    /// The size of a block: a tile, or a strip (the image's width by its rows per strip).
    pub block_w: usize,
    pub block_h: usize,
    pub tiled: bool,
    pub bands: usize,
    pub sample: Option<SampleType>,
    /// Bits per sample and sample format as the file gives them (for a message when `sample` is
    /// `None`).
    pub bits: u16,
    pub format: u16,
    pub compression: u16,
    pub predictor: u16,
    pub planar: u16,
    pub photometric: u16,
    /// NewSubfileType: 1 marks a reduced image (an overview), 4 a mask.
    pub subfile: u32,
    pub offsets: Vec<u64>,
    pub counts: Vec<u64>,
    /// The JPEG tables shared by every tile (JPEGTABLESMODE 1), kept for the imagery reader.
    pub jpeg_tables: Option<Vec<u8>>,
}

impl Image {
    pub fn across(&self) -> usize {
        self.width.div_ceil(self.block_w)
    }
    pub fn down(&self) -> usize {
        self.height.div_ceil(self.block_h)
    }
    pub fn blocks(&self) -> usize {
        self.across() * self.down()
    }
    /// The rows that hold image data in block `i` (a last strip or tile row may be short).
    pub fn rows_in(&self, i: usize) -> usize {
        let r0 = (i / self.across()) * self.block_h;
        self.block_h.min(self.height - r0)
    }
    /// Whether the reader can decode this image's blocks; the reason when not.
    pub fn readable(&self) -> Result<SampleType, GeoTiffError> {
        let Some(s) = self.sample else {
            let kind = match self.format {
                3 => "float",
                2 => "signed integer",
                _ => "unsigned integer",
            };
            return unsupported(format!(
                "{}-bit {kind} samples; the reader handles 8, 16 and 32 bit integers and 32 or 64 bit floats",
                self.bits
            ));
        };
        if !matches!(self.compression, 1 | 5 | 8 | 32946) {
            let jpeg = if matches!(self.compression, 6 | 7) {
                " (a JPEG orthophoto is read by the imagery reader, not as terrain)"
            } else {
                ""
            };
            return unsupported(format!(
                "compression {}{jpeg}; the reader handles uncompressed, DEFLATE and LZW",
                compression_name(self.compression)
            ));
        }
        if self.bands > 1 && self.planar == 2 {
            return unsupported(
                "bands stored as separate planes; the reader handles interleaved pixels",
            );
        }
        if !matches!(self.predictor, 1..=3) {
            return unsupported(format!("predictor {}", self.predictor));
        }
        if self.predictor == 3 && !matches!(s, SampleType::F32 | SampleType::F64) {
            return unsupported("the floating-point predictor on integer samples");
        }
        Ok(s)
    }
}

/// The georeferencing of the file, from the first image.
#[derive(Debug, Clone, PartialEq)]
pub struct Geo {
    pub geotransform: [f64; 6],
    pub epsg: Option<u32>,
    pub nodata: Option<f64>,
}

/// A TIFF's directories, read once.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub little: bool,
    pub big: bool,
    /// The full image first, then the overviews from the largest down. Masks are left out.
    pub images: Vec<Image>,
    /// `None` when the first image has no georeferencing (the error says why).
    pub geo: Result<Geo, GeoTiffError>,
    /// The bytes that hold the header, every directory and every out-of-line tag value, as
    /// `(offset, length)`: what the quick identity check hashes besides the tiles.
    pub structure: Vec<(u64, u64)>,
}

/// A directory entry, its value bytes read.
struct Entry {
    tag: u16,
    typ: u16,
    data: Vec<u8>,
}

struct Reader<'a> {
    src: &'a dyn ByteSource,
    little: bool,
    big: bool,
    structure: Vec<(u64, u64)>,
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 0,
    }
}

impl Reader<'_> {
    fn read(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, GeoTiffError> {
        self.structure.push((offset, len as u64));
        self.src.read_vec(offset, len).map_err(io)
    }

    fn uint(&self, b: &[u8]) -> u64 {
        let mut v = 0u64;
        for i in 0..b.len() {
            let byte = if self.little {
                b[b.len() - 1 - i]
            } else {
                b[i]
            };
            v = (v << 8) | u64::from(byte);
        }
        v
    }

    fn entries(&mut self, at: u64) -> Result<(Vec<Entry>, u64), GeoTiffError> {
        let (count_len, entry_len, inline) = if self.big { (8, 20, 8) } else { (2, 12, 4) };
        let head = self.read(at, count_len)?;
        let n = self.uint(&head);
        if n > 4096 {
            return corrupt(format!("a directory of {n} entries at byte {at}"));
        }
        let n = n as usize;
        let body = self.read(at + count_len as u64, n * entry_len + inline)?;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let e = &body[k * entry_len..(k + 1) * entry_len];
            let tag = self.uint(&e[0..2]) as u16;
            let typ = self.uint(&e[2..4]) as u16;
            let (count, value) = if self.big {
                (self.uint(&e[4..12]), &e[12..20])
            } else {
                (self.uint(&e[4..8]), &e[8..12])
            };
            let size = type_size(typ) as u64;
            let total = size.saturating_mul(count);
            if size == 0 {
                out.push(Entry {
                    tag,
                    typ,
                    data: Vec::new(),
                });
                continue;
            }
            if total > 1 << 30 || total > self.src.len() {
                return corrupt(format!("tag {tag} claims {count} values"));
            }
            let data = if total <= inline as u64 {
                value[..total as usize].to_vec()
            } else {
                let off = self.uint(value);
                self.read(off, total as usize)?
            };
            out.push(Entry { tag, typ, data });
        }
        let next = self.uint(&body[n * entry_len..n * entry_len + inline]);
        Ok((out, next))
    }

    fn ints(&self, e: &Entry) -> Vec<u64> {
        let s = type_size(e.typ);
        e.data.chunks(s).map(|c| self.uint(c)).collect()
    }

    fn floats(&self, e: &Entry) -> Vec<f64> {
        let s = type_size(e.typ);
        e.data
            .chunks(s)
            .map(|c| match e.typ {
                11 => f64::from(f32::from_bits(self.uint(c) as u32)),
                12 => f64::from_bits(self.uint(c)),
                5 => {
                    let (n, d) = (self.uint(&c[..4]), self.uint(&c[4..]));
                    n as f64 / d as f64
                }
                _ => self.uint(c) as f64,
            })
            .collect()
    }
}

fn find(es: &[Entry], tag: u16) -> Option<&Entry> {
    es.iter().find(|e| e.tag == tag)
}

impl Layout {
    /// Read the header and every directory of the file in `src`.
    pub fn read(src: &dyn ByteSource) -> Result<Layout, GeoTiffError> {
        if src.len() < 8 {
            return corrupt("the file is shorter than a TIFF header");
        }
        let head = src.read_vec(0, 16.min(src.len() as usize)).map_err(io)?;
        let little = match &head[..2] {
            b"II" => true,
            b"MM" => false,
            _ => return corrupt("not a TIFF (no II or MM byte-order mark)"),
        };
        let mut r = Reader {
            src,
            little,
            big: false,
            structure: vec![(0, head.len() as u64)],
        };
        let magic = r.uint(&head[2..4]);
        let mut at = match magic {
            42 => r.uint(&head[4..8]),
            43 if head.len() >= 16 => {
                r.big = true;
                r.uint(&head[8..16])
            }
            _ => return corrupt(format!("not a TIFF (version {magic})")),
        };
        let mut images = Vec::new();
        let mut first: Option<Vec<Entry>> = None;
        let mut seen = std::collections::HashSet::new();
        while at != 0 {
            if !seen.insert(at) || images.len() > 64 {
                return corrupt("the directories loop");
            }
            let (es, next) = r.entries(at)?;
            let img = image(&r, &es, at)?;
            if img.subfile & 4 == 0 {
                images.push(img);
            }
            if first.is_none() {
                first = Some(es);
            }
            at = next;
        }
        let Some(first) = first else {
            return corrupt("the TIFF has no image");
        };
        if images.is_empty() {
            return corrupt("the TIFF holds only masks");
        }
        // The full image first, then the overviews by size, largest first.
        let full = images.remove(0);
        images.retain(|i| i.width < full.width || i.height < full.height);
        images.sort_by_key(|i| std::cmp::Reverse(i.width));
        images.insert(0, full);
        let geo = geo(&r, &first);
        Ok(Layout {
            little,
            big: r.big,
            images,
            geo,
            structure: r.structure,
        })
    }
}

fn image(r: &Reader, es: &[Entry], at: u64) -> Result<Image, GeoTiffError> {
    let one = |tag: u16| find(es, tag).and_then(|e| r.ints(e).first().copied());
    let width = one(256).ok_or(GeoTiffError::Corrupt("no ImageWidth".into()))? as usize;
    let height = one(257).ok_or(GeoTiffError::Corrupt("no ImageLength".into()))? as usize;
    let bands = one(277).unwrap_or(1) as usize;
    let bits_all = find(es, 258).map(|e| r.ints(e)).unwrap_or_else(|| vec![1]);
    let bits = bits_all.iter().copied().max().unwrap_or(1) as u16;
    if bits_all.iter().any(|&b| b != u64::from(bits)) {
        return unsupported("bands of different sample sizes");
    }
    let format = one(339).unwrap_or(1) as u16;
    let sample = match (format, bits) {
        (1, 8) => Some(SampleType::U8),
        (2, 8) => Some(SampleType::I8),
        (1, 16) => Some(SampleType::U16),
        (2, 16) => Some(SampleType::I16),
        (1, 32) => Some(SampleType::U32),
        (2, 32) => Some(SampleType::I32),
        (3, 32) => Some(SampleType::F32),
        (3, 64) => Some(SampleType::F64),
        _ => None,
    };
    let tiled = find(es, 322).is_some();
    let (block_w, block_h, offsets, counts) = if tiled {
        let tw = one(322).unwrap_or(0) as usize;
        let th = one(323).unwrap_or(0) as usize;
        let off = find(es, 324).map(|e| r.ints(e)).unwrap_or_default();
        let cnt = find(es, 325).map(|e| r.ints(e)).unwrap_or_default();
        (tw, th, off, cnt)
    } else {
        let rps = one(278).map_or(height, |v| (v as usize).clamp(1, height.max(1)));
        let off = find(es, 273).map(|e| r.ints(e)).unwrap_or_default();
        let cnt = find(es, 279).map(|e| r.ints(e)).unwrap_or_default();
        (width, rps, off, cnt)
    };
    if width == 0 || height == 0 || block_w == 0 || block_h == 0 {
        return corrupt("an image or a block of no size");
    }
    let img = Image {
        ifd_offset: at,
        width,
        height,
        block_w,
        block_h,
        tiled,
        bands,
        sample,
        bits,
        format,
        compression: one(259).unwrap_or(1) as u16,
        predictor: one(317).unwrap_or(1) as u16,
        planar: one(284).unwrap_or(1) as u16,
        photometric: one(262).unwrap_or(1) as u16,
        subfile: one(254).unwrap_or(0) as u32,
        offsets,
        counts,
        jpeg_tables: find(es, 347).map(|e| e.data.clone()),
    };
    let want = img.blocks() * if img.planar == 2 { bands } else { 1 };
    if img.offsets.len() < want || img.counts.len() < want {
        return corrupt(format!(
            "the image of {width} x {height} has {} block offsets, expected {want}",
            img.offsets.len()
        ));
    }
    Ok(img)
}

/// The georeferencing, read as [`crate::geotiff`] reads it.
fn geo(r: &Reader, es: &[Entry]) -> Result<Geo, GeoTiffError> {
    let (mut epsg, mut point) = (None, false);
    if let Some(e) = find(es, 34735) {
        let dir = r.ints(e);
        if dir.len() < 4 || dir.len() < 4 + 4 * dir[3] as usize {
            return corrupt("the GeoKey directory is truncated");
        }
        for k in 0..dir[3] as usize {
            let e = &dir[4 + 4 * k..8 + 4 * k];
            let (key, location, value) = (e[0], e[1], e[3]);
            if location != 0 {
                continue;
            }
            match key {
                3072 if value != 32767 => epsg = Some(value as u32),
                2048 if value != 32767 && epsg.is_none() => epsg = Some(value as u32),
                1025 => point = value == 2,
                _ => {}
            }
        }
    }
    let scale = find(es, 33550).map(|e| r.floats(e));
    let tie = find(es, 33922).map(|e| r.floats(e));
    let model = find(es, 34264).map(|e| r.floats(e));
    let mut gt = match (scale, tie, model) {
        (_, _, Some(m)) if m.len() == 16 => {
            if m[1] != 0.0 || m[4] != 0.0 {
                return unsupported(
                    "a rotated or sheared geotransform (ModelTransformation); only north-up rasters are read",
                );
            }
            [m[0], 0.0, m[3], 0.0, m[5], m[7]]
        }
        (Some(s), Some(t), None) if s.len() >= 2 && t.len() == 6 => {
            let (sx, sy) = (s[0], s[1]);
            [sx, 0.0, t[3] - t[0] * sx, 0.0, -sy, t[4] + t[1] * sy]
        }
        (_, Some(t), None) if t.len() > 6 => {
            return unsupported("more than one tiepoint (ground control points)");
        }
        _ => {
            return unsupported(
                "no georeferencing: the ModelPixelScale and ModelTiepoint tags (or ModelTransformation) are missing",
            );
        }
    };
    if point {
        gt[2] -= gt[0] * 0.5;
        gt[5] -= gt[4] * 0.5;
    }
    let nodata = match find(es, 42113) {
        Some(e) => {
            let text = String::from_utf8_lossy(&e.data);
            let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            Some(text.parse::<f64>().map_err(|_| {
                GeoTiffError::Corrupt(format!("the GDAL_NODATA value `{text}` is not a number"))
            })?)
        }
        None => None,
    };
    Ok(Geo {
        geotransform: gt,
        epsg,
        nodata,
    })
}

/// The samples of one decoded block, pixel-interleaved, row by row across the whole block width
/// (`block_w * rows * bands` of them; a tile on the image's edge keeps its padding).
#[derive(Debug, Clone, PartialEq)]
pub enum Samples {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(v) => v.len(),
            Samples::I8(v) => v.len(),
            Samples::U16(v) => v.len(),
            Samples::I16(v) => v.len(),
            Samples::U32(v) => v.len(),
            Samples::I32(v) => v.len(),
            Samples::F32(v) => v.len(),
            Samples::F64(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Sample `i` as `f64`, exactly.
    pub fn get(&self, i: usize) -> f64 {
        match self {
            Samples::U8(v) => f64::from(v[i]),
            Samples::I8(v) => f64::from(v[i]),
            Samples::U16(v) => f64::from(v[i]),
            Samples::I16(v) => f64::from(v[i]),
            Samples::U32(v) => f64::from(v[i]),
            Samples::I32(v) => f64::from(v[i]),
            Samples::F32(v) => f64::from(v[i]),
            Samples::F64(v) => v[i],
        }
    }
}

/// Undo the horizontal predictor on native-endian words of `N` bytes: each sample is the sum of
/// the stored difference and the sample `stride` before it, in wrapping integer arithmetic.
/// libtiff applies it to float words as integers too, which is how PREDICTOR=2 on float32 works.
fn undo_horizontal<const N: usize>(row: &mut [u8], stride: usize, little: bool) {
    let words = row.len() / N;
    let get = |b: &[u8]| -> u64 {
        let mut a = [0u8; 8];
        if little {
            a[..N].copy_from_slice(b);
            u64::from_le_bytes(a)
        } else {
            a[8 - N..].copy_from_slice(b);
            u64::from_be_bytes(a)
        }
    };
    let mask = if N == 8 {
        u64::MAX
    } else {
        (1u64 << (8 * N)) - 1
    };
    for i in stride..words {
        let prev = get(&row[(i - stride) * N..(i - stride + 1) * N]);
        let cur = get(&row[i * N..(i + 1) * N]);
        let v = cur.wrapping_add(prev) & mask;
        let out = if little {
            v.to_le_bytes()[..N].to_vec()
        } else {
            v.to_be_bytes()[8 - N..].to_vec()
        };
        row[i * N..(i + 1) * N].copy_from_slice(&out);
    }
}

/// Undo the floating-point predictor (TIFF technical note 3) on one row of `wc` samples of `bps`
/// bytes with `stride` bands: byte differences undone, then the byte planes (most significant
/// first) put back together. The result is written in the file's byte order.
fn undo_float(row: &mut [u8], wc: usize, bps: usize, stride: usize, little: bool) {
    for i in stride..row.len() {
        row[i] = row[i].wrapping_add(row[i - stride]);
    }
    let tmp = row.to_vec();
    for j in 0..wc {
        for k in 0..bps {
            // Plane k holds byte k of each value, most significant first.
            let b = tmp[k * wc + j];
            let at = if little {
                j * bps + (bps - 1 - k)
            } else {
                j * bps + k
            };
            row[at] = b;
        }
    }
}

/// Decode block `i` of `img` from `src` into samples. `little` is the file's byte order. A block
/// with no bytes (a sparse file) is all `fill`.
pub fn decode_block(
    src: &dyn ByteSource,
    little: bool,
    img: &Image,
    i: usize,
    fill: f64,
) -> Result<Samples, GeoTiffError> {
    let sample = img.readable()?;
    let bps = sample.bytes();
    let rows = if img.tiled {
        img.block_h
    } else {
        img.rows_in(i)
    };
    let row_bytes = img.block_w * img.bands * bps;
    let want = row_bytes * rows;
    let (off, n) = (img.offsets[i], img.counts[i]);
    let mut raw = if n == 0 {
        Vec::new()
    } else {
        if n > (want as u64).saturating_mul(4).max(1 << 20) {
            return corrupt(format!("block {i} claims {n} bytes"));
        }
        src.read_vec(off, n as usize).map_err(io)?
    };
    if n == 0 {
        return Ok(filled(sample, rows * img.block_w * img.bands, fill));
    }
    let mut data = match img.compression {
        1 => std::mem::take(&mut raw),
        8 | 32946 => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&raw, want)
            .map_err(|e| GeoTiffError::Corrupt(format!("block {i}: DEFLATE stream: {e:?}")))?,
        5 => {
            let mut d = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            let mut out = Vec::with_capacity(want);
            let r = d.into_vec(&mut out).decode(&raw);
            if let Err(e) = r.status
                && out.len() < want
            {
                return corrupt(format!("block {i}: LZW stream: {e}"));
            }
            out
        }
        _ => unreachable!("refused by readable()"),
    };
    if data.len() < want {
        // A last strip may hold fewer rows than RowsPerStrip says; anything else is damage.
        return corrupt(format!(
            "block {i} decodes to {} bytes, expected {want}",
            data.len()
        ));
    }
    data.truncate(want);
    match img.predictor {
        2 => {
            for row in data.chunks_mut(row_bytes) {
                match bps {
                    1 => undo_horizontal::<1>(row, img.bands, little),
                    2 => undo_horizontal::<2>(row, img.bands, little),
                    4 => undo_horizontal::<4>(row, img.bands, little),
                    _ => undo_horizontal::<8>(row, img.bands, little),
                }
            }
        }
        3 => {
            for row in data.chunks_mut(row_bytes) {
                undo_float(row, img.block_w * img.bands, bps, img.bands, little);
            }
        }
        _ => {}
    }
    Ok(to_samples(sample, &data, little))
}

fn filled(s: SampleType, n: usize, fill: f64) -> Samples {
    match s {
        SampleType::U8 => Samples::U8(vec![fill as u8; n]),
        SampleType::I8 => Samples::I8(vec![fill as i8; n]),
        SampleType::U16 => Samples::U16(vec![fill as u16; n]),
        SampleType::I16 => Samples::I16(vec![fill as i16; n]),
        SampleType::U32 => Samples::U32(vec![fill as u32; n]),
        SampleType::I32 => Samples::I32(vec![fill as i32; n]),
        SampleType::F32 => Samples::F32(vec![fill as f32; n]),
        SampleType::F64 => Samples::F64(vec![fill; n]),
    }
}

fn to_samples(s: SampleType, d: &[u8], little: bool) -> Samples {
    macro_rules! conv {
        ($t:ty, $n:expr) => {
            d.chunks_exact($n)
                .map(|c| {
                    let a: [u8; $n] = c.try_into().expect("size");
                    if little {
                        <$t>::from_le_bytes(a)
                    } else {
                        <$t>::from_be_bytes(a)
                    }
                })
                .collect()
        };
    }
    match s {
        SampleType::U8 => Samples::U8(d.to_vec()),
        SampleType::I8 => Samples::I8(d.iter().map(|&b| b as i8).collect()),
        SampleType::U16 => Samples::U16(conv!(u16, 2)),
        SampleType::I16 => Samples::I16(conv!(i16, 2)),
        SampleType::U32 => Samples::U32(conv!(u32, 4)),
        SampleType::I32 => Samples::I32(conv!(i32, 4)),
        SampleType::F32 => Samples::F32(conv!(f32, 4)),
        SampleType::F64 => Samples::F64(conv!(f64, 8)),
    }
}
