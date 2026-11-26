//! The sectioned GeoTIFF reader (COG-1): block by block, the same values as the whole-raster
//! reader; BigTIFF offsets past 4 GiB; refusals that name the layout; a cache within its budget;
//! windows from the expected overview. Synthetic files only, made by the writer.
//!
//! Label: property

use owd_format::geotiff::{Dem, GeoTiffError, read_bytes};
use owd_format::sectioned::layout::{Layout, SampleType, decode_block};
use owd_format::sectioned::write::{Compression, SparseFile, Spec, write, write_to_vec};
use owd_format::sectioned::{SectionedDem, SharedSource, Terrain};
use std::sync::Arc;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/dem/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A smooth ground with a few sharp places, so a predictor and the bilinear read both have work.
fn ground(r: usize, c: usize) -> f64 {
    let (x, y) = (c as f64, r as f64);
    100.0 + 0.37 * x - 0.11 * y
        + 5.0 * libm::sin(x / 13.0) * libm::cos(y / 7.0)
        + ((r * 31 + c * 17) % 5) as f64 * 0.25
}

fn spec(w: usize, h: usize, sample: SampleType, comp: Compression, pred: u16) -> Spec {
    Spec {
        width: w,
        height: h,
        tile: 32,
        sample,
        compression: comp,
        predictor: pred,
        big: None,
        overviews: Some(2),
        nodata: Some(-9999.0),
        geotransform: [2.0, 0.0, 500_000.0, 0.0, -2.0, 600_000.0],
        epsg: Some(32630),
        gap: 0,
    }
}

fn open(bytes: Vec<u8>) -> SectionedDem {
    SectionedDem::open(Arc::new(bytes) as SharedSource, 1 << 30).unwrap()
}

/// Every cell of the full level equal, and every point read the same, between the two readers.
fn same_cells(whole: &Dem, sec: &SectionedDem) {
    assert_eq!((whole.width, whole.height), (sec.width(), sec.height()));
    assert_eq!(whole.geotransform, sec.geotransform());
    assert_eq!(whole.nodata, sec.nodata());
    assert_eq!(whole.epsg, sec.epsg());
    for r in 0..whole.height {
        for c in 0..whole.width {
            let (a, b) = (whole.value(r, c), sec.value(r, c));
            assert!(
                a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
                "cell ({r}, {c}): whole {a}, sectioned {b}"
            );
        }
    }
    let (x0, y0, x1, y1) = whole.bounds();
    let (xs, ys): (Vec<f64>, Vec<f64>) = (0..997)
        .map(|i| {
            let t = i as f64 / 997.0;
            (x0 + (x1 - x0) * t, y1 - (y1 - y0) * (t * 7.3).fract())
        })
        .unzip();
    let a = whole.sample_many(&xs, &ys);
    let b = Terrain::sample_many(sec, &xs, &ys);
    for (p, q) in a.iter().zip(&b) {
        assert!(p.to_bits() == q.to_bits() || (p.is_nan() && q.is_nan()));
    }
}

