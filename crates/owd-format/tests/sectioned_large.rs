//! A whole-river DEM read in sections (COG-1): a 20,000 x 60,000 cell DEM (1.2 billion cells,
//! 4.8 GB as `f32`, 9.6 GB as the `f64` grid the whole-raster reader would need) made by formula,
//! written in bounded memory, opened, and a 100 km line sampled and its volumes run with the decoded
//! blocks held to the default cache.
//!
//! Slow (it writes the file once, a few minutes in a debug build): in the nightly list
//! (`.config/nextest.toml`, `docs/agents/slow-tests.md`). The file is kept in the target's
//! temporary folder and made again only when it is not there.
//!
//! `measure_the_owners_raster` is a measurement, not a check: it is ignored, and run by hand in a
//! release build (`cargo test --release -p owd-format --test sectioned_large -- --ignored
//! --nocapture`) to print the numbers the ticket asks for.
//!
//! Label: property

use owd_core::volumes::sections::{HalfWidth, Level, section_volumes};
use owd_format::design::terrain::dem_sampler;
use owd_format::sectioned::layout::SampleType;
use owd_format::sectioned::source::FileSource;
use owd_format::sectioned::terrain::DEFAULT_CACHE_BYTES;
use owd_format::sectioned::write::{Compression, Spec, Spool, write};
use owd_format::sectioned::{SectionedDem, Terrain};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const X0: f64 = 400_000.0;
const Y1: f64 = 900_000.0;

/// The ground: a plain falling south to north at 0.5 m/km, a valley 300 m wide winding down its
/// middle, and some relief.
fn ground(x: f64, y: f64, w_m: f64) -> f64 {
    let axis = X0 + w_m / 2.0 + 2000.0 * libm::sin((Y1 - y) / 9000.0);
    let d = (x - axis) / 150.0;
    100.0 - 0.0005 * (Y1 - y) + 5.0 * libm::sin(x / 700.0) * libm::cos(y / 1100.0)
        - 8.0 * libm::exp(-d * d)
}

fn axis(y: f64, w_m: f64) -> f64 {
    X0 + w_m / 2.0 + 2000.0 * libm::sin((Y1 - y) / 9000.0)
}

fn spec(w: usize, h: usize, cell: f64) -> Spec {
    Spec {
        width: w,
        height: h,
        tile: 512,
        sample: SampleType::F32,
        compression: Compression::Deflate,
        predictor: 3,
        big: None,
        overviews: None,
        nodata: Some(-9999.0),
        geotransform: [cell, 0.0, X0, 0.0, -cell, Y1],
        epsg: Some(32630),
        gap: 0,
    }
}

/// The file of `spec`, written once into the target's temporary folder. Returns its path and the
/// seconds the write took (0 when it was there already).
fn made(name: &str, s: &Spec) -> (PathBuf, f64) {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let done = path.with_extension("done");
    if path.exists() && done.exists() {
        return (path, 0.0);
    }
    let started = Instant::now();
    let cell = s.geotransform[0];
    let w_m = s.width as f64 * cell;
    let mut out = std::fs::File::create(&path).unwrap();
    let spools: std::cell::RefCell<Vec<PathBuf>> = Default::default();
    write(
        s,
        |r0, r1| {
            let mut v = Vec::with_capacity((r1 - r0) * s.width);
            for r in r0..r1 {
                let y = Y1 - (r as f64 + 0.5) * cell;
                for c in 0..s.width {
                    v.push(ground(X0 + (c as f64 + 0.5) * cell, y, w_m));
                }
            }
            Ok(v)
        },
        |k| {
            let p = path.with_extension(format!("spool{k}"));
            spools.borrow_mut().push(p.clone());
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(p)
                .map(|f| Box::new(f) as Box<dyn Spool>)
        },
        &mut out,
        |_, _| {},
    )
    .unwrap();
    for p in spools.borrow().iter() {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&done, b"").unwrap();
    (path, started.elapsed().as_secs_f64())
}

