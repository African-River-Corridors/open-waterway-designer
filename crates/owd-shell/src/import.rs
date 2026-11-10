//! One import behind every surface: the app shell and `owd import` both call [`import`], so the
//! same files give the same document, provenance included.
//!
//! The caller reads the files (the shell has no file access of its own); this function does the
//! checks, the session calls and the provenance records.

use owd_format::design::provenance::{Mark, record};
use owd_format::design::{DesignError, ImportReport, Role, Session};
use owd_format::geotiff::{Dem, read_bytes};

/// A terrain file to import, already read by the caller.
pub struct Terrain<'a> {
    /// The path as the document records it: relative to the document's folder, with `/`.
    pub path: &'a str,
    /// The file as named in messages.
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// What an import did.
pub struct Imported {
    /// The project CRS and, per input, its source CRS and size (positions, or pixels for `dem`).
    pub report: ImportReport,
    /// The terrain, read, when one was imported.
    pub dem: Option<Dem>,
}

/// Why an import was refused.
#[derive(Debug)]
pub enum ImportError {
    /// The session refused an input.
    Design(DesignError),
    /// The terrain file does not read as a GeoTIFF.
    Terrain(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Design(e) => e.fmt(f),
            ImportError::Terrain(m) => f.write_str(m),
        }
    }
}

/// Import vector inputs (GeoJSON text) and a terrain file into `session`. Each input is marked
/// `user` in the provenance, dated `date` (`YYYY-MM-DD`, the day of the import). A terrain the
/// stages could not sample in the project CRS is refused, with the CRS named; with no project CRS
/// yet it is kept. A refusal leaves `session` as it was.
pub fn import(
    session: &mut Session,
    vectors: &[(Role, &str)],
    terrain: Option<Terrain>,
    date: &str,
) -> Result<Imported, ImportError> {
    let before = session.clone();
    let done = import_into(session, vectors, terrain, date);
    if done.is_err() {
        *session = before;
    }
    done
}

fn import_into(
    session: &mut Session,
    vectors: &[(Role, &str)],
    terrain: Option<Terrain>,
    date: &str,
) -> Result<Imported, ImportError> {
    let mut report = if vectors.is_empty() && terrain.is_some() {
        ImportReport {
            project_epsg: project_epsg(session).unwrap_or(0),
            inputs: Vec::new(),
            warnings: Vec::new(),
        }
    } else {
        // An empty import is refused here, by the session.
        let report = session.import(vectors).map_err(ImportError::Design)?;
        let roles: Vec<Role> = vectors.iter().map(|(r, _)| *r).collect();
        session
            .mark_user(&roles, date)
            .map_err(ImportError::Design)?;
        report
    };
    let Some(t) = terrain else {
        return Ok(Imported { report, dem: None });
    };
    let dem = read_bytes(t.bytes).map_err(|e| ImportError::Terrain(format!("{}: {e}", t.name)))?;
    let epsg = project_epsg(session);
    if epsg.is_some() {
        owd_format::design::terrain::dem_sampler(&dem, epsg)
            .map(|_| ())
            .map_err(ImportError::Design)?;
    }
    session.set_dem(t.path, t.bytes, &dem);
    session
        .set_provenance(
            "dem",
            record("user-file", date, "User-supplied data", Mark::User, false),
        )
        .map_err(ImportError::Design)?;
    report.inputs.push((
        "dem".to_string(),
        owd_format::design::terrain::dem_crs_name(&dem),
        dem.width * dem.height,
    ));
    Ok(Imported {
        report,
        dem: Some(dem),
    })
}

fn project_epsg(session: &Session) -> Option<u32> {
    session.document()["project"]["crs"]["epsg"]
        .as_u64()
        .map(|c| c as u32)
}

/// Import a bank dataset file (ticket B7) into `session`: the banks and the river polygon through
/// [`import`] (so each is marked in the provenance, dated `date`), and what the source says into
/// `inputs.banks`. The whole file is read first; a refusal leaves `session` as it was.
pub fn import_bank_dataset(
    session: &mut Session,
    text: &str,
    date: &str,
) -> Result<Imported, ImportError> {
    let file = owd_format::design::banks::read_bank_file(text).map_err(ImportError::Design)?;
    let mut s = session.clone();
    let imported = import(&mut s, &file.items(), None, date)?;
    s.set_bank_metadata(&file.banks);
    *session = s;
    Ok(imported)
}