#[test]
fn every_written_layout_reads_the_same_block_by_block_as_whole() {
    use Compression::*;
    use SampleType::*;
    let cases = [
        (F32, Deflate, 3),
        (F32, Deflate, 2),
        (F32, Lzw, 3),
        (F32, None, 1),
        (F64, Deflate, 3),
        (F64, Lzw, 1),
        (I16, Deflate, 2),
        (I16, Lzw, 2),
        (U16, Deflate, 2),
        (I32, Deflate, 2),
        (U32, Lzw, 1),
        (U8, Deflate, 2),
        (I8, None, 1),
    ];
    for (sample, comp, pred) in cases {
        let s = spec(77, 45, sample, comp, pred);
        let cell = |r: usize, c: usize| {
            if (r + c).is_multiple_of(23) {
                f64::NAN
            } else if matches!(sample, U8 | I8) {
                ((r * 7 + c * 3) % 100) as f64
            } else {
                ground(r, c)
            }
        };
        let mut s = s;
        if matches!(sample, U8 | I8) {
            s.nodata = Some(0.0);
        }
        let bytes = write_to_vec(&s, cell).unwrap();
        let sec = open(bytes.clone());
        assert_eq!(sec.levels().len(), 3, "{sample:?}");
        // The `tiff` crate's whole read is the reference where it reads the layout; the horizontal
        // predictor on floats it does not read, so there the reference is the formula itself.
        match read_bytes(&bytes) {
            Ok(whole) => same_cells(&whole, &sec),
            Err(e) => {
                assert!(
                    matches!(sample, F32 | F64) && pred == 2,
                    "{sample:?} {comp:?} {pred}: the whole reader refused: {e}"
                );
                for r in 0..45 {
                    for c in 0..77 {
                        let want = cell(r, c);
                        let got = sec.value(r, c);
                        assert!(
                            (want.is_nan() && got.is_nan()) || f64::from(want as f32) == got,
                            "({r}, {c}): {want} vs {got}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_tiled_fixtures_read_the_same_block_by_block_as_whole() {
    for name in ["f32_tiled_deflate_pred3.tif", "i16_tiled_lzw.tif"] {
        let bytes = fixture(name);
        let whole = read_bytes(&bytes).unwrap();
        same_cells(&whole, &open(bytes));
    }
}

#[test]
fn the_stripped_fixtures_decode_block_by_block_to_the_whole_read() {
    // `owd dem prepare` reads a file in strips this way, a band of rows at a time.
    for name in [
        "f32_strip_none.tif",
        "f32_strip_deflate.tif",
        "i16_strip_lzw_pred2_nodata.tif",
        "f64_strip_none.tif",
        "u16_strip_deflate_pred2.tif",
        "f32_mercator_nodata.tif",
        "f32_geographic.tif",
        "glo30_n05_w003_clip.tif",
    ] {
        let bytes = fixture(name);
        let whole = read_bytes(&bytes).unwrap();
        let layout = Layout::read(&bytes).unwrap();
        let geo = layout.geo.clone().unwrap();
        assert_eq!(geo.geotransform, whole.geotransform, "{name}");
        assert_eq!(
            (geo.epsg, geo.nodata.map(f64::to_bits)),
            (whole.epsg, whole.nodata.map(f64::to_bits)),
            "{name}"
        );
        let img = &layout.images[0];
        for i in 0..img.blocks() {
            let s = decode_block(&bytes, layout.little, img, i, f64::NAN).unwrap();
            let (bx, by) = (
                (i % img.across()) * img.block_w,
                (i / img.across()) * img.block_h,
            );
            let rows = if img.tiled {
                img.block_h
            } else {
                img.rows_in(i)
            };
            for r in 0..rows {
                for c in 0..img.block_w {
                    let (rr, cc) = (by + r, bx + c);
                    if rr < img.height && cc < img.width {
                        let a = whole.data[rr * whole.width + cc];
                        let b = s.get(r * img.block_w + c);
                        assert!(
                            a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
                            "{name} ({rr}, {cc})"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn predictor_two_on_float32_is_integer_differencing_of_the_words() {
    // The owner's terrain uses PREDICTOR=2 on float32, as libtiff writes it.
    let s = spec(64, 40, SampleType::F32, Compression::Deflate, 2);
    let sec = open(write_to_vec(&s, ground).unwrap());
    for r in 0..40 {
        for c in 0..64 {
            assert_eq!(sec.value(r, c), f64::from(ground(r, c) as f32));
        }
    }
}

#[test]
fn a_uint8_sources_raster_reads_through_the_same_reader() {
    let mut s = spec(100, 70, SampleType::U8, Compression::Deflate, 1);
    s.nodata = Some(0.0);
    let sec = open(write_to_vec(&s, |r, c| ((r / 10 + c / 10) % 4) as f64).unwrap());
    assert_eq!(sec.image(0).sample, Some(SampleType::U8));
    assert!(sec.value(0, 0).is_nan(), "0 is nodata");
    assert_eq!(sec.value(15, 5), 1.0);
    assert_eq!(sec.levels().len(), 3);
}

#[test]
fn bigtiff_offsets_past_4_gib_are_read() {
    let mut s = spec(90, 70, SampleType::F32, Compression::Deflate, 3);
    s.gap = 5 << 30; // the tiles start past 5 GiB; nothing is written in the gap
    let mut out = SparseFile::default();
    write(
        &s,
        |r0, r1| {
            Ok((r0..r1)
                .flat_map(|r| (0..90).map(move |c| ground(r, c)))
                .collect())
        },
        |_| {
            Ok(Box::new(std::io::Cursor::new(Vec::new()))
                as Box<dyn owd_format::sectioned::write::Spool>)
        },
        &mut out,
        |_, _| {},
    )
    .unwrap();
    let src: SharedSource = Arc::new(out);
    assert!(src.len() > 5 << 30);
    let sec = SectionedDem::open(src, 1 << 26).unwrap();
    assert!(sec.layout().big, "a file past 4 GiB is a BigTIFF");
    assert!(sec.image(0).offsets.iter().all(|&o| o > 4 << 30));
    for r in 0..70 {
        for c in 0..90 {
            assert_eq!(sec.value(r, c), f64::from(ground(r, c) as f32));
        }
    }
}

#[test]
fn a_bigtiff_by_choice_reads_as_a_classic_one() {
    let mut s = spec(50, 50, SampleType::F32, Compression::Lzw, 3);
    let classic = open(write_to_vec(&s, ground).unwrap());
    s.big = Some(true);
    let big = open(write_to_vec(&s, ground).unwrap());
    assert!(big.layout().big && !classic.layout().big);
    for r in 0..50 {
        for c in 0..50 {
            assert_eq!(big.value(r, c).to_bits(), classic.value(r, c).to_bits());
        }
    }
}

fn refusal(bytes: Vec<u8>) -> String {
    match SectionedDem::open(Arc::new(bytes), 1 << 20) {
        Err(GeoTiffError::Unsupported(m)) => m,
        other => panic!("not refused as unsupported: {other:?}"),
    }
}

#[test]
fn refusals_name_what_the_file_has_and_what_to_do() {
    let m = refusal(fixture("f32_strip_deflate.tif"));
    assert!(
        m.contains("strips, not tiles") && m.contains("owd dem prepare"),
        "{m}"
    );
    let m = refusal(fixture("refuse_two_bands.tif"));
    assert!(m.contains("2 bands") || m.contains("strips"), "{m}");
    // ZSTD and PackBits named by compression, made tiled so the layout is not what is refused.
    for (code, name) in [(50000u16, "ZSTD"), (32773, "PackBits"), (7, "JPEG")] {
        let mut b =
            write_to_vec(&spec(40, 40, SampleType::F32, Compression::None, 1), ground).unwrap();
        patch_compression(&mut b, code);
        let m = refusal(b);
        assert!(m.contains(name), "{m}");
    }
}

/// Set the Compression of the first directory of a classic little-endian TIFF.
fn patch_compression(b: &mut [u8], code: u16) {
    let ifd = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes(b[ifd..ifd + 2].try_into().unwrap()) as usize;
    for k in 0..n {
        let e = ifd + 2 + 12 * k;
        if u16::from_le_bytes([b[e], b[e + 1]]) == 259 {
            b[e + 8..e + 10].copy_from_slice(&code.to_le_bytes());
        }
    }
}

#[test]
fn overviews_are_means_that_respect_nodata_and_windows_come_from_the_expected_level() {
    let mut s = spec(64, 64, SampleType::F64, Compression::Deflate, 3);
    s.overviews = Some(3);
    let cell = |r: usize, c: usize| {
        if r < 2 && c < 1 {
            f64::NAN
        } else {
            (r * 64 + c) as f64
        }
    };
    let sec = open(write_to_vec(&s, cell).unwrap());
    let lv = sec.levels();
    assert_eq!(
        lv.iter().map(|l| l.width).collect::<Vec<_>>(),
        [64, 32, 16, 8]
    );
    assert_eq!(lv[1].geotransform[0], 4.0);
    // Level 1 cell (0, 0): cells (0,0),(0,1),(1,0),(1,1) with (0,0) and (1,0) nodata.
    assert_eq!(sec.level_value(1, 0, 0), (1.0 + 65.0) / 2.0);
    assert_eq!(
        sec.level_value(1, 5, 7),
        (cell(10, 14) + cell(10, 15) + cell(11, 14) + cell(11, 15)) / 4.0
    );
    // Cell sizes 2, 4, 8, 16 m: a 5 m ask reads the 4 m level, a 100 m ask the coarsest.
    assert_eq!(sec.level_for_cell(1.0), 0);
    assert_eq!(sec.level_for_cell(5.0), 1);
    assert_eq!(sec.level_for_cell(8.0), 2);
    assert_eq!(sec.level_for_cell(100.0), 3);
    let b = sec.bounds();
    let w = sec.window(b, 5.0);
    assert_eq!((w.width, w.height, w.geotransform[0]), (32, 32, 4.0));
    let w = sec.window_within(b, 300);
    assert_eq!((w.width, w.height), (16, 16));
    let part = sec.window_level(0, (500_010.0, 599_900.0, 500_020.0, 599_910.0));
    assert_eq!((part.width, part.height), (5, 5));
    assert_eq!(part.value(0, 0), sec.value(45, 5));
}

#[test]
fn the_cache_stays_within_its_budget_while_a_long_line_is_sampled() {
    // 1,024 x 1,024 cells in 32 x 32 tiles of 4 kB each; a budget of 64 kB holds 16 tiles.
    let mut s = spec(1024, 1024, SampleType::F32, Compression::Deflate, 3);
    s.overviews = Some(0);
    let sec = SectionedDem::open(Arc::new(write_to_vec(&s, ground).unwrap()), 64 << 10).unwrap();
    let (x0, y0, x1, y1) = sec.bounds();
    let n = 20_000;
    let (xs, ys): (Vec<f64>, Vec<f64>) = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                x0 + 1.0 + (x1 - x0 - 2.0) * t,
                y1 - 1.0 - (y1 - y0 - 2.0) * t,
            )
        })
        .unzip();
    let z = Terrain::sample_many(&sec, &xs, &ys);
    assert!(z.iter().all(|v| v.is_finite()));
    let st = sec.cache_stats();
    assert!(st.peak_bytes <= 64 << 10, "peak {} bytes", st.peak_bytes);
    assert!(
        st.evictions > 0 && st.misses >= 32,
        "the line crosses 32 tiles: {st:?}"
    );
}
