//! `owd dem prepare` and `owd dem info` (COG-1, part 4): a DEM in strips becomes a sectioned copy
//! that reads back cell for cell; a sectioned file is left alone unless forced.
//!
//! Label: property

use owd_cli::dem::{first_difference, info, prepare};
use owd_format::geotiff::Dem;
use owd_format::sectioned::layout::Layout;
use owd_format::sectioned::source::FileSource;
use owd_format::sectioned::{SectionedDem, Terrain};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn dir(tag: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("owd-dem-prepare-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A DEM in strips (as the `tiff` crate writes one), 700 x 500 cells with a nodata hole.
fn stripped(path: &Path) {
    let (w, h) = (700usize, 500usize);
    let data = (0..w * h)
        .map(|i| {
            let (r, c) = (i / w, i % w);
            if (200..220).contains(&r) && (300..340).contains(&c) {
                -9999.0
            } else {
                20.0 + 0.013 * c as f64 + 0.007 * r as f64 + ((c % 34) as f64 - 17.0).abs() / 17.0
            }
        })
        .collect();
    let dem = Dem {
        width: w,
        height: h,
        data,
        geotransform: [2.0, 0.0, 500_000.0, 0.0, -2.0, 600_000.0],
        nodata: Some(-9999.0),
        epsg: Some(32630),
    };
    let bytes = owd_format::design::dem_source::write_geotiff(&dem).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_dem_in_strips_becomes_a_sectioned_copy_that_reads_back_cell_for_cell() {
    let d = dir("roundtrip");
    let (input, output) = (d.join("in.tif"), d.join("out.tif"));
    stripped(&input);
    let (i, o) = (input.to_str().unwrap(), output.to_str().unwrap());
    let before = Layout::read(&FileSource::open(&input).unwrap()).unwrap();
    assert!(!before.images[0].tiled, "the input is in strips");
    prepare(i, o, false, false, true).unwrap();
    let after = Layout::read(&FileSource::open(&output).unwrap()).unwrap();
    let full = &after.images[0];
    assert!(full.tiled && full.block_w == 512 && full.compression == 8 && full.predictor == 3);
    // 700 x 500, then 350 x 250, then 175 x 125: down to about 256 cells.
    let sizes: Vec<_> = after.images.iter().map(|i| (i.width, i.height)).collect();
    assert_eq!(sizes, [(700, 500), (350, 250), (175, 125)]);
    // Directories first, then the coarsest overview's tiles, the full image's last.
    let first_tile = |k: usize| after.images[k].offsets.iter().copied().min().unwrap();
    let last_ifd = after.images.iter().map(|i| i.ifd_offset).max().unwrap();
    assert!(
        last_ifd < first_tile(2) && first_tile(2) < first_tile(1) && first_tile(1) < first_tile(0)
    );
    assert_eq!(first_difference(i, o).unwrap(), None);
    // The overview respects nodata: a cell over the hole's edge is the mean of the cells that hold ground.
    let sec = SectionedDem::open(Arc::new(FileSource::open(&output).unwrap()), 1 << 24).unwrap();
    assert!(sec.level_value(1, 105, 160).is_nan(), "inside the hole");
    assert!(sec.level_value(1, 99, 160).is_finite());
    info(o, true).unwrap();
}

#[test]
fn a_sectioned_file_is_left_alone_unless_forced() {
    let d = dir("again");
    let (input, out, again) = (d.join("in.tif"), d.join("out.tif"), d.join("again.tif"));
    stripped(&input);
    prepare(
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        false,
        false,
        true,
    )
    .unwrap();
    prepare(
        out.to_str().unwrap(),
        again.to_str().unwrap(),
        false,
        false,
        true,
    )
    .unwrap();
    assert!(
        !again.exists(),
        "nothing is written for a file that is already sectioned"
    );
    prepare(
        out.to_str().unwrap(),
        again.to_str().unwrap(),
        false,
        true,
        true,
    )
    .unwrap();
    assert!(again.exists());
    assert_eq!(
        first_difference(out.to_str().unwrap(), again.to_str().unwrap()).unwrap(),
        None
    );
}
