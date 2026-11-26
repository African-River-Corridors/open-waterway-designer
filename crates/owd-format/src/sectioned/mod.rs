//! Terrain read in sections (COG-1): a large DEM read a block at a time, at the detail asked for.
//!
//! - [`source`]: random access to a file's bytes ([`ByteSource`]): memory, a file, a browser file.
//! - [`layout`]: the TIFF's directories (BigTIFF too) and one block decoded at a time, any number
//!   of bands.
//! - [`cache`]: a least-recently-used cache of decoded blocks within a memory budget.
//! - [`terrain`]: the one interface every reader of the ground asks through ([`Terrain`]), with the
//!   whole raster in memory ([`crate::geotiff::Dem`]) and the sectioned file ([`SectionedDem`]).
//! - [`identity`]: the file's identity without reading it all.
//! - [`write`]: a sectioned copy of a raster, in the cloud-optimised order (`owd dem prepare`).

pub mod cache;
pub mod identity;
pub mod layout;
pub mod source;
pub mod terrain;
pub mod write;

pub use source::{ByteSource, SharedSource};
pub use terrain::{SectionedDem, Terrain, TerrainRef};

use crate::geotiff::GeoTiffError;
use std::sync::Arc;

/// Files at least this large are read in sections when they are tiled; a smaller file is read
/// whole, as it always was (it is quick, and every stage gives the same number either way).
pub const SECTIONED_FROM_BYTES: u64 = 64 << 20;

/// Whether the file in `src` is a tiled TIFF: one the sectioned reader can open.
pub fn is_tiled(src: &dyn ByteSource) -> bool {
    layout::Layout::read(src).is_ok_and(|l| l.images[0].tiled)
}

/// Open the terrain in `src`: in sections when it is tiled and at least `from_bytes` long, else
/// whole within `limit_bytes` (the reader's memory limit for a whole grid).
pub fn open_terrain(
    src: SharedSource,
    from_bytes: u64,
    limit_bytes: u64,
    cache_bytes: u64,
) -> Result<TerrainRef, GeoTiffError> {
    if src.len() >= from_bytes && is_tiled(&*src) {
        return Ok(Arc::new(SectionedDem::open(src, cache_bytes)?));
    }
    let dem = match src.as_slice() {
        Some(b) => crate::geotiff::read_bytes_within(b, limit_bytes)?,
        None => {
            // A whole read of a file not in memory: refused by size before it is loaded.
            let need = src.len();
            if need > limit_bytes {
                return Err(GeoTiffError::TooLarge(format!(
                    "the file is {need} bytes and is not in tiles, so it cannot be read in sections; \
                     run `owd dem prepare` to make a sectioned copy"
                )));
            }
            let bytes = src.read_vec(0, need as usize).map_err(GeoTiffError::Io)?;
            crate::geotiff::read_bytes_within(&bytes, limit_bytes)?
        }
    };
    Ok(Arc::new(dem))
}
