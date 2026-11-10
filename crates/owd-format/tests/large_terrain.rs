//! A large terrain file opens (ticket C11). The `tiff` crate's default decoding limit (256 MiB)
//! no longer decides what a study can load: the reader sets its own, stated limit. And a file that
//! opened before gives the same grid, bit for bit.
//!
//! Label: property (the large file); snapshot (the grid hashes of the existing fixtures, printed by
//! the reader before this ticket).

use owd_format::design::hash::sha256_hex;
use owd_format::geotiff::{Dem, read_bytes};
use std::io::Cursor;
use std::path::Path;
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

const W: u32 = 9_000;
const H: u32 = 8_000;
const ROWS_PER_STRIP: u32 = 64;

/// The value of the cell at (row, col): 10 m everywhere but three cells.
fn written(row: u32, col: u32) -> f32 {
    match (row, col) {
        (0, 0) => 1.0,
        (r, c) if r == H / 2 && c == W / 2 => 3.0,
        (r, c) if r == H - 1 && c == W - 1 => 2.0,
        _ => 10.0,
    }
}

/// A 9,000 by 8,000 GeoTIFF of 32-bit floats (288 MB decoded, over the crate's 256 MiB default),
/// DEFLATE-compressed, so the file on disk stays small.
fn large_geotiff() -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out)
            .unwrap()
            .with_compression(Compression::Deflate(DeflateLevel::Fast));
        let mut img = enc.new_image::<colortype::Gray32Float>(W, H).unwrap();
        img.rows_per_strip(ROWS_PER_STRIP).unwrap();
        let keys: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32630];
        let e = img.encoder();
        e.write_tag(Tag::Unknown(33550), &[30.0, 30.0, 0.0][..])
            .unwrap();
        e.write_tag(
            Tag::Unknown(33922),
            &[0.0, 0.0, 0.0, 500_000.0, 700_000.0, 0.0][..],
        )
        .unwrap();
        e.write_tag(Tag::Unknown(34735), &keys[..]).unwrap();
        // `write_data`, not `write_strip`: in tiff 0.11.3 only `write_data` turns the compression
        // on. The buffer is freed before the file is read.
        let data: Vec<f32> = (0..H)
            .flat_map(|r| (0..W).map(move |c| written(r, c)))
            .collect();
        img.write_data(&data).unwrap();
    }
    out.into_inner()
}

#[test]
fn a_terrain_file_larger_than_the_crates_default_limit_opens() {
    let bytes = large_geotiff();
    assert!(
        bytes.len() < 10_000_000,
        "the file stays small: {} bytes",
        bytes.len()
    );
    let dem =
        read_bytes(&bytes).unwrap_or_else(|e| panic!("the large terrain file is refused: {e}"));
    assert_eq!((dem.width, dem.height), (W as usize, H as usize));
    for (r, c) in [
        (0, 0),
        (H / 2, W / 2),
        (H - 1, W - 1),
        (1, 1),
        (H - 1, 0),
        (0, W - 1),
    ] {
        assert_eq!(
            dem.data[r as usize * W as usize + c as usize],
            f64::from(written(r, c)),
            "cell ({r}, {c})"
        );
    }
}

/// The grid's bits: width, height and every value as little-endian bytes.
fn grid_hash(dem: &Dem) -> String {
    let mut b = Vec::with_capacity(16 + dem.data.len() * 8);
    b.extend_from_slice(&(dem.width as u64).to_le_bytes());
    b.extend_from_slice(&(dem.height as u64).to_le_bytes());
    for v in &dem.data {
        b.extend_from_slice(&v.to_le_bytes());
    }
    sha256_hex(&b)
}

/// The grid hashes the reader gave before ticket C11, on `main` at 0edfc38 (printed by this test).
const BEFORE: [(&str, &str); 14] = [
    (
        "glo30_sassandra_lower.tif",
        "982813eed1a20c14c81eff558d235f7af64b022447dd10253c19db3ef011576c",
    ),
    (
        "river-dem.tif",
        "cf465030bd9b94d48e6757b0364dd9617c09eaea90206c3142aecde8dc1b98ad",
    ),
    (
        "f32_geographic.tif",
        "507e94905d6883d2ec196be5ece55cd49d156a3fccb6518680121f4a40fbc8fd",
    ),
    (
        "f32_mercator_nodata.tif",
        "909960ba9fc16e844aeb59e2d57c98873ef7352dc7b6829ccf3b820ecdac1a8d",
    ),
    (
        "f32_nan_nodata.tif",
        "01f67f4f3dedbcfab970be82786279109e288b616e8eb130277ad204b7e7e510",
    ),
    (
        "f32_pixel_is_point.tif",
        "c1a6ae0f62fd2cbbcc2c55a955bd293537b171d24dd64dbd5137d11157bde69c",
    ),
    (
        "f32_strip_deflate.tif",
        "060ea0d4f17e5a9ed98aafbcfa9ee288c0c0db4e975e4ff94be30a9dd3ef7cea",
    ),
    (
        "f32_strip_none.tif",
        "83ad83534e72f7fa470026312a09624f1a2739a4ca0bec6434249229fb4154da",
    ),
    (
        "f32_tiled_deflate_pred3.tif",
        "30258b276ecd6d650d8449e355a41db2ab9b867fdc883ed62561742417fced2d",
    ),
    (
        "f64_strip_none.tif",
        "dbcba2a985b5c6ab8c1ce7bf2d0dceb46195f3279e73445f67c742664f943444",
    ),
    (
        "glo30_n05_w003_clip.tif",
        "027b244c9601686e7fed059cb7d76b7c96710255a89586f288b12ea58cf342e1",
    ),
    (
        "i16_strip_lzw_pred2_nodata.tif",
        "fca7d736a200764e8975cd9b1acb402235438531342aa382d37198908fe95448",
    ),
    (
        "i16_tiled_lzw.tif",
        "5e9403792b55c727369758ddc7cfcc53c30a91ca7f3babea0cac74806636ac78",
    ),
    (
        "u16_strip_deflate_pred2.tif",
        "ec273eefed7e66f1fdb4192a2b400faf4d1dbcbde8a3540e3b9fd6b360e1a610",
    ),
];

fn fixtures() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests/fixtures/dem"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tif"))
        .filter(|p| {
            !p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("refuse_")
        })
        .collect();
    files.push(root.join("../../examples/web/river-dem.tif"));
    files.push(root.join("../../examples/reach/glo30_sassandra_lower.tif"));
    files.sort();
    files
}

#[test]
fn a_file_that_opened_before_gives_the_same_grid_bit_for_bit() {
    let mut now = Vec::new();
    for f in fixtures() {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let dem = read_bytes(&std::fs::read(&f).unwrap()).unwrap();
        now.push((name, grid_hash(&dem)));
    }
    for (name, hash) in &now {
        println!("(\"{name}\", \"{hash}\"),");
    }
    assert!(
        !BEFORE.is_empty(),
        "no hashes recorded from before the change"
    );
    let before: Vec<(String, String)> = BEFORE
        .iter()
        .map(|(n, h)| (n.to_string(), h.to_string()))
        .collect();
    assert_eq!(now, before);
}
