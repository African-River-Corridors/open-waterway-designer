//! The identity of a terrain file without reading it all (COG-1, part 3).
//!
//! A design records which DEM it was made on. A small file is recorded by the SHA-256 of all its
//! bytes, as always. A large sectioned file (a tiled TIFF of [`super::SECTIONED_FROM_BYTES`] or
//! more) is recorded by its **quick check** instead, which reads a few megabytes whatever the
//! file's size:
//!
//! - the file's length in bytes;
//! - every byte of its structure: the header, every image directory, and every value a directory
//!   points to (among them the tables of block offsets and block byte counts, the georeferencing
//!   and the nodata value);
//! - the stored (compressed) bytes of a sample of blocks: of each image (the full one and every
//!   overview), every block when it has at most [`ALL_BLOCKS_UP_TO`], else [`SAMPLED_BLOCKS`]
//!   blocks spaced evenly by number: block `floor(i * (n - 1) / (SAMPLED_BLOCKS - 1))` for `i` in
//!   `0..SAMPLED_BLOCKS` (the first, the last, and the ones evenly between).
//!
//! **What it misses.** A change to the bytes of a block it does not sample, made in place so that
//! the block's length and offset stay the same, and the file's length too. A tool that rewrites a
//! block of a tiled TIFF almost always writes it at the end and moves its offset (which the check
//! reads), and a recompressed block almost always changes its length (which it reads too); what
//! is left is damage made in place: a flipped bit, or a block overwritten with another of the same
//! length. The full hash ([`sha256_streaming`]) catches that, and `owd project status --full`
//! computes it.

use super::layout::Layout;
use super::source::ByteSource;
use sha2::{Digest, Sha256};

/// The name of the method, written beside the hash, so a later method can be told apart.
pub const QUICK_METHOD: &str = "owd-quick-1";
/// An image of at most this many blocks has every block read.
pub const ALL_BLOCKS_UP_TO: usize = 64;
/// How many blocks are read of a larger image.
pub const SAMPLED_BLOCKS: usize = 32;

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The blocks of an image of `n` blocks the quick check reads.
pub fn sampled_blocks(n: usize) -> Vec<usize> {
    if n <= ALL_BLOCKS_UP_TO {
        return (0..n).collect();
    }
    let mut v: Vec<usize> = (0..SAMPLED_BLOCKS)
        .map(|i| i * (n - 1) / (SAMPLED_BLOCKS - 1))
        .collect();
    v.dedup();
    v
}

/// The quick check of a tiled TIFF: `(length, hex SHA-256)`. `Err` when the file is not a TIFF the
/// layout reader reads.
pub fn quick_check(src: &dyn ByteSource) -> Result<(u64, String), String> {
    let layout = Layout::read(src).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    h.update(QUICK_METHOD.as_bytes());
    h.update(src.len().to_le_bytes());
    for &(off, len) in &layout.structure {
        h.update(off.to_le_bytes());
        h.update(src.read_vec(off, len as usize)?);
    }
    for img in &layout.images {
        for i in sampled_blocks(img.offsets.len()) {
            let (off, len) = (img.offsets[i], img.counts[i]);
            h.update((i as u64).to_le_bytes());
            if len > 0 {
                h.update(src.read_vec(off, len as usize)?);
            }
        }
    }
    Ok((src.len(), hex(h.finalize())))
}

/// The SHA-256 of every byte of the file, read 8 MiB at a time: the file is never held whole.
/// `progress(done, total)` is called after each piece.
pub fn sha256_streaming(
    src: &dyn ByteSource,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, String> {
    if let Some(all) = src.as_slice() {
        progress(all.len() as u64, all.len() as u64);
        return Ok(hex(Sha256::digest(all)));
    }
    const PIECE: u64 = 8 << 20;
    let total = src.len();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; PIECE.min(total) as usize];
    let mut at = 0u64;
    while at < total {
        let n = PIECE.min(total - at) as usize;
        src.read_at(at, &mut buf[..n])?;
        h.update(&buf[..n]);
        at += n as u64;
        progress(at, total);
    }
    Ok(hex(h.finalize()))
}

/// How a design records its DEM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// The SHA-256 of all the bytes.
    Full(String),
    /// The quick check: length and hash.
    Quick { length: u64, sha256: String },
}

/// Whether a file of `len` bytes is recorded by its quick check: a tiled TIFF of at least
/// `from_bytes`. Every other file is recorded by its full hash.
pub fn records_quick(src: &dyn ByteSource, from_bytes: u64) -> bool {
    src.len() >= from_bytes && super::is_tiled(src)
}

/// What to record for the file in `src`.
pub fn record(src: &dyn ByteSource, from_bytes: u64) -> Result<Recorded, String> {
    if records_quick(src, from_bytes) {
        let (length, sha256) = quick_check(src)?;
        Ok(Recorded::Quick { length, sha256 })
    } else {
        Ok(Recorded::Full(sha256_streaming(src, &mut |_, _| {})?))
    }
}

/// A file's identities, each worked out once (a large file's full hash takes a while): what a
/// shell keeps beside the file it holds.
#[derive(Debug, Default)]
pub struct Identities {
    full: std::sync::OnceLock<Result<String, String>>,
    quick: std::sync::OnceLock<Result<(u64, String), String>>,
}

impl Identities {
    pub fn full(&self, src: &dyn ByteSource) -> Result<String, String> {
        self.full
            .get_or_init(|| sha256_streaming(src, &mut |_, _| {}))
            .clone()
    }
    /// The full hash with progress, worked out now unless it was already.
    pub fn full_with_progress(
        &self,
        src: &dyn ByteSource,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<String, String> {
        if let Some(v) = self.full.get() {
            return v.clone();
        }
        let v = sha256_streaming(src, progress);
        let _ = self.full.set(v.clone());
        v
    }
    pub fn quick(&self, src: &dyn ByteSource) -> Result<(u64, String), String> {
        self.quick.get_or_init(|| quick_check(src)).clone()
    }
}
