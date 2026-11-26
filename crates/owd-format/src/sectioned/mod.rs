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
    let layout = layout::Layout::read(&*src).ok();
    let tiled = layout.as_ref().is_some_and(|l| l.images[0].tiled);
    if src.len() >= from_bytes && tiled {
        return Ok(Arc::new(SectionedDem::open(src, cache_bytes)?));
    }
    // Read whole. A file whose grid would not fit is refused from its header, before it is read,
    // with what to do: a sectioned copy is read a block at a time whatever its size.
    if let Some(l) = &layout {
        let img = &l.images[0];
        let cells = img.width as u64 * img.height as u64;
        let need = cells * 8;
        if need > limit_bytes {
            let how = if img.tiled {
                "it is in tiles but small enough to read whole"
            } else {
                "it is in strips, not tiles, so OWD cannot read it in sections"
            };
            return Err(GeoTiffError::TooLarge(format!(
                "it has {cells} cells ({} x {}); read whole it needs {:.2} GiB, over the limit of \
                 {:.2} GiB, and {how}. Run `owd dem prepare FILE -o COPY.tif` to make \
                 a sectioned copy, which OWD reads a block at a time",
                img.width,
                img.height,
                need as f64 / f64::from(1u32 << 30),
                limit_bytes as f64 / f64::from(1u32 << 30),
            )));
        }
    }
    let dem = match src.as_slice() {
        Some(b) => crate::geotiff::read_bytes_within(b, limit_bytes)?,
        None => {
            let bytes = src
                .read_vec(0, src.len() as usize)
                .map_err(GeoTiffError::Io)?;
            crate::geotiff::read_bytes_within(&bytes, limit_bytes)?
        }
    };
    Ok(Arc::new(dem))
}

/// A terrain file a shell holds: where its bytes are, and its identities, each worked out once.
/// Cheap to clone.
#[derive(Clone)]
pub struct DemFile {
    source: SharedSource,
    ids: Arc<identity::Identities>,
}

impl core::fmt::Debug for DemFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DemFile")
            .field("len", &self.source.len())
            .finish()
    }
}

impl DemFile {
    pub fn new(source: SharedSource) -> DemFile {
        DemFile {
            source,
            ids: Arc::default(),
        }
    }

    /// A file held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> DemFile {
        DemFile::new(Arc::new(bytes))
    }

    pub fn source(&self) -> &SharedSource {
        &self.source
    }
}

impl ByteSource for DemFile {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        self.source.read_at(offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        self.source.as_slice()
    }
    fn identities(&self) -> Option<&identity::Identities> {
        Some(&self.ids)
    }
    fn shared(&self) -> Option<SharedSource> {
        Some(self.source.clone())
    }
}

/// The terrain in a file: in sections when it is a large tiled file held as a [`DemFile`] (see
/// [`open_terrain`]), else read whole, within the reader's memory limit.
pub fn terrain_of(src: &dyn ByteSource) -> Result<TerrainRef, GeoTiffError> {
    let limit = crate::geotiff::grid_limit();
    match src.shared() {
        Some(shared) => open_terrain(
            shared,
            SECTIONED_FROM_BYTES,
            limit,
            terrain::DEFAULT_CACHE_BYTES,
        ),
        None => match src.as_slice() {
            Some(b) => Ok(Arc::new(crate::geotiff::read_bytes_within(b, limit)?)),
            None => {
                let bytes = src
                    .read_vec(0, src.len() as usize)
                    .map_err(GeoTiffError::Io)?;
                Ok(Arc::new(crate::geotiff::read_bytes_within(&bytes, limit)?))
            }
        },
    }
}