/// This process's resident memory now, in MiB, from `ps` (a measurement aid; 0 when `ps` is not
/// there).
fn rss_mib() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|t| t.trim().parse::<u64>().ok())
        .map_or(0, |kb| kb / 1024)
}

/// The valley's axis from `y0` down to `len_m` further south, a point every 50 m.
fn line(y0: f64, len_m: f64, w_m: f64) -> Vec<[f64; 2]> {
    let n = (len_m / 50.0) as usize;
    let mut pts = Vec::with_capacity(n + 1);
    let mut y = y0;
    let mut run = 0.0;
    let mut last = [axis(y, w_m), y];
    pts.push(last);
    while run < len_m {
        y -= 20.0;
        let p = [axis(y, w_m), y];
        run += libm::hypot(p[0] - last[0], p[1] - last[1]);
        if libm::hypot(p[0] - pts[pts.len() - 1][0], p[1] - pts[pts.len() - 1][1]) >= 50.0 {
            pts.push(p);
        }
        last = p;
    }
    pts
}

/// Volumes along `pts` on `terrain`, the water 2 m above the valley floor.
fn volumes(pts: &[[f64; 2]], terrain: owd_format::types::TerrainSampler) -> (f64, f64) {
    let ys: Vec<f64> = pts.iter().map(|p| p[1]).collect();
    let y0 = ys[0];
    let len = owd_core::geom::linear::length(pts);
    let dy = ys[0] - ys[ys.len() - 1];
    let wl = move |s: f64| 100.0 - 0.0005 * (Y1 - (y0 - s / len * dy)) - 6.0;
    let v = section_volumes(
        pts,
        terrain,
        &[],
        &Level::AtChainage(&wl),
        3.0,
        &HalfWidth::Fixed(25.0),
        "b",
        3.0,
        25.0,
        200.0,
        1.0,
        None,
        None,
        40,
    )
    .unwrap();
    (v.dredge_m3(), v.excav_m3())
}

#[test]
fn a_dem_of_1_2_billion_cells_is_opened_and_a_100_km_line_runs_within_the_cache() {
    let (w, h, cell) = (20_000usize, 60_000usize, 2.0);
    let s = spec(w, h, cell);
    let (path, _) = made("cog1-20000x60000.tif", &s);
    let started = Instant::now();
    let dem = SectionedDem::open(
        Arc::new(FileSource::open(&path).unwrap()),
        DEFAULT_CACHE_BYTES,
    )
    .unwrap();
    let open_s = started.elapsed().as_secs_f64();
    assert!(
        open_s < 5.0,
        "opening read the directories only: {open_s} s"
    );
    assert_eq!((dem.width(), dem.height()), (w, h));
    // Compressed, the file is a BigTIFF only if it passes 4 GiB; the offsets past 4 GiB are proved
    // in `sectioned.rs` by layout.
    let size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(
        dem.layout().big,
        size > u64::from(u32::MAX) - 1024,
        "{size} bytes"
    );
    let sampler = dem_sampler(&dem, Some(32630)).unwrap();
    let pts = line(Y1 - 5_000.0, 100_000.0, w as f64 * cell);
    let started = Instant::now();
    let (dredge, excav) = volumes(&pts, &sampler);
    let run_s = started.elapsed().as_secs_f64();
    // No river polygon is given, so every cut counts as dry cut.
    assert!(
        dredge == 0.0 && excav > 0.0,
        "dredge {dredge}, dry cut {excav}"
    );
    let st = dem.cache_stats();
    assert!(
        st.peak_bytes <= DEFAULT_CACHE_BYTES,
        "decoded blocks peaked at {} bytes, over the {} budget",
        st.peak_bytes,
        DEFAULT_CACHE_BYTES
    );
    assert!(run_s < 900.0, "the volumes of 100 km took {run_s} s");
    eprintln!(
        "open {open_s:.3} s; 100 km volumes {run_s:.1} s; peak decoded {} MiB; blocks decoded {}",
        st.peak_bytes >> 20,
        st.misses
    );
}

