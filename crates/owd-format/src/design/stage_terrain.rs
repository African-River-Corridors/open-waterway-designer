//! The terrain rule, in one place: which terrain file a stage call uses.
//!
//! A document names its terrain file (the DEM) and records the file's hash. A stage uses that file
//! when it is beside the document with the recorded hash. Every stage call on every surface asks
//! [`stage_terrain`]; none reads [`Session::dem_check`] itself. What a surface does when there is no
//! terrain (refuse, or go on without it) stays the surface's and the stage's choice.

use super::session::Session;
use super::volumes::DemCheck;

/// Why a stage call has no terrain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTerrain {
    /// The document names no terrain file.
    NotNamed,
    /// No file at the path the document names (the path, as the document says it).
    Missing(String),
    /// A file whose hash differs from the recorded one (the path, as the document says it).
    Changed(String),
}

/// The terrain file a stage call uses: `bytes`, the file found beside the document at
/// [`Session::dem_path`] (`None` when there is none), when it is the file whose hash was recorded.
/// Otherwise the reason there is none.
pub fn stage_terrain<'a>(
    session: &Session,
    bytes: Option<&'a [u8]>,
) -> Result<&'a [u8], NoTerrain> {
    let path = || session.dem_path().unwrap_or("DEM").to_string();
    match (session.dem_check(bytes), bytes) {
        (DemCheck::Matches, Some(b)) => Ok(b),
        (DemCheck::NoDem, _) => Err(NoTerrain::NotNamed),
        (DemCheck::Changed, _) => Err(NoTerrain::Changed(path())),
        (DemCheck::Missing, _) | (DemCheck::Matches, None) => Err(NoTerrain::Missing(path())),
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use crate::geotiff::read_bytes;

    fn dem() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/web/river-dem.tif"
        ))
        .expect("examples/web/river-dem.tif")
    }

    #[test]
    fn the_named_file_with_its_hash_is_used_and_anything_else_is_named() {
        let mut s = Session::new("t", "2026-01-02");
        assert_eq!(stage_terrain(&s, None), Err(NoTerrain::NotNamed));
        let bytes = dem();
        s.set_dem("river-dem.tif", &bytes, &read_bytes(&bytes).unwrap());
        assert_eq!(stage_terrain(&s, Some(&bytes)), Ok(bytes.as_slice()));
        assert_eq!(
            stage_terrain(&s, None),
            Err(NoTerrain::Missing("river-dem.tif".into()))
        );
        let mut other = bytes.clone();
        other.push(0);
        assert_eq!(
            stage_terrain(&s, Some(&other)),
            Err(NoTerrain::Changed("river-dem.tif".into()))
        );
    }
}
