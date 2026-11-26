//! The identity of a DEM without reading it all (COG-1, part 3): the quick check catches a changed
//! block it samples and a changed length; the miss it documents is real; a design that records a
//! full hash keeps working on a file read by range; the terrain rule still has one reader.
//!
//! Label: property

use owd_format::design::{DemCheck, Session};
use owd_format::sectioned::identity::{quick_check, sampled_blocks, sha256_streaming};
use owd_format::sectioned::layout::{Layout, SampleType};
use owd_format::sectioned::source::FileSource;
use owd_format::sectioned::write::{Compression, SparseFile, Spec, Spool, write, write_to_vec};
use owd_format::sectioned::{ByteSource, DemFile, SECTIONED_FROM_BYTES, SectionedDem};
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

fn spec(gap: u64) -> Spec {
    Spec {
        width: 640,
        height: 640,
        tile: 32,
        sample: SampleType::F32,
        compression: Compression::Deflate,
        predictor: 3,
        big: None,
        overviews: Some(2),
        nodata: Some(-9999.0),
        geotransform: [2.0, 0.0, 500_000.0, 0.0, -2.0, 600_000.0],
        epsg: Some(32630),
        gap,
    }
}

fn cell(r: usize, c: usize) -> f64 {
    50.0 + 0.01 * (r * 7 + c * 3) as f64
}

/// A sectioned file of 400 full-size blocks, laid out past a gap so it is larger than
/// [`SECTIONED_FROM_BYTES`] without that much memory.
fn large() -> SparseFile {
    let s = spec(SECTIONED_FROM_BYTES);
    let mut out = SparseFile::default();
    write(
        &s,
        |r0, r1| {
            Ok((r0..r1)
                .flat_map(|r| (0..640).map(move |c| cell(r, c)))
                .collect())
        },
        |_| Ok(Box::new(std::io::Cursor::new(Vec::new())) as Box<dyn Spool>),
        &mut out,
        |_, _| {},
    )
    .unwrap();
    out
}

/// Overwrite the bytes at `at` with `with`, in place.
fn poke(f: &mut SparseFile, at: u64, with: &[u8]) {
    f.seek(SeekFrom::Start(at)).unwrap();
    f.write_all(with).unwrap();
}

fn block_at(f: &SparseFile, i: usize) -> u64 {
    Layout::read(f).unwrap().images[0].offsets[i]
}

#[test]
fn the_quick_check_catches_a_sampled_block_and_a_new_length_and_misses_what_it_says() {
    let full = Layout::read(&large()).unwrap().images[0].offsets.len();
    assert_eq!(full, 400);
    let sampled = sampled_blocks(full);
    assert_eq!(sampled.len(), 32);
    assert!(sampled.contains(&0) && sampled.contains(&399) && !sampled.contains(&1));
    let base = quick_check(&large()).unwrap();

    // A sampled block changed in place: caught.
    let mut f = large();
    let at = block_at(&f, 0) + 10;
    let b = f.read_vec(at, 1).unwrap()[0];
    poke(&mut f, at, &[b ^ 0xff]);
    assert_ne!(quick_check(&f).unwrap(), base);

    // A byte added at the end: the length differs, caught.
    let mut f = large();
    let n = f.len();
    poke(&mut f, n, &[0]);
    assert_ne!(quick_check(&f).unwrap(), base);

    // The documented miss: a block it does not sample, changed in place, same length.
    let mut f = large();
    let at = block_at(&f, 1) + 10;
    let b = f.read_vec(at, 1).unwrap()[0];
    poke(&mut f, at, &[b ^ 0xff]);
    assert_eq!(
        quick_check(&f).unwrap(),
        base,
        "the quick check misses this, as documented"
    );
    let mut none = |_: u64, _: u64| {};
    assert_ne!(
        sha256_streaming(&f, &mut none).unwrap(),
        sha256_streaming(&large(), &mut none).unwrap(),
        "the full hash catches it"
    );
}

#[test]
fn a_large_sectioned_dem_is_recorded_by_its_quick_check_and_checked_without_reading_it_all() {
    let file = large();
    assert!(file.len() >= SECTIONED_FROM_BYTES);
    let dem = SectionedDem::open(Arc::new(large()), 1 << 24).unwrap();
    let mut s = Session::new("t", "2026-11-26");
    s.set_dem("big.tif", &file, &dem);
    let rec = &s.document()["inputs"]["dem"];
    assert!(rec.get("sha256").is_none(), "{rec}");
    assert_eq!(rec["quick_check"]["method"], "owd-quick-1");
    assert_eq!(rec["quick_check"]["length"], file.len());
    let counted = owd_format::sectioned::source::CountingSource::new(large());
    assert_eq!(s.dem_check(Some(&counted)), DemCheck::Matches);
    let read = counted.bytes.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        read < 1 << 20,
        "the check read {read} bytes of {}",
        file.len()
    );
    // Changed in a sampled block: refused.
    let mut f = large();
    let at = block_at(&f, 399) + 3;
    let b = f.read_vec(at, 1).unwrap()[0];
    poke(&mut f, at, &[b ^ 1]);
    assert_eq!(s.dem_check(Some(&f)), DemCheck::Changed);
    assert!(s.dem_warning(Some(&f)).unwrap().contains("changed"));
}

#[test]
fn a_design_that_records_a_full_hash_is_checked_streaming_on_a_file_read_by_range() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/web/river-dem.tif"
    );
    let bytes = std::fs::read(path).unwrap();
    let dem = owd_format::geotiff::read_bytes(&bytes).unwrap();
    let mut s = Session::new("t", "2026-11-26");
    s.set_dem("river-dem.tif", &bytes, &dem);
    let sha = s.document()["inputs"]["dem"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(sha, owd_format::design::hash::sha256_hex(&bytes));
    let file = DemFile::new(Arc::new(
        FileSource::open(std::path::Path::new(path)).unwrap(),
    ));
    assert_eq!(s.dem_check(Some(&file)), DemCheck::Matches);
    // A small tiled file is still recorded by its full hash: nothing changes for it.
    let small = write_to_vec(&spec(0), cell).unwrap();
    s.set_dem("small.tif", &small, &dem);
    assert!(s.document()["inputs"]["dem"]["sha256"].is_string());
}