/// The owner's raster: 2 m, about 12,000 x 77,000 cells. Prints the time to write it (as
/// `owd dem prepare` writes), its size, the time to open it, the volumes of a 200 km line on it
/// and on a 6 m reduction held in memory, and how far the two differ.
#[test]
#[ignore = "a measurement, run by hand in a release build"]
fn measure_the_owners_raster() {
    let (w, h, cell) = (11_850usize, 77_275usize, 2.0);
    let s = spec(w, h, cell);
    let (path, write_s) = made("cog1-11850x77275.tif", &s);
    let size = std::fs::metadata(&path).unwrap().len();
    let started = Instant::now();
    let dem = SectionedDem::open(
        Arc::new(FileSource::open(&path).unwrap()),
        DEFAULT_CACHE_BYTES,
    )
    .unwrap();
    let open_s = started.elapsed().as_secs_f64();
    let rss_open = rss_mib();
    let w_m = w as f64 * cell;
    // A 200 km line: the valley is 154.5 km long, so the line runs down it and back up beside it.
    let mut pts = line(Y1 - 1_000.0, 150_000.0, w_m);
    let back: Vec<[f64; 2]> = line(Y1 - 1_000.0, 50_000.0, w_m)
        .into_iter()
        .rev()
        .map(|p| [p[0] + 1500.0, p[1] - 101_000.0])
        .collect();
    pts.extend(back);
    let len_km = owd_core::geom::linear::length(&pts) / 1000.0;
    let sampler = dem_sampler(&dem, Some(32630)).unwrap();
    let started = Instant::now();
    let full = volumes(&pts, &sampler);
    let full_s = started.elapsed().as_secs_f64();
    let st = dem.cache_stats();
    let rss_full = rss_mib();
    // A 6 m DEM of the same ground, held in memory (about 0.8 GB as `f64`).
    let coarse = {
        let (cw, ch) = (w / 3, h / 3);
        let mut data = vec![0.0; cw * ch];
        for r in 0..ch {
            for c in 0..cw {
                let y = Y1 - (r as f64 + 0.5) * 6.0;
                data[r * cw + c] = ground(X0 + (c as f64 + 0.5) * 6.0, y, w_m) as f32 as f64;
            }
        }
        owd_format::geotiff::Dem {
            width: cw,
            height: ch,
            data,
            geotransform: [6.0, 0.0, X0, 0.0, -6.0, Y1],
            nodata: Some(-9999.0),
            epsg: Some(32630),
        }
    };
    let cs = dem_sampler(&coarse, Some(32630)).unwrap();
    let started = Instant::now();
    let six = volumes(&pts, &cs);
    let six_s = started.elapsed().as_secs_f64();
    let rss_six = rss_mib();
    let pct = |a: f64, b: f64| 100.0 * (b - a) / a.abs().max(1.0);
    eprintln!(
        "raster {w} x {h} 2 m: written in {write_s:.1} s, {size} bytes ({:.2} GiB); opened in {open_s:.3} s\n\
         line {len_km:.1} km: 2 m sectioned {full_s:.1} s (dredge {:.0} m3, dry cut {:.0} m3; peak decoded {} MiB, {} blocks decoded)\n\
         6 m in memory {six_s:.1} s (dredge {:.0} m3, dry cut {:.0} m3); difference {:+.3} % dredge, {:+.3} % dry cut",
        size as f64 / f64::from(1u32 << 30),
        full.0,
        full.1,
        st.peak_bytes >> 20,
        st.misses,
        six.0,
        six.1,
        pct(full.0, six.0),
        pct(full.1, six.1),
    );
    eprintln!(
        "resident memory: {rss_open} MiB after opening, {rss_full} MiB after the 2 m volumes, {rss_six} MiB with the 6 m DEM in memory and its volumes"
    );
}
