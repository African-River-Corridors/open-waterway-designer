//! Random access to the bytes of a file (COG-1): the one thing the sectioned reader needs from where
//! a file lives. Three sources now: bytes in memory, a file on disk, and (in `owd-web`) a browser
//! `File` read inside the worker. A later source reads the same byte ranges over HTTP.

use std::sync::Arc;

/// Random access to a file's bytes. `read_at` fills all of `buf` from `offset`, or fails: a short
/// read is an error, so a reader never decodes half a block.
pub trait ByteSource: Send + Sync {
    /// The length of the file in bytes.
    fn len(&self) -> u64;

    /// Whether the file is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fill `buf` with the bytes from `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String>;

    /// The bytes from `offset`, `len` of them.
    fn read_vec(&self, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        let mut v = vec![0u8; len];
        self.read_at(offset, &mut v)?;
        Ok(v)
    }

    /// The whole file in memory when the source already holds it there (no copy, no read).
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }

    /// The file's identities, kept so each is worked out once ([`super::DemFile`] keeps them).
    fn identities(&self) -> Option<&super::identity::Identities> {
        None
    }

    /// The source as a shared handle, when it is one (a [`super::DemFile`]): what a sectioned
    /// reader opened from it holds.
    fn shared(&self) -> Option<SharedSource> {
        None
    }
}

/// A shared source.
pub type SharedSource = Arc<dyn ByteSource>;

fn past_end(offset: u64, want: usize, len: u64) -> String {
    format!("a read of {want} bytes at byte {offset} runs past the end of the file ({len} bytes)")
}

fn read_slice(s: &[u8], offset: u64, buf: &mut [u8]) -> Result<(), String> {
    let start = usize::try_from(offset).map_err(|_| past_end(offset, buf.len(), s.len() as u64))?;
    let end = start
        .checked_add(buf.len())
        .filter(|&e| e <= s.len())
        .ok_or_else(|| past_end(offset, buf.len(), s.len() as u64))?;
    buf.copy_from_slice(&s[start..end]);
    Ok(())
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        read_slice(self, offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

impl<const N: usize> ByteSource for [u8; N] {
    fn len(&self) -> u64 {
        N as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        read_slice(self, offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        read_slice(self, offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        Some(self)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        (**self).read_at(offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
    fn identities(&self) -> Option<&super::identity::Identities> {
        (**self).identities()
    }
    fn shared(&self) -> Option<SharedSource> {
        (**self).shared()
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        (**self).read_at(offset, buf)
    }
    fn as_slice(&self) -> Option<&[u8]> {
        (**self).as_slice()
    }
    fn identities(&self) -> Option<&super::identity::Identities> {
        (**self).identities()
    }
    fn shared(&self) -> Option<SharedSource> {
        (**self).shared()
    }
}

/// A file on disk, read by range. The file is opened once and held; reads are serialised on it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct FileSource {
    path: std::path::PathBuf,
    file: std::sync::Mutex<std::fs::File>,
    len: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileSource {
    pub fn open(path: &std::path::Path) -> Result<FileSource, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let len = file
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        Ok(FileSource {
            path: path.to_path_buf(),
            file: std::sync::Mutex::new(file),
            len,
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        use std::io::{Read, Seek, SeekFrom};
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|e| e > self.len)
        {
            return Err(past_end(offset, buf.len(), self.len));
        }
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(offset))
            .and_then(|_| f.read_exact(buf))
            .map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// Counts the bytes and the reads that pass through a source: for tests and measurements.
#[derive(Debug)]
pub struct CountingSource<S> {
    pub inner: S,
    pub reads: std::sync::atomic::AtomicU64,
    pub bytes: std::sync::atomic::AtomicU64,
}

impl<S> CountingSource<S> {
    pub fn new(inner: S) -> CountingSource<S> {
        CountingSource {
            inner,
            reads: 0.into(),
            bytes: 0.into(),
        }
    }
}

impl<S: ByteSource> ByteSource for CountingSource<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        use std::sync::atomic::Ordering::Relaxed;
        self.reads.fetch_add(1, Relaxed);
        self.bytes.fetch_add(buf.len() as u64, Relaxed);
        self.inner.read_at(offset, buf)
    }
}
